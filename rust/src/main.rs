#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod controller;
mod game_icons;
mod ui;
mod ui_assets;
fn main() {
    if let Err(e) = run() {
        if std::env::args().any(|s| {
            [
                "--scan",
                "--selftest-child",
                "--selftest-run",
                "--signature",
                "--hags-probe",
                "--payload-report",
                "--gpu-name-action",
                "--download-check",
                "--cloud-check",
            ]
            .contains(&s.as_str())
        }) {
            eprintln!("{e:#}");
        } else {
            rtx_fg_manager::win::message("RTX Manager", &format!("{e:#}"));
        }
        std::process::exit(1);
    }
}
fn run() -> anyhow::Result<()> {
    use anyhow::Context;
    use rtx_fg_manager::{core, hags, preferences, scanner, updater, win};
    use std::{path::PathBuf, sync::atomic::AtomicBool};
    let args: Vec<_> = std::env::args().collect();
    let arg = |n: usize| args.get(n).context("缺少命令参数");
    match args.get(1).map(String::as_str) {
        Some("--cloud-check") => {
            let mode = args.get(3).map(String::as_str).unwrap_or("normal");
            let mut catalog = if mode.starts_with("bundled") {
                rtx_fg_manager::cloud::bundled()
            } else if mode == "offline" {
                rtx_fg_manager::cloud::cached()
            } else {
                rtx_fg_manager::cloud::refresh()?
            };
            if mode == "fallback" || mode == "offline" || mode == "bundled-fallback" {
                catalog.sources.get_mut("domestic").unwrap().base_url =
                    "https://127.0.0.1:1/".into();
            }
            if mode == "offline" || mode == "domestic-only" {
                catalog.sources.get_mut("github").unwrap().base_url = "https://127.0.0.1:1/".into();
            }
            catalog.prefer_github = mode == "bundled-github";
            let mut results = Vec::new();
            for p in &catalog.packages {
                let series = if p
                    .backends
                    .iter()
                    .any(|b| b.ends_with("30") || b == "upstream_sm86")
                {
                    1
                } else {
                    0
                };
                let result = rtx_fg_manager::cloud::prepare(
                    &catalog,
                    &p.scheme_id,
                    series,
                    std::slice::from_ref(&p.proxy),
                    &AtomicBool::new(false),
                    |_| {},
                )?;
                results.push(serde_json::json!({"package":p.id,"backend":result.backend,"version":result.version,"dll_sha256":core::hash(&result.files[&p.proxy])}));
            }
            core::atomic_json(
                &PathBuf::from(arg(2)?),
                &serde_json::json!({"packages":results,"game_tested":false}),
            )
        }
        Some("--download-check") => {
            // Development-only transfer verification. Never installs/launches the download.
            use std::io::Write;
            let m: updater::Manifest =
                serde_json::from_value(core::read_json(&PathBuf::from(arg(2)?), 16384)?)?;
            m.validate(&m.version)?;
            updater::safe_url(&m.url)?;
            let data = PathBuf::from(arg(3)?);
            std::fs::create_dir_all(&data)?;
            let mut log = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(data.join("transfer.jsonl"))?;
            let cancel = std::sync::Arc::new(AtomicBool::new(false));
            if let Some(ms) = args.get(4) {
                let ms = ms.parse::<u64>()?.min(600_000);
                let flag = cancel.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(ms));
                    flag.store(true, std::sync::atomic::Ordering::Relaxed);
                });
            }
            let started = std::time::Instant::now();
            let mut write_error = None;
            let result = updater::download(&m, &data, &cancel, |p| {
                if let Err(e) = writeln!(
                    log,
                    "{}",
                    serde_json::json!({"seconds":started.elapsed().as_secs_f64(),"progress":p})
                ) {
                    write_error = Some(e);
                }
            });
            if let Some(e) = write_error {
                return Err(e.into());
            }
            core::atomic_json(
                &data.join("result.json"),
                &serde_json::json!({
                    "success":result.is_ok(),"seconds":started.elapsed().as_secs_f64(),
                    "file":result.as_ref().ok(),"error":result.as_ref().err().map(|e|format!("{e:#}"))
                }),
            )?;
            result.map(|_| ())
        }
        #[cfg(feature = "native-probes")]
        Some("--selftest-child") => rtx_fg_manager::selftest::child(&PathBuf::from(arg(2)?)),
        #[cfg(feature = "native-probes")]
        Some("--selftest-run") => {
            let adapters = rtx_fg_manager::selftest::adapters()?;
            let adapter = adapters.into_iter().next().context("No NVIDIA adapter")?;
            let request = rtx_fg_manager::selftest::Request {
                adapter,
                complete: args.iter().any(|s| s == "--complete"),
                forward_sm75: args.iter().any(|s| s == "--forward-sm75"),
                game: None,
            };
            let report = rtx_fg_manager::selftest::run(
                &PathBuf::from(arg(2)?),
                request,
                &AtomicBool::new(false),
                |s| eprintln!("{s}"),
            )?;
            eprintln!("{}", report.folder.display());
            Ok(())
        }
        #[cfg(not(feature = "native-probes"))]
        Some("--selftest-child" | "--selftest-run") => {
            anyhow::bail!("Native GPU diagnostics are not included in this build; see BUILDING.md")
        }
        Some("--gpu-name-action") => rtx_fg_manager::gpu_alias::elevated_action(arg(2)?, arg(3)?),
        Some("--hags-probe") => core::atomic_json(&PathBuf::from(arg(2)?), &hags::probe()?),
        Some("--scan") => {
            let out = PathBuf::from(arg(2)?);
            let roots = args[3..].iter().map(PathBuf::from).collect::<Vec<_>>();
            core::atomic_json(
                &out,
                &scanner::scan(&roots, &AtomicBool::new(false), |_, _, _| {})?,
            )
        }
        Some("--signature") => core::atomic_json(
            &PathBuf::from(arg(3)?),
            &win::verify_signature(&PathBuf::from(arg(2)?))?,
        ),
        Some("--apply-update") => updater::apply(&PathBuf::from(arg(2)?)),
        Some("--payload-report") => {
            let rows=rtx_fg_manager::assets::EMBEDDED.iter().map(|r|{let bytes=rtx_fg_manager::assets::bytes(r.name)?;Ok(serde_json::json!({"name":r.name,"bytes":bytes.len(),"sha256":core::hash(&bytes)}))}).collect::<anyhow::Result<Vec<_>>>()?;
            core::atomic_json(&PathBuf::from(arg(2)?), &rows)
        }
        _ => {
            let mut data = preferences::directory();
            let mut smoke = None;
            for (i, item) in args.iter().enumerate().skip(1) {
                match item.as_str() {
                    "--data-dir" => data = PathBuf::from(arg(i + 1)?),
                    "--ui-smoke" => smoke = Some(PathBuf::from(arg(i + 1)?)),
                    _ => {}
                }
            }
            ui::run(data, smoke)
        }
    }
}
