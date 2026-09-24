#!/usr/bin/env python3
"""Build and publish immutable DLL resources, then promote the catalog.

Default operations are local. Explicit `publish` and `probe` commands write to
remote releases. No DLL is rebuilt, re-signed, installed in a game or overwritten.
"""
from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time
import unittest
import urllib.error
import urllib.parse
import urllib.request
import uuid
import zipfile

REPO = "pandaligx/RTX-FG-Manager"
GITEE_RESOURCE_REPO = "pandaligx/RTX-FG-Manager-payloads"
RESOURCE_TAG = "payloads"
MAX_ZIP = 128 * 1024 * 1024
MAX_JSON = 1024 * 1024
PROXIES = {"version.dll", "winmm.dll", "dinput8.dll", "dbghelp.dll", "dxgi.dll", "d3d12.dll", "winhttp.dll"}
GH = f"https://api.github.com/repos/{REPO}"
GT = f"https://gitee.com/api/v5/repos/{REPO}"
GT_RESOURCES = f"https://gitee.com/api/v5/repos/{GITEE_RESOURCE_REPO}"
GH_RAW = f"https://raw.githubusercontent.com/{REPO}/main/"
GT_RAW = f"https://gitee.com/{REPO}/raw/main/"


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def canonical(value):
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode("utf-8")


def digest(data):
    return hashlib.sha256(data).hexdigest()


def filename(value):
    require(isinstance(value, str) and re.fullmatch(r"[A-Za-z0-9_.-]{1,146}\.zip", value), "Invalid ZIP filename")
    return value


def archive_name(item):
    return filename(item if isinstance(item, str) else item["file"])


def validate_defaults(profile, defaults):
    boolean = {"0", "1"}
    if profile == "mfg_vulkan_sm86_7":
        choices = {"max_interpolated_frames": {"1", "2", "3", "4", "5"},
                   "force_multiplier": {"0", "2", "3", "4", "5", "6"},
                   "dynamic_target_fps": {"0", "60", "90", "120", "144", "165", "180", "240", "360"}}
        choices.update({key: boolean for key in ("dynamic_mfg", "mfg_logging", "mfg_bilinear", "conv13_shared", "conv0_shared", "residual_vector")})
        require(int(defaults.get("force_multiplier", "0")) <= int(defaults.get("max_interpolated_frames", "5")) + 1, "MFG requested multiplier exceeds its cap")
    else:
        choices = {"enabled": boolean, "logging_level": {"0", "1", "2", "3"},
                   "max_generated_frames": {"0", "1", "2", "3"}}
        if profile.startswith("upstream"):
            choices.update(optimized={"0", "1", "2", "3"} if profile == "upstream035" else boolean,
                           preset={"Auto", "A", "B"}, max_generated_frames={"0", "1", "2", "3", "4", "5"})
        elif profile == "native026":
            del choices["enabled"]
            choices.update(hardware_bilinear=boolean, max_generated_frames={"1", "2", "3"})
    require(all(key in choices and value in choices[key] for key, value in defaults.items()), "Unknown or invalid preset default")


def validate_spec(spec):
    require(spec.get("schema") == 1 and isinstance(spec.get("schemes"), list), "Invalid schemes document")
    require(1 <= len(spec["schemes"]) <= 32, "Invalid scheme count")
    ids, names = set(), set()
    for scheme in spec["schemes"]:
        sid = scheme.get("id", "")
        require(re.fullmatch(r"[A-Za-z0-9_.-]{1,150}", sid) and sid not in ids, "Duplicate/invalid scheme ID")
        ids.add(sid)
        require(isinstance(scheme.get("name"), str) and 0 < len(scheme["name"]) <= 200, "Invalid scheme name")
        require(re.fullmatch(r"\d{1,4}\.\d{1,4}\.\d{1,4}", scheme.get("version", "")), "Invalid package version")
        require(scheme.get("profile") in {"initial", "native026", "upstream031", "upstream035", "mfg_vulkan_sm86_7"}, "New protocols require a manager/tool update")
        require(isinstance(scheme.get("defaults", {}), dict), "Invalid defaults")
        validate_defaults(scheme["profile"], scheme.get("defaults", {}))
        if "min_manager_version" in scheme:
            require(re.fullmatch(r"\d{1,4}\.\d{1,4}\.\d{1,4}", scheme["min_manager_version"]), "Invalid minimum manager version")
        require(isinstance(scheme.get("archives"), list) and scheme["archives"], "Missing archives")
        for item in scheme["archives"]:
            name = archive_name(item)
            require(name not in names, "ZIP used by multiple scheme routes")
            names.add(name)
        if scheme.get("capabilities"):
            require(scheme["capabilities"] == ["delta_force_mfg_v1"] and scheme["profile"] == "upstream035", "Unknown capability")
    require(spec.get("default_scheme") in ids, "Default scheme is absent")
    require(len(names) <= 256, "Too many active archives")


def package_from_zip(scheme, item, data):
    name = archive_name(item)
    require(0 < len(data) <= MAX_ZIP, "Invalid ZIP size")
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        entries = archive.infolist()
        require(len(entries) == 2 and len({e.filename for e in entries}) == 2, "ZIP must contain exactly DLL and INI")
        dlls = [e.filename for e in entries if e.filename in PROXIES]
        require(len(dlls) == 1 and {e.filename for e in entries} == {dlls[0], "dlssg_sm86.ini"}, "Unexpected ZIP path")
        proxy = dlls[0]
        files = []
        for entry in entries:
            require(not entry.is_dir() and not entry.flag_bits & 1 and 0 < entry.file_size <= MAX_ZIP, "Invalid archive entry")
            require((entry.external_attr >> 16) & 0o170000 != 0o120000, "Symlink archive entry")
            content = archive.read(entry)
            require(len(content) == entry.file_size, "Archive size mismatch")
            if entry.filename == proxy:
                require(content[:2] == b"MZ" and len(content) >= 64, "Invalid DLL header")
                offset = int.from_bytes(content[60:64], "little")
                header = content[offset:offset + 24]
                require(len(header) == 24 and header[:6] == b"PE\0\0\x64\x86" and int.from_bytes(header[22:24], "little") & 0x2000, "Not an x64 DLL")
            else:
                content.decode("utf-8-sig")
            files.append({"name": entry.filename, "bytes": len(content), "sha256": digest(content)})
    profile = scheme["profile"]
    if profile == "initial":
        require(isinstance(item, dict) and item.get("gpu") in {"rtx20", "rtx30"}, "Initial archive needs explicit GPU route")
        backends = [item["gpu"]]
        require(proxy == "version.dll", "Initial scheme only supplies version.dll")
    elif profile == "native026":
        backends = ["native20", "native30"]
        require(proxy not in {"d3d12.dll", "dbghelp.dll"}, "Native 0.2.6 proxy mismatch")
    else:
        backends = ["upstream_sm86"]
        require(proxy != "winhttp.dll", "Upstream proxy mismatch")
        require(profile != "mfg_vulkan_sm86_7" or proxy == "version.dll", "MFG protocol only supplies version.dll")
    return {"id": item.get("id", name[:-4]) if isinstance(item, dict) else name[:-4],
            "scheme_id": scheme["id"], "label": scheme["name"], "labels": scheme.get("names", {}),
            "version": scheme["version"], "backends": backends, "proxy": proxy, "archive": name,
            "bytes": len(data), "sha256": digest(data), "files": sorted(files, key=lambda f: f["name"])}


def build_documents(spec, archives):
    validate_spec(spec)
    packages, schemes, routes = [], [], set()
    for scheme in spec["schemes"]:
        compact = {k: scheme[k] for k in ("id", "name", "names", "profile", "defaults", "capabilities", "min_manager_version") if k in scheme}
        compact["archives"] = []
        for item in scheme["archives"]:
            name = archive_name(item)
            require(name in archives, "Missing archive: " + name)
            package = package_from_zip(scheme, item, archives[name])
            for backend in package["backends"]:
                route = (scheme["id"], backend, package["proxy"])
                require(route not in routes, "Ambiguous GPU/proxy route")
                routes.add(route)
            packages.append(package)
            compact["archives"].append(name)
        schemes.append(compact)
    index = canonical({"schema": 1, "packages": packages})
    require(len(index) <= MAX_JSON, "Index too large")
    revision = "r-" + digest(canonical({"schemes": spec, "gitee_resources": GITEE_RESOURCE_REPO,
                                     "github_resources": REPO, "tag": RESOURCE_TAG}) + index)[:20]
    index_path = f"cloud/indexes/payload-index-{revision}.json"
    catalog = {"schema": 2, "revision": revision, "default_scheme": spec["default_scheme"],
               "sources": {"domestic": {"base_url": f"https://gitee.com/{GITEE_RESOURCE_REPO}/releases/download/{RESOURCE_TAG}/"},
                           "github": {"base_url": f"https://github.com/{REPO}/releases/download/{RESOURCE_TAG}/"}},
               "index": {"url": GT_RAW + index_path, "fallback_url": GH_RAW + index_path,
                         "sha256": digest(index), "bytes": len(index)}, "schemes": schemes}
    return index_path, index, canonical(catalog)


def official_url(url):
    parsed = urllib.parse.urlsplit(url)
    host = parsed.hostname or ""
    require(parsed.scheme == "https" and not parsed.username and not parsed.password and parsed.port in (None, 443), "Unsafe URL")
    require(host in {"github.com", "api.github.com", "uploads.github.com", "raw.githubusercontent.com", "objects.githubusercontent.com", "release-assets.githubusercontent.com", "gitee.com", "gitee.cn"} or host.endswith(".gitee.com"), "Unofficial transfer host")
    return url


def public_location(url):
    """Never print URL queries, fragments, userinfo or response bodies."""
    parsed = urllib.parse.urlsplit(url)
    path = parsed.path[:240]
    return (parsed.hostname or "unknown") + path


class SafeRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        official_url(newurl)
        redirected = super().redirect_request(req, fp, code, msg, headers, newurl)
        if redirected and urllib.parse.urlsplit(req.full_url).hostname != urllib.parse.urlsplit(newurl).hostname:
            redirected.remove_header("Authorization")
        return redirected


OPENER = urllib.request.build_opener(SafeRedirect())


def read_url(url, limit=MAX_JSON, headers=None):
    request = urllib.request.Request(official_url(url), headers={"User-Agent": "RTXFG-cloud-publisher", **(headers or {})})
    print("[read] GET " + public_location(url), flush=True)
    for retry in range(3):
        try:
            with OPENER.open(request, timeout=90) as response:
                official_url(response.url)
                data = response.read(limit + 1)
                require(len(data) <= limit, "Remote file exceeds size limit")
                return data
        except urllib.error.HTTPError as error:
            if error.code not in (429, 500, 502, 503, 504) or retry == 2:
                raise
        except (TimeoutError, urllib.error.URLError):
            if retry == 2:
                raise
        time.sleep(2 + retry * 2)
    raise RuntimeError("Download retry limit reached")


class Publisher:
    def __init__(self):
        self.github_token = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")
        self.gitee_token = os.environ.get("GITEE_TOKEN")
        require(self.github_token and self.gitee_token, "Both repository tokens must be supplied through the environment")

    def ensure_gitee_resource_repository(self):
        print("[stage] Check isolated Gitee resource repository", flush=True)
        try:
            repository = self.api("gitee", GT_RESOURCES)
        except urllib.error.HTTPError as error:
            if error.code != 404:
                raise
            repository = None
        if repository is None:
            print("[stage] Verify Gitee token owner before creating the resource repository", flush=True)
            owner = self.api("gitee", "https://gitee.com/api/v5/user")
            require(isinstance(owner, dict) and owner.get("login", "").lower() == "pandaligx", "Gitee token owner does not match the resource namespace")
            print("[stage] Create the public resource repository", flush=True)
            repository = self.api("gitee", "https://gitee.com/api/v5/user/repos", {
                "name": "RTX-FG-Manager-payloads", "path": "RTX-FG-Manager-payloads",
                "description": "Immutable DLL ZIPs and build tools for RTX-FG-Manager. Manager updates remain in the main repository.",
                "private": "false", "auto_init": "true"})
        require(isinstance(repository, dict) and
                repository.get("full_name", "").lower() == GITEE_RESOURCE_REPO.lower() and
                repository.get("owner", {}).get("login", "").lower() == "pandaligx" and
                repository.get("private") is False,
                "Resource repository owner/path/visibility mismatch; refusing changes")
        require(repository.get("default_branch"), "Resource repository has no initialized branch")
        return repository

    def resource_api(self, host, path, fields=None, file=None):
        return self.api(host, (GH if host == "github" else GT_RESOURCES) + path, fields, file)

    def api(self, host, path, fields=None, file=None, method=None):
        base = GH if host == "github" else GT
        url = official_url(path if path.startswith("https://") else base + path)
        print("[api] " + (method or ("POST" if fields is not None or file else "GET")) + " " + public_location(url), flush=True)
        headers = {"User-Agent": "RTXFG-cloud-publisher", "Accept": "application/json"}
        if host == "gitee" and url.startswith("https://gitee.com/api/v5/"):
            headers["Authorization"] = "Bearer " + self.gitee_token
        data = None
        if host == "github":
            headers["Authorization"] = "Bearer " + self.github_token
            if file:
                data = file.read_bytes()
                headers["Content-Type"] = "application/octet-stream"
            elif fields is not None:
                data = canonical(fields)
                headers["Content-Type"] = "application/json"
        elif fields is not None:
            fields = dict(fields, access_token=self.gitee_token)
            if file:
                boundary = "RTXFG" + uuid.uuid4().hex
                parts = []
                for key, value in fields.items():
                    parts.append(f'--{boundary}\r\nContent-Disposition: form-data; name="{key}"\r\n\r\n{value}\r\n'.encode())
                parts.extend([f'--{boundary}\r\nContent-Disposition: form-data; name="file"; filename="{file.name}"\r\nContent-Type: application/octet-stream\r\n\r\n'.encode(), file.read_bytes(), f'\r\n--{boundary}--\r\n'.encode()])
                data = b"".join(parts)
                headers["Content-Type"] = "multipart/form-data; boundary=" + boundary
            else:
                data = urllib.parse.urlencode(fields).encode()
                headers["Content-Type"] = "application/x-www-form-urlencoded"
        if data is None:
            return json.loads(read_url(url, headers=headers))
        # Never automatically repeat an uncertain POST. A subsequent invocation
        # re-reads assets/releases and checks immutable contents before resuming.
        with OPENER.open(urllib.request.Request(url, data=data, headers=headers, method=method), timeout=900) as response:
            body = response.read(MAX_JSON + 1)
            require(len(body) <= MAX_JSON, "API response too large")
            return json.loads(body) if body else None

    def release(self, host, tag, resource=True):
        try:
            method = self.resource_api if resource else self.api
            return method(host, "/releases/tags/" + urllib.parse.quote(tag, safe=""))
        except urllib.error.HTTPError as error:
            if error.code == 404:
                return None
            raise

    def assets(self, host, release, resource=True):
        result = []
        endpoint = f'/releases/{release["id"]}/' + ("assets" if host == "github" else "attach_files")
        for page in range(1, 101):
            method = self.resource_api if resource else self.api
            batch = method(host, endpoint + f"?per_page=100&page={page}")
            require(isinstance(batch, list), "Invalid attachment response")
            result.extend(batch)
            if len(batch) < 100:
                break
        else:
            raise RuntimeError("Attachment pagination limit exceeded")
        names = [a.get("name") for a in result]
        require(len(set(names)) == len(names), "Duplicate remote asset names")
        return {a["name"]: a for a in result}

    def ensure_release(self, host, tag=RESOURCE_TAG):
        require(tag in {RESOURCE_TAG, "build-tools"}, "Unexpected resource tag")
        print(f"[stage] Check {host} manager latest before resource release {tag}", flush=True)
        previous_latest = self.api(host, "/releases/latest")
        require(isinstance(previous_latest, dict) and re.fullmatch(r"v\d{1,4}\.\d{1,4}\.\d{1,4}", previous_latest.get("tag_name", "")), "Legacy latest endpoint is not a manager release; stop migration")
        branch = self.ensure_gitee_resource_repository()["default_branch"] if host == "gitee" else "main"
        current = self.release(host, tag)
        if current is None:
            print(f"[stage] Create {host} resource release {tag}", flush=True)
            data = {"tag_name": tag, "name": "Resources (not a manager update)",
                    "body": "Immutable, versioned DLL packages. Managed by the cloud publication workflow.",
                    "prerelease": True if host == "github" else "true", "target_commitish": branch}
            if host == "github":
                data.update(draft=False, make_latest="false")
            current = self.resource_api(host, "/releases", data)
        require(isinstance(current, dict) and current.get("prerelease") in (True, "true") and current.get("draft", False) in (False, "false"), "Resource release must be a public prerelease")
        latest = self.api(host, "/releases/latest")
        require(isinstance(latest, dict) and latest.get("tag_name") == previous_latest["tag_name"], "Resource release changed legacy latest; stop migration")
        return current

    @staticmethod
    def verify_asset(asset, data):
        url = asset.get("browser_download_url") or asset.get("download_url") or asset.get("url")
        actual = read_url(url, limit=len(data))
        require(len(actual) == len(data) and digest(actual) == digest(data), "Remote same-name asset differs; refusing overwrite")

    def ensure_asset(self, host, release, path):
        print(f"[stage] Check {host} attachment {path.name}", flush=True)
        current = self.assets(host, release)
        data = path.read_bytes()
        if path.name not in current:
            print(f"[stage] Upload {host} attachment {path.name}", flush=True)
            if host == "github":
                url = release["upload_url"].split("{", 1)[0] + "?name=" + urllib.parse.quote(path.name)
                self.api(host, url, {}, path)
            else:
                self.resource_api(host, f'/releases/{release["id"]}/attach_files', {}, path)
            current = self.assets(host, release)
        require(path.name in current, "Uploaded attachment is not visible")
        print(f"[stage] Anonymous readback {host} attachment {path.name}", flush=True)
        self.verify_asset(current[path.name], data)
        print("Verified " + host + ": " + path.name, flush=True)


def local_archives(spec, folders):
    result = {}
    for scheme in spec["schemes"]:
        for item in scheme["archives"]:
            name = archive_name(item)
            found = [Path(folder) / name for folder in folders if (Path(folder) / name).is_file()]
            if found:
                require(all(p.stat().st_size <= MAX_ZIP for p in found), "Local archive too large")
                data = found[0].read_bytes()
                require(all(p.read_bytes() == data for p in found[1:]), "Conflicting local archives")
                result[name] = data
    return result


def write_documents(out, documents):
    path, index, catalog = documents
    destination = out / path
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists():
        require(destination.read_bytes() == index, "Immutable index path already has different content")
    destination.write_bytes(index)
    (out / "cloud/catalog.json").write_bytes(catalog)


def promote_documents(documents, write, promote):
    """Both indexes must be available before either active catalog advances."""
    index_path, index, catalog = documents
    write(index_path, index)
    promote(index_path, index)
    write("cloud/catalog.json", catalog)
    promote("cloud/catalog.json", catalog)


def catalog_promotion_allowed(committed, active_mirror, generated, indexes_ready):
    # A failed last catalog mirror can be retried only if the committed catalog
    # is exactly today's verified candidate and both immutable indexes exist.
    return committed == active_mirror or (committed == generated and indexes_ready())


def verify_catalog_gate(root, documents):
    local = root / "cloud/catalog.json"
    require(not local.exists() or local.stat().st_size <= MAX_JSON, "Committed catalog too large")
    committed = local.read_bytes() if local.exists() else None
    try:
        active_mirror = read_url(GT_RAW + "cloud/catalog.json", MAX_JSON)
    except urllib.error.HTTPError as error:
        if error.code != 404:
            raise
        active_mirror = None
    index_path, index, generated = documents
    def indexes_ready():
        return all(read_url(base + index_path, MAX_JSON) == index for base in (GT_RAW, GH_RAW))
    require(catalog_promotion_allowed(committed, active_mirror, generated, indexes_ready),
            "Committed catalog differs from the active Gitee mirror; do not hand-edit generated metadata")


def git(root, *args, env=None):
    return subprocess.check_output(["git", "-C", str(root), *args], env=env, stderr=subprocess.DEVNULL, text=True).strip()


def push(root, host, publisher):
    with tempfile.TemporaryDirectory(prefix="rtxfg-git-") as folder:
        helper = Path(folder) / "askpass.py"
        helper.write_text('#!/usr/bin/env python3\nimport os,sys\nprint(os.environ["RTXFG_PUSH_USER"] if "username" in sys.argv[1].lower() else os.environ["RTXFG_PUSH_TOKEN"])\n', encoding="utf-8")
        helper.chmod(0o700)
        env = dict(os.environ, GIT_ASKPASS=str(helper), GIT_TERMINAL_PROMPT="0",
                   RTXFG_PUSH_USER="x-access-token" if host == "github" else REPO.split("/")[0],
                   RTXFG_PUSH_TOKEN=publisher.github_token if host == "github" else publisher.gitee_token)
        git(root, "-c", "credential.helper=", "push", f"https://{host}.com/{REPO}.git", "HEAD:refs/heads/main", env=env)


def publish(args, spec):
    root = Path(args.root).resolve()
    require((root / ".git").exists(), "Publish only from the reviewed public repository checkout")
    origin = git(root, "remote", "get-url", "origin").removesuffix(".git")
    require(origin in {f"https://github.com/{REPO}", f"git@github.com:{REPO}"}, "Unexpected publication repository")
    require(not git(root, "status", "--porcelain"), "Publication checkout must be clean")
    publisher = Publisher()
    # Do not mirror main here: a user may have edited generated metadata in the
    # triggering commit. No Gitee main push is allowed before every ZIP passes
    # two-site readback. Resource releases live in an independently initialized
    # Gitee repository and no longer need a preliminary manager-repo push.
    releases = {host: publisher.ensure_release(host) for host in ("github", "gitee")}
    github_assets = publisher.assets("github", releases["github"])
    seed = publisher.release("github", args.seed_release) if args.seed_release else None
    seed_assets = publisher.assets("github", seed) if seed else {}
    archives = local_archives(spec, args.archives)
    for scheme in spec["schemes"]:
        for item in scheme["archives"]:
            name = archive_name(item)
            if name not in archives:
                asset = github_assets.get(name) or seed_assets.get(name)
                require(asset is not None, "Upload the signed ZIP to the payloads prerelease first: " + name)
                archives[name] = read_url(asset["browser_download_url"], MAX_ZIP)
    documents = build_documents(spec, archives)
    with tempfile.TemporaryDirectory(prefix="rtxfg-payloads-") as temp:
        for name, data in archives.items():
            path = Path(temp) / name
            path.write_bytes(data)
            for host in ("github", "gitee"):
                publisher.ensure_asset(host, releases[host], path)
    verify_catalog_gate(root, documents)
    # Every referenced ZIP now exists and was anonymously read back from both
    # sites. Only at this boundary may active metadata be written and committed.
    index_path, index, catalog = documents
    def write(relative, data):
        destination = root / relative
        if relative != "cloud/catalog.json" and destination.exists():
            require(destination.read_bytes() == data, "Immutable index already has different content")
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(data)
    def promote(relative, data):
        git(root, "add", "--", relative)
        if git(root, "diff", "--cached", "--name-only"):
            git(root, "-c", "user.name=RTXFG Cloud Publisher", "-c", "user.email=actions@users.noreply.github.com",
                "commit", "-m", "Publish verified DLL metadata " + relative)
        # Distributed git promotion is not atomic. GitHub first makes a failed
        # Gitee push resumable from the next checkout. Old mirrors stay valid;
        # every ZIP and both indexes precede the first catalog promotion.
        for host in ("github", "gitee"):
            push(root, host, publisher)
            base = GT_RAW if host == "gitee" else GH_RAW
            require(read_url(base + relative, MAX_JSON) == data, "Catalog/index mirror readback mismatch")
    promote_documents(documents, write, promote)
    print("Cloud publication completed: " + json.loads(catalog)["revision"], flush=True)


def probe(args):
    """Explicit remote smoke test: copy one immutable public GitHub asset to
    Gitee and verify its anonymous download. Never writes an active catalog."""
    require(args.source_tag and args.asset and args.expected_sha256, "Probe requires source tag, asset and expected SHA-256")
    require(re.fullmatch(r"[A-Za-z0-9_.-]{1,150}", args.asset), "Invalid probe asset name")
    require(re.fullmatch(r"[0-9a-f]{64}", args.expected_sha256), "Invalid probe digest")
    publisher = Publisher()
    print("[stage] Locate the verified GitHub source asset", flush=True)
    source = publisher.release("github", args.source_tag)
    require(source is not None, "Source resource release is absent")
    asset = publisher.assets("github", source).get(args.asset)
    require(asset is not None, "Source asset is absent")
    started = time.monotonic()
    print("[stage] Download and verify the GitHub source asset", flush=True)
    data = read_url(asset["browser_download_url"], MAX_ZIP)
    require(digest(data) == args.expected_sha256, "Probe source digest mismatch")
    target = publisher.ensure_release("gitee", args.destination_tag)
    with tempfile.TemporaryDirectory(prefix="rtxfg-cloud-probe-") as folder:
        path = Path(folder) / args.asset
        path.write_bytes(data)
        publisher.ensure_asset("gitee", target, path)
    report = {"passed": True, "asset": args.asset, "bytes": len(data), "sha256": digest(data),
              "source_tag": args.source_tag, "destination_tag": args.destination_tag,
              "seconds": round(time.monotonic() - started, 3), "catalog_promoted": False,
              "url": f"https://gitee.com/{GITEE_RESOURCE_REPO}/releases/download/{args.destination_tag}/{args.asset}"}
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    (out / (args.asset + ".probe.json")).write_bytes(canonical(report))
    print(json.dumps(report), flush=True)


def cleanup_probe(args):
    """Compensate this experiment's empty Gitee build-tools release only.
    Never deletes a tag, historical release, or any attachment."""
    require(args.destination_tag == "build-tools", "Cleanup only permits the empty build-tools probe release")
    publisher = Publisher()
    release = publisher.release("gitee", "build-tools", resource=False)
    require(isinstance(release, dict) and release.get("tag_name") == "build-tools", "Probe release is absent or unexpected")
    require(release.get("name") == "Resources (not a manager update)" and
            release.get("body") == "Immutable, versioned DLL packages. Managed by the cloud publication workflow.",
            "Release does not match this probe; refusing removal")
    require(not publisher.assets("gitee", release, resource=False), "Probe release has attachments; refusing removal")
    latest = publisher.api("gitee", "/releases/latest")
    require(latest.get("tag_name") == "build-tools" and latest.get("id") == release.get("id"), "Unexpected latest release; stop cleanup")
    require(isinstance(release.get("id"), int) and release["id"] > 0, "Invalid probe release ID")
    publisher.api("gitee", f'/releases/{release["id"]}', {}, method="DELETE")
    require(publisher.release("gitee", "build-tools", resource=False) is None, "Probe release deletion was not confirmed")
    latest = publisher.api("gitee", "/releases/latest")
    require(latest.get("tag_name") == "v4.2.2", "Latest did not return to v4.2.2; stop migration")
    print("Removed only the empty Gitee build-tools probe release; latest restored to v4.2.2; tags and assets untouched.")


class OfflineTests(unittest.TestCase):
    @staticmethod
    def fixture():
        pe = bytearray(128)
        pe[:2] = b"MZ"
        pe[60:64] = (64).to_bytes(4, "little")
        pe[64:70] = b"PE\0\0\x64\x86"
        pe[86:88] = (0x2000).to_bytes(2, "little")
        stream = io.BytesIO()
        with zipfile.ZipFile(stream, "w") as archive:
            archive.writestr("version.dll", pe)
            archive.writestr("dlssg_sm86.ini", "[General]\nEnabled=1\n")
        spec = {"schema": 1, "default_scheme": "test", "schemes": [{"id": "test", "name": "Test", "profile": "upstream035", "version": "0.3.5", "archives": ["test-r1-version.zip"]}]}
        return spec, {"test-r1-version.zip": stream.getvalue()}

    def test_deterministic_revision_and_pair(self):
        spec, archives = self.fixture()
        first = build_documents(spec, archives)
        self.assertEqual(first, build_documents(spec, archives))
        index_path, index, catalog = first
        parsed = json.loads(catalog)
        self.assertEqual(parsed["index"]["sha256"], digest(index))
        self.assertTrue(parsed["index"]["url"].endswith(index_path))

    def test_reject_missing_or_ambiguous_archive(self):
        spec, archives = self.fixture()
        with self.assertRaises(RuntimeError): build_documents(spec, {})
        spec["schemes"][0]["archives"].append("test-r1-version.zip")
        with self.assertRaises(RuntimeError): build_documents(spec, archives)

    def test_no_traversal(self):
        spec, archives = self.fixture()
        spec["schemes"][0]["archives"] = ["../escape.zip"]
        with self.assertRaises(RuntimeError): build_documents(spec, archives)

    def test_immutable_index(self):
        spec, archives = self.fixture()
        documents = build_documents(spec, archives)
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            write_documents(root, documents)
            (root / documents[0]).write_bytes(b"different")
            with self.assertRaises(RuntimeError): write_documents(root, documents)

    def test_reject_unsigned_transport_not_package_signature(self):
        for url in ("http://gitee.com/file", "https://github.com@evil.invalid/file", "https://gitee.com.evil.invalid/file"):
            with self.assertRaises(RuntimeError): official_url(url)

    def test_invalid_defaults_stop_publication(self):
        spec, archives = self.fixture()
        spec["schemes"][0]["defaults"] = {"optimized": "99"}
        with self.assertRaises(RuntimeError): build_documents(spec, archives)

    def test_index_mirror_failure_never_writes_catalog(self):
        events = []
        def write(path, data): events.append(("write", path))
        def promote(path, data):
            events.append(("promote", path))
            raise RuntimeError("mirror unavailable")
        with self.assertRaises(RuntimeError):
            promote_documents(("cloud/indexes/immutable.json", b"index", b"catalog"), write, promote)
        self.assertEqual(events, [("write", "cloud/indexes/immutable.json"), ("promote", "cloud/indexes/immutable.json")])

    def test_index_precedes_catalog_and_repeat_is_deterministic(self):
        spec, archives = self.fixture()
        documents = build_documents(spec, archives)
        events = []
        promote_documents(documents, lambda path, data: events.append(("write", path)),
                          lambda path, data: events.append(("promote", path)))
        self.assertEqual(events, [("write", documents[0]), ("promote", documents[0]),
                                  ("write", "cloud/catalog.json"), ("promote", "cloud/catalog.json")])

    def test_resource_release_cannot_take_latest(self):
        class Fake(Publisher):
            def __init__(self): self.created = False
            def ensure_gitee_resource_repository(self): return {"default_branch": "main"}
            def resource_api(self, host, path, fields=None, file=None): return self.api(host, path, fields, file)
            def api(self, host, path, data=None, file=None):
                if path == "/releases/latest":
                    return {"tag_name": "payloads" if self.created else "v4.2.2"}
                if path == "/releases":
                    self.created = True
                    return {"tag_name": "payloads", "prerelease": True}
                return None
        with self.assertRaisesRegex(RuntimeError, "changed legacy latest"):
            Fake().ensure_release("gitee")

    def test_gitee_resource_repo_is_separate_from_manager_repo(self):
        spec, archives = self.fixture()
        catalog = json.loads(build_documents(spec, archives)[2])
        self.assertIn("/RTX-FG-Manager-payloads/", catalog["sources"]["domestic"]["base_url"])
        self.assertIn("/RTX-FG-Manager/raw/", catalog["index"]["url"])

    def test_wrong_gitee_owner_does_not_create_repository(self):
        class Fake(Publisher):
            def __init__(self): self.calls = []
            def api(self, host, path, fields=None, file=None):
                self.calls.append((path, fields))
                return {"login": "someone-else"} if path.endswith("/user") else None
        publisher = Fake()
        with self.assertRaisesRegex(RuntimeError, "owner does not match"):
            publisher.ensure_gitee_resource_repository()
        self.assertFalse(any(path.endswith("/user/repos") for path, _ in publisher.calls))

    def test_existing_private_resource_repository_is_never_changed(self):
        class Fake(Publisher):
            def __init__(self): self.calls = []
            def api(self, host, path, fields=None, file=None):
                self.calls.append((path, fields))
                return {"full_name": GITEE_RESOURCE_REPO, "owner": {"login": "pandaligx"}, "private": True}
        publisher = Fake()
        with self.assertRaisesRegex(RuntimeError, "visibility mismatch"):
            publisher.ensure_gitee_resource_repository()
        self.assertEqual(publisher.calls, [(GT_RESOURCES, None)])

    def test_catalog_gate_rejects_hand_edited_metadata_before_any_push(self):
        self.assertFalse(catalog_promotion_allowed(b"hand-edited", b"old", b"generated", lambda: True))
        self.assertFalse(catalog_promotion_allowed(b"generated", b"old", b"generated", lambda: False))
        self.assertTrue(catalog_promotion_allowed(b"generated", b"old", b"generated", lambda: True))
        self.assertTrue(catalog_promotion_allowed(None, None, b"generated", lambda: False))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("prepare", "publish", "probe", "cleanup-probe", "self-test"))
    parser.add_argument("--root", default=".")
    parser.add_argument("--schemes", default="cloud/schemes.json")
    parser.add_argument("--archives", action="append", default=[])
    parser.add_argument("--out", default="cloud-candidate")
    parser.add_argument("--seed-release", help="One-time import from an existing GitHub resource release")
    parser.add_argument("--source-tag")
    parser.add_argument("--asset")
    parser.add_argument("--destination-tag", default=RESOURCE_TAG, choices=(RESOURCE_TAG, "build-tools"))
    parser.add_argument("--expected-sha256")
    args = parser.parse_args()
    if args.command == "self-test":
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(OfflineTests)
        require(unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful(), "Offline tests failed")
        return
    if args.command == "probe":
        probe(args)
        return
    if args.command == "cleanup-probe":
        cleanup_probe(args)
        return
    spec = json.loads(Path(args.schemes).read_text(encoding="utf-8-sig"))
    validate_spec(spec)
    if args.command == "prepare":
        write_documents(Path(args.out), build_documents(spec, local_archives(spec, args.archives)))
        print("Prepared local candidate only; nothing uploaded or published.")
    else:
        publish(args, spec)


if __name__ == "__main__":
    try:
        main()
    except urllib.error.HTTPError as error:
        # URL/query strings and API response bodies may contain signed links.
        print(f"Cloud publication stopped: HTTP {error.code} at {public_location(error.url)}; active metadata was not promoted before asset verification.", file=sys.stderr)
        raise SystemExit(1) from None
    except Exception as error:
        print(f"Cloud publication stopped: {type(error).__name__}: {error}", file=sys.stderr)
        raise SystemExit(1) from None
