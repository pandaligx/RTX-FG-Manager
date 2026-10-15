"""Build a schema-3 candidate from already signed payloads; never publish it.

Old catalog/archives remain untouched. Only the three new distributions are
read. No DLL execution, signature sweep, download, or rebuilding is performed.
"""
from __future__ import annotations
import argparse
import copy
import hashlib
import io
import json
from pathlib import Path
import zipfile

ROOT = Path(__file__).resolve().parents[1]
GC = "https://gitcode.com/gcw_qWIjmve7/RTX-FG-Manager-payloads/releases/download/v430/"
GH = "https://github.com/pandaligx/RTX-FG-Manager/releases/download/payloads-v430/"
RAW = "https://raw.githubusercontent.com/pandaligx/RTX-FG-Manager/main/cloud/v3/"
SCHEMES = ("upstream-0.3.5-310-9", "rtx40mfg-1.3.3-hf2", "dlssg-transfusion-1.4.5.3")

def canonical(value):
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode()

def digest(data):
    return hashlib.sha256(data).hexdigest()

def write_immutable(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        if path.read_bytes() != data:
            raise ValueError(f"Immutable candidate differs: {path}")
    else:
        with path.open("xb") as f:
            f.write(data)

def archive(entries):
    output = io.BytesIO()
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=6) as z:
        for name, data in sorted(entries.items()):
            if name.startswith("/") or ".." in Path(name).parts or "\\" in name:
                raise ValueError("Unsafe archive entry")
            info = zipfile.ZipInfo(name, (2026, 10, 15, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o100644 << 16
            z.writestr(info, data)
    return output.getvalue()

def build(output: Path, mfg_notices: Path):
    baseline = json.loads((ROOT / "rust/cloud-catalog.json").read_text(encoding="utf-8"))
    compact = json.loads((ROOT / "cloud/catalog.json").read_text(encoding="utf-8"))
    if baseline["schema"] != 2 or compact["schema"] != 2:
        raise ValueError("Use the frozen 4.2.9 schema-2 baseline; do not derive from a promoted candidate")
    packages = []
    for p in baseline["packages"]:
        if p["scheme_id"] not in SCHEMES:
            p = copy.deepcopy(p)
            # Legacy packages keep their already published URLs. Do not download
            # or re-hash all historic DLLs just to build the new catalog.
            p["urls"] = {k: s["base_url"] + p["archive"] for k, s in baseline["sources"].items()}
            packages.append(p)
    new_packages = []
    source_receipts = {}
    def source(path):
        path = path.resolve()
        data = path.read_bytes()
        source_receipts[str(path.relative_to(ROOT))] = {"bytes": len(data), "sha256": digest(data)}
        return data
    def add(scheme, backend, version, upstream, proxy, stem, entries, component=False):
        data = archive(entries)
        sha = digest(data)
        filename = f"{stem}-{sha[:12]}.zip"
        write_immutable(output / "files" / filename, data)
        p = dict(id=filename[:-4], scheme_id=scheme, label="", labels={}, version=version,
                 upstream_version=upstream, backends=[backend], proxy=proxy, archive=filename,
                 bytes=len(data), sha256=sha, component=component,
                 urls={"domestic": GC + filename, "github": GH + filename},
                 files=[dict(name=n, bytes=len(b), sha256=digest(b)) for n, b in sorted(entries.items())])
        if len(data) > 128 * 1024 * 1024 or any(len(b) > 128 * 1024 * 1024 for b in entries.values()):
            raise ValueError("Package exceeds client transfer bound")
        packages.append(p)
        new_packages.append(p)

    fg = ROOT / "payloads/dlssg-release-0.4.0-x64"
    ini = source(fg / "dlssg_sm86.ini")
    for proxy in ("version.dll", "winmm.dll", "dbghelp.dll", "dinput8.dll", "dxgi.dll", "d3d12.dll"):
        add(SCHEMES[0], "upstream040", "0.4.0", "0.4.0", proxy,
            "dlssg-0.4.0-entry-" + proxy[:-4],
            {proxy: source(fg / (proxy if proxy == "version.dll" else "alternatives/" + proxy)), "dlssg_sm86.ini": ini})
    components = ["backend/sm86_backend.dll", "kernels/fg_kernels.dll", "runtime/nvngx_dlssg.dll",
                  "xefg/xefg_kernels.dll", "sr/nvngx_dlss.dll", "sr/sr_kernels.dll"]
    for entry in components:
        add(SCHEMES[0], "upstream040", "0.4.0", "0.4.0", "version.dll",
            "dlssg-0.4.0-component-" + entry.replace("/", "-")[:-4],
            {"dlssg_sm86/" + entry: source(fg / "dlssg_sm86" / entry)}, True)
    add(SCHEMES[0], "upstream040", "0.4.0", "0.4.0", "version.dll", "dlssg-0.4.0-notices",
        {"dlssg_sm86/LICENSE.md": source(fg / "LICENSE.md"),
         "dlssg_sm86/THIRD_PARTY_NOTICES.txt": source(fg / "THIRD_PARTY_NOTICES.txt")}, True)
    add(SCHEMES[1], "rtx40mfg", "1.5.0", "1.5.0", "version.dll", "mfg-unlocker-1.5.0-universal",
        {"version.dll": source(ROOT / "payloads/MFGUnlocker-v1.5.0/MFGUnlocker.dll"),
         "RTXMFG-Universal.json": canonical(dict(followGame=True, mode="follow", multiplier=2, dynamicTargetFrameRate=0, dlssgPreset=0)),
         "MFG-Unlocker-NOTICES.txt": source(mfg_notices)})
    encore = ROOT / "payloads/RTX-Encore-v1.0.0-rc.3"
    config = source(ROOT / "rust/assets/encore-defaults.jsonc")
    notices = source(encore / "THIRD-PARTY-NOTICES.md")
    for proxy in ("winhttp.dll", "version.dll", "winmm.dll", "dxgi.dll", "dinput8.dll", "rtx-encore.asi"):
        dll = encore / ("rtx-encore.dll" if proxy == "winhttp.dll" else "alternative-proxies/" + proxy)
        add(SCHEMES[2], "encore", "1.0.0", "1.0.0-rc.3", proxy,
            "rtx-encore-1.0.0-rc.3-" + proxy.replace(".", "-"),
            {proxy: source(dll), "rtx-encore.jsonc": config, "rtx-encore-THIRD-PARTY-NOTICES.md": notices})

    policies = copy.deepcopy(baseline["scheme_policies"])
    policies[SCHEMES[0]].update(parameter_profile="upstream040", ini_policy="upstream040", max_selected_proxies=1)
    policies[SCHEMES[0]]["defaults"].update(optimized="2", fg_engine="DLSS", sr_mode="Off")
    policies[SCHEMES[2]]["capabilities"] = sorted(set(policies[SCHEMES[2]].get("capabilities", [])) | {"encore_dedicated_entries"})
    names = [
        {"zh": "dlssg_for_sm86 · 0.4.0", "en": "dlssg_for_sm86 · 0.4.0", "ru": "dlssg_for_sm86 · 0.4.0", "ja": "dlssg_for_sm86 · 0.4.0", "ko": "dlssg_for_sm86 · 0.4.0"},
        {l: "MFG Unlocker · 1.5.0" for l in ("zh", "en", "ru", "ja", "ko")},
        {l: "RTX Encore · 1.0.0-rc.3" for l in ("zh", "en", "ru", "ja", "ko")},
    ]
    schemes = copy.deepcopy(compact["schemes"])
    for i, sid in enumerate(SCHEMES):
        for p in packages:
            if p["scheme_id"] == sid:
                p.update(label=names[i]["zh"], labels=names[i])
        s = next(s for s in schemes if s["id"] == sid)
        s.update(name=names[i]["zh"], names=names[i], profile=policies[sid]["parameter_profile"],
                 defaults=policies[sid]["defaults"], capabilities=policies[sid].get("capabilities", []),
                 min_manager_version="4.3.0", archives=[p["archive"] for p in packages if p["scheme_id"] == sid])
    sources = {"domestic": {"base_url": GC}, "github": {"base_url": GH}}
    index = canonical(dict(schema=2, packages=packages))
    revision = "r-" + digest(index + canonical(schemes))[:20]
    index_name = "indexes/" + revision + ".json"
    candidate = dict(schema=3, revision=revision, default_scheme=baseline["default_scheme"], sources=sources,
        schemes=schemes, index=dict(bytes=len(index), sha256=digest(index),
            url=GC + revision + ".json", fallback_url=RAW + index_name))
    expanded = dict(schema=3, revision=revision, default_scheme=baseline["default_scheme"], sources=sources,
                    packages=packages, scheme_policies=policies)
    write_immutable(output / "cloud/v3" / index_name, index)
    write_immutable(output / "files" / (revision + ".json"), index)
    write_immutable(output / "cloud/v3/catalog.json", canonical(candidate))
    write_immutable(output / "rust/cloud-catalog.json", canonical(expanded))
    write_immutable(output / "BUILD.json", canonical(dict(revision=revision, new_packages=len(new_packages),
        legacy_packages=len(packages)-len(new_packages), source_files=source_receipts,
        note="Candidate only; original signed bytes packaged unchanged. No release/catalog promotion.")))
    print(json.dumps(dict(revision=revision, new_packages=len(new_packages), legacy_packages=len(packages)-len(new_packages), output=str(output))))

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--mfg-notices", type=Path, required=True)
    args = parser.parse_args()
    build(args.output, args.mfg_notices)
