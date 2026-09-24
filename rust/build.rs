use flate2::{Compression, write::ZlibEncoder};
use sha2::{Digest, Sha256};
use std::{env, fs, io::Write, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=rust/native");
    if env::var_os("CARGO_FEATURE_NATIVE_PROBES").is_some() {
        assert!(
            PathBuf::from("rust/native/probe_entry.cpp").is_file(),
            "native-probes is a maintainer-only diagnostic extension; its extra native sources and licensed SDK headers are not part of the public Rust manager. See BUILDING.md"
        );
        for name in ["RTXFG_VULKAN_INCLUDE", "RTXFG_NGX_INCLUDE"] {
            println!("cargo:rerun-if-env-changed={name}");
        }
        let vulkan = env::var_os("RTXFG_VULKAN_INCLUDE")
            .expect("native-probes requires RTXFG_VULKAN_INCLUDE; see BUILDING.md");
        let ngx = env::var_os("RTXFG_NGX_INCLUDE")
            .expect("native-probes requires RTXFG_NGX_INCLUDE; see BUILDING.md");
        cc::Build::new()
            .cpp(true)
            .std("c++17")
            .static_crt(true)
            .flag("/EHsc")
            .flag("/O2")
            .include(vulkan)
            .include(ngx)
            .files([
                "rust/native/probe_entry.cpp",
                "rust/native/probe_d3d12.cpp",
                "rust/native/probe_vulkan.cpp",
            ])
            .compile("rtxfg_probe");
        for lib in ["d3d12", "dxgi", "dxguid"] {
            println!("cargo:rustc-link-lib={lib}");
        }
    }
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let mut names = Vec::new();
    for folder in ["native", "native-x6", "rtx20", "rtx30"] {
        // Only explicitly requested regression builds carry historical fixtures.
        if env::var_os("CARGO_FEATURE_FIXTURE_TESTS").is_none() {
            break;
        }
        let proxies: &[&str] = if folder.starts_with("native") {
            &[
                "version.dll",
                "winmm.dll",
                "dinput8.dll",
                "winhttp.dll",
                "dxgi.dll",
            ]
        } else {
            &["version.dll"]
        };
        for name in proxies.iter().copied().chain(["dlssg_sm86.ini"]) {
            names.push(format!("payloads/{folder}/{name}"));
        }
    }
    names.extend(["app/tools/aria2c.exe", "app/tools/aria2.conf"].map(str::to_owned));
    let mut code = String::from("pub static EMBEDDED: &[Embedded] = &[\n");
    for (index, name) in names.iter().enumerate() {
        let path = if name.starts_with("payloads/") {
            root.join("tests/fixtures/runtime/payloads")
                .join(name.strip_prefix("payloads/").unwrap())
        } else {
            root.join(name)
        };
        println!("cargo:rerun-if-changed={}", path.display());
        let bytes = fs::read(&path).unwrap_or_else(|e| {
            panic!(
                "Missing build resource {}: {e}. See BUILDING.md and tools/prepare-fixtures.ps1",
                path.display()
            )
        });
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
        encoder.write_all(&bytes).unwrap();
        let destination = out.join(format!("resource-{index}.z"));
        fs::write(&destination, encoder.finish().unwrap()).unwrap();
        code.push_str(&format!("Embedded {{ name: {name:?}, sha256: \"{:x}\", size: {}, compressed: include_bytes!({:?}) }},\n",Sha256::digest(&bytes),bytes.len(),destination.to_str().unwrap()));
    }
    code.push_str("];\n");
    fs::write(out.join("embedded.rs"), code).unwrap();
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=rust/assets/manager.ico");
        println!("cargo:rustc-link-arg-bin=RTXManager=/NOIMPLIB");
        println!("cargo:rustc-link-arg-bin=RTXManager=/NOEXP");
        // GPUI supplies the asInvoker / PerMonitorV2 manifest; do not embed a duplicate.
        let mut resource = winres::WindowsResource::new();
        resource
            .set_icon("rust/assets/manager.ico")
            .set("InternalName", "RTXManager")
            .set("FileDescription", "RTX Frame Generation Manager by 小南瓜")
            .set("ProductName", "RTX Frame Generation Manager")
            .set("FileVersion", env!("CARGO_PKG_VERSION"))
            .set("ProductVersion", env!("CARGO_PKG_VERSION"));
        resource
            .compile()
            .expect("Windows version/icon resource compilation");
    }
}
