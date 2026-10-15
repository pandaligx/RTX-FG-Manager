// WebCrypto-only administrator credentials. This module has no network access.
const encoder=new TextEncoder();
export const base64=bytes=>btoa(String.fromCharCode(...new Uint8Array(bytes)));
const unbase64=text=>Uint8Array.from(atob(text),c=>c.charCodeAt(0));
const hex=bytes=>Array.from(new Uint8Array(bytes),b=>b.toString(16).padStart(2,'0')).join('');
const hash=async bytes=>hex(await crypto.subtle.digest('SHA-256',bytes));
const context='RTXFG-MODERATOR-BACKUP-V1';
async function wrappingKey(password,salt) {
  if(typeof password!=='string' || password.length<12 || password.length>1024)throw Error('密码需要 12–1024 个字符。');
  const key=await crypto.subtle.importKey('raw',encoder.encode(password),'PBKDF2',false,['deriveKey']);
  return crypto.subtle.deriveKey({name:'PBKDF2',hash:'SHA-256',salt,iterations:600000},key,{name:'AES-GCM',length:256},false,['encrypt','decrypt']);
}
export async function createIdentity(password) {
  const pair=await crypto.subtle.generateKey({name:'ECDSA',namedCurve:'P-256'},true,['sign','verify']);
  const raw=new Uint8Array(await crypto.subtle.exportKey('pkcs8',pair.privateKey));
  const publicKey=await crypto.subtle.exportKey('raw',pair.publicKey), identity=await hash(publicKey);
  const salt=crypto.getRandomValues(new Uint8Array(16)),nonce=crypto.getRandomValues(new Uint8Array(12));
  try {
    const key=await wrappingKey(password,salt);
    const ciphertext=await crypto.subtle.encrypt({name:'AES-GCM',iv:nonce,additionalData:encoder.encode(`${context}\n${identity}`)},key,raw);
    const backup={schema:1,kind:context,identity,publicKey:base64(publicKey),salt:base64(salt),nonce:base64(nonce),ciphertext:base64(ciphertext)};
    const privateKey=await crypto.subtle.importKey('pkcs8',raw,{name:'ECDSA',namedCurve:'P-256'},false,['sign']);
    return {backup,privateKey};
  } finally {raw.fill(0);}
}
export async function unlock(backup,password) {
  if(!backup || Object.keys(backup).sort().join(',')!=='ciphertext,identity,kind,nonce,publicKey,salt,schema' || backup.schema!==1 || backup.kind!==context || !/^[a-f0-9]{64}$/.test(backup.identity))throw Error('无效的管理员备份。');
  const salt=unbase64(backup.salt),nonce=unbase64(backup.nonce),ciphertext=unbase64(backup.ciphertext),publicKey=unbase64(backup.publicKey);
  if(salt.length!==16 || nonce.length!==12 || ciphertext.length>512 || publicKey.length!==65 || await hash(publicKey)!==backup.identity)throw Error('无效的管理员备份。');
  const key=await wrappingKey(password,salt);let raw;
  try {
    raw=new Uint8Array(await crypto.subtle.decrypt({name:'AES-GCM',iv:nonce,additionalData:encoder.encode(`${context}\n${backup.identity}`)},key,ciphertext));
    const privateKey=await crypto.subtle.importKey('pkcs8',raw,{name:'ECDSA',namedCurve:'P-256'},false,['sign']);
    const check=encoder.encode(context),signature=await crypto.subtle.sign({name:'ECDSA',hash:'SHA-256'},privateKey,check);
    const verifyKey=await crypto.subtle.importKey('raw',publicKey,{name:'ECDSA',namedCurve:'P-256'},false,['verify']);
    if(!await crypto.subtle.verify({name:'ECDSA',hash:'SHA-256'},verifyKey,signature,check))throw Error();
    return {backup,privateKey};
  } catch {throw Error('密码不正确或备份已损坏。');} finally {raw?.fill(0);}
}
export async function headers(session,method,route,body='') {
  const timestamp=Math.floor(Date.now()/1000).toString(),nonce=hex(crypto.getRandomValues(new Uint8Array(16))),identity=session.backup.identity;
  const canonical=['RTXFG-COMMUNITY-V1',method,route,await hash(encoder.encode(body)),timestamp,nonce,identity].join('\n');
  const signature=await crypto.subtle.sign({name:'ECDSA',hash:'SHA-256'},session.privateKey,encoder.encode(canonical));
  return {'x-rtx-identity':identity,'x-rtx-timestamp':timestamp,'x-rtx-nonce':nonce,'x-rtx-signature':base64(signature)};
}
