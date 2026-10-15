//! Independent community identity. Private keys never enter request bodies or logs.
use crate::core;
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use p256::{
    ecdsa::{Signature, SigningKey, signature::Signer},
    elliptic_curve::rand_core::{OsRng, RngCore},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};
use windows::{
    Win32::{
        Foundation::{HLOCAL, LocalFree},
        Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
        },
    },
    core::PCWSTR,
};
use zeroize::{Zeroize, Zeroizing};

const BACKUP_CONTEXT: &str = "RTXFG-COMMUNITY-IDENTITY-BACKUP-V1";
pub struct Identity(SigningKey);
#[derive(Clone, Debug, Serialize)]
pub struct PublicIdentity {
    pub identity: String,
    pub public_key: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    schema: u32,
    identity: String,
    protected: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Backup {
    schema: u32,
    identity: String,
    salt: String,
    nonce: String,
    ciphertext: String,
}
pub struct RequestSignature {
    pub identity: String,
    pub timestamp: String,
    pub nonce: String,
    pub signature: String,
}

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
impl Identity {
    fn generate() -> Self {
        Self(SigningKey::random(&mut OsRng))
    }
    pub fn public(&self) -> PublicIdentity {
        let public = self.0.verifying_key().to_encoded_point(false);
        PublicIdentity {
            identity: digest(public.as_bytes()),
            public_key: STANDARD.encode(public.as_bytes()),
        }
    }
    pub fn sign(&self, method: &str, route: &str, body: &[u8]) -> Result<RequestSignature> {
        ensure!(
            matches!(method, "POST" | "PUT" | "DELETE")
                && route.starts_with("/v1/")
                && !route.contains(['\r', '\n', '#']),
            "无效社区请求"
        );
        let public = self.public();
        let timestamp = chrono::Utc::now().timestamp().to_string();
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let canonical = canonical(method, route, body, &timestamp, &nonce, &public.identity);
        let signature: Signature = self.0.sign(canonical.as_bytes());
        Ok(RequestSignature {
            identity: public.identity,
            timestamp,
            nonce,
            signature: STANDARD.encode(signature.to_bytes()),
        })
    }
    pub fn backup(&self, password: &str) -> Result<Vec<u8>> {
        ensure!(
            (12..=1024).contains(&password.chars().count()),
            "备份密码至少需要 12 个字符，最多 1024 个字符"
        );
        let mut salt = [0u8; 16];
        let mut nonce = [0u8; 12];
        OsRng.fill_bytes(&mut salt);
        OsRng.fill_bytes(&mut nonce);
        let key = derive_key(password, &salt)?;
        let cipher =
            Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_| anyhow::anyhow!("备份加密失败"))?;
        let identity = self.public().identity;
        let aad = format!("{BACKUP_CONTEXT}\n{identity}");
        let raw = Zeroizing::new(self.0.to_bytes());
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: raw.as_ref(),
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| anyhow::anyhow!("备份加密失败"))?;
        Ok(serde_json::to_vec_pretty(&Backup {
            schema: 1,
            identity,
            salt: STANDARD.encode(salt),
            nonce: STANDARD.encode(nonce),
            ciphertext: STANDARD.encode(ciphertext),
        })?)
    }
    pub fn from_backup(bytes: &[u8], password: &str) -> Result<Self> {
        ensure!(
            bytes.len() <= 4096 && password.len() <= 4096,
            "备份文件或密码过长"
        );
        let backup: Backup = serde_json::from_slice(bytes).context("无效的身份备份文件")?;
        ensure!(
            backup.schema == 1
                && backup.identity.len() == 64
                && backup.identity.bytes().all(|b| b.is_ascii_hexdigit()),
            "无效的身份备份文件"
        );
        let salt = STANDARD.decode(&backup.salt)?;
        let nonce = STANDARD.decode(&backup.nonce)?;
        let ciphertext = STANDARD.decode(&backup.ciphertext)?;
        ensure!(
            salt.len() == 16 && nonce.len() == 12 && ciphertext.len() == 48,
            "无效的身份备份文件"
        );
        let key = derive_key(password, &salt)?;
        let cipher =
            Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_| anyhow::anyhow!("恢复身份失败"))?;
        let aad = format!("{BACKUP_CONTEXT}\n{}", backup.identity);
        let raw = Zeroizing::new(
            cipher
                .decrypt(
                    Nonce::from_slice(&nonce),
                    Payload {
                        msg: &ciphertext,
                        aad: aad.as_bytes(),
                    },
                )
                .map_err(|_| anyhow::anyhow!("密码不正确或备份文件已损坏"))?,
        );
        let identity =
            Self(SigningKey::from_slice(&raw).map_err(|_| anyhow::anyhow!("无效的身份密钥"))?);
        ensure!(
            identity.public().identity == backup.identity,
            "身份备份校验失败"
        );
        Ok(identity)
    }
}
fn canonical(
    method: &str,
    route: &str,
    body: &[u8],
    timestamp: &str,
    nonce: &str,
    identity: &str,
) -> String {
    format!(
        "RTXFG-COMMUNITY-V1\n{method}\n{route}\n{}\n{timestamp}\n{nonce}\n{identity}",
        digest(body)
    )
}
fn derive_key(password: &str, salt: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
    let params = argon2::Params::new(65536, 3, 1, Some(32))
        .map_err(|_| anyhow::anyhow!("备份加密参数错误"))?;
    let mut key = Zeroizing::new([0u8; 32]);
    argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, key.as_mut())
        .map_err(|_| anyhow::anyhow!("备份密码派生失败"))?;
    Ok(key)
}
fn dpapi(input: &[u8], protect: bool) -> Result<Zeroizing<Vec<u8>>> {
    ensure!(
        !input.is_empty() && input.len() <= 16384,
        "身份密钥大小无效"
    );
    let source = CRYPT_INTEGER_BLOB {
        cbData: input.len() as u32,
        pbData: input.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: input lives for the call, output is initialized by DPAPI; no UI or machine-wide scope.
    unsafe {
        if protect {
            CryptProtectData(
                &source,
                PCWSTR::null(),
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &source,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    }
    .context("Windows 无法解密此身份；请使用原 Windows 账户或主动恢复加密备份")?;
    // SAFETY: successful DPAPI calls return cbData bytes allocated with LocalAlloc.
    let bytes = unsafe { std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize) };
    let result = Zeroizing::new(bytes.to_vec());
    bytes.zeroize();
    // SAFETY: release precisely the DPAPI-owned buffer, after copying and wiping it.
    unsafe {
        let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
    }
    Ok(result)
}
fn root() -> Result<PathBuf> {
    Ok(
        PathBuf::from(std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA unavailable")?)
            .join("RTXFGManager")
            .join("community-identity"),
    )
}
fn lock(root: &Path) -> Result<File> {
    core::no_links(root)?;
    fs::create_dir_all(root)?;
    let path = root.join("identity.lock");
    core::no_links(&path)?;
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(path)
        .context("社区身份正被其他操作使用，请稍后重试")
}
fn read_at(root: &Path) -> Result<Option<Identity>> {
    let path = root.join("identity.json");
    core::no_links(&path)?;
    if !path.try_exists()? {
        return Ok(None);
    }
    let stored: Stored = serde_json::from_value(core::read_json(&path, 32768)?)?;
    ensure!(stored.schema == 1, "社区身份格式不支持");
    let protected = STANDARD.decode(stored.protected)?;
    let bytes = dpapi(&protected, false)?;
    let identity =
        Identity(SigningKey::from_slice(&bytes).map_err(|_| anyhow::anyhow!("身份密钥已损坏"))?);
    ensure!(
        stored.identity == identity.public().identity,
        "社区身份校验失败"
    );
    Ok(Some(identity))
}
fn write_at(root: &Path, identity: &Identity) -> Result<()> {
    let bytes = Zeroizing::new(identity.0.to_bytes());
    let protected = dpapi(bytes.as_ref(), true)?;
    core::atomic_json(
        &root.join("identity.json"),
        &Stored {
            schema: 1,
            identity: identity.public().identity,
            protected: STANDARD.encode(protected.as_slice()),
        },
    )
}
/// Called only after the user chooses a community write action.
pub fn load_or_create() -> Result<Identity> {
    let root = root()?;
    let _guard = lock(&root)?;
    if let Some(identity) = read_at(&root)? {
        return Ok(identity);
    }
    let identity = Identity::generate();
    write_at(&root, &identity)?;
    Ok(identity)
}
pub fn current() -> Result<Option<PublicIdentity>> {
    let root = root()?;
    if !root.try_exists()? {
        return Ok(None);
    }
    let _guard = lock(&root)?;
    Ok(read_at(&root)?.map(|id| id.public()))
}
/// The UI previews the recovered public identity and asks before replacing an existing one.
pub fn restore(identity: &Identity, expected_current: Option<&str>) -> Result<()> {
    let root = root()?;
    let _guard = lock(&root)?;
    let current = read_at(&root)?.map(|id| id.public().identity);
    ensure!(
        current.as_deref() == expected_current,
        "当前身份已改变，请重新预览恢复操作"
    );
    write_at(&root, identity)
}
pub fn save_backup(path: &Path, identity: &Identity, password: &str) -> Result<()> {
    let bytes = identity.backup(password)?;
    core::write_new(path, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::signature::Verifier;
    #[test]
    fn signatures_bind_method_route_body_and_identity() {
        let identity = Identity::generate();
        let signed = identity.sign("POST", "/v1/profiles", b"{}").unwrap();
        let signature =
            Signature::from_slice(&STANDARD.decode(&signed.signature).unwrap()).unwrap();
        let text = canonical(
            "POST",
            "/v1/profiles",
            b"{}",
            &signed.timestamp,
            &signed.nonce,
            &signed.identity,
        );
        assert!(
            identity
                .0
                .verifying_key()
                .verify(text.as_bytes(), &signature)
                .is_ok()
        );
        assert!(
            identity
                .0
                .verifying_key()
                .verify(text.replace("POST", "DELETE").as_bytes(), &signature)
                .is_err()
        );
        assert_eq!(
            STANDARD.decode(identity.public().public_key).unwrap().len(),
            65
        );
    }
    #[test]
    fn backup_restores_and_rejects_wrong_password_and_tampering() {
        let identity = Identity::generate();
        let bytes = identity.backup("a-long-test-password").unwrap();
        assert_eq!(
            identity.public().identity,
            Identity::from_backup(&bytes, "a-long-test-password")
                .unwrap()
                .public()
                .identity
        );
        assert!(Identity::from_backup(&bytes, "incorrect-password").is_err());
        let mut backup: Backup = serde_json::from_slice(&bytes).unwrap();
        backup.identity = "0".repeat(64);
        assert!(
            Identity::from_backup(
                &serde_json::to_vec(&backup).unwrap(),
                "a-long-test-password"
            )
            .is_err()
        );
    }
    #[test]
    fn dpapi_roundtrip_persists_without_plaintext_and_excludes_concurrent_writers() {
        let dir = tempfile::tempdir().unwrap();
        let identity = Identity::generate();
        let _guard = lock(dir.path()).unwrap();
        assert!(lock(dir.path()).is_err());
        write_at(dir.path(), &identity).unwrap();
        let file = fs::read(dir.path().join("identity.json")).unwrap();
        assert!(
            !file
                .windows(32)
                .any(|w| w == identity.0.to_bytes().as_slice())
        );
        assert_eq!(
            identity.public().identity,
            read_at(dir.path()).unwrap().unwrap().public().identity
        );
    }
}
