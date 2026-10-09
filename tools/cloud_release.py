#!/usr/bin/env python3
"""Build and publish immutable DLL resources, then promote the catalog.

Default operations are local. Explicit `publish` and `probe` commands write to
remote releases. No DLL is rebuilt, re-signed, installed in a game or overwritten.
"""
from __future__ import annotations

import argparse
from concurrent.futures import FIRST_COMPLETED, ThreadPoolExecutor, wait
from contextlib import nullcontext
import hashlib
import http.client
import io
import json
import math
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch
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
ENCORE_PROFILE = "rtx_encore_json_v4"
ENCORE_CONFIG = "rtx-encore.jsonc"
ENCORE_NOTICES = "rtx-encore-THIRD-PARTY-NOTICES.md"
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
    if profile == ENCORE_PROFILE:
        fields = {field["key"]: field for field in encore_fields()}
        require(all(key in fields and encore_value_valid(fields[key], value)
                    for key, value in defaults.items()), "Unknown or invalid Encore preset default")
        return
    elif profile == "transfusion_json_v3":
        choices = {"tf_mode": {"game", "2", "3", "4", "5", "6", "dynamic"}, "tf_target": {str(n) for n in range(1001)}, "tf_dynamic56": boolean, "tf_overlay": boolean}
    elif profile == "rtxmfg_universal_133":
        choices = {"rtx_mode": {"follow", "1", "2", "3", "4", "5", "6", "dynamic"}, "rtx_target": {str(n) for n in range(1001)}, "rtx_preset": {"0", "1", "2"}, "rtx_vsync": {"0", "1"}, "rtx_reflex_limit": {str(n) for n in range(1001)}}
    elif profile == "mfg_vulkan_sm86_7":
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
        require(scheme.get("profile") in {"initial", "native026", "upstream031", "upstream035", "mfg_vulkan_sm86_7", "rtxmfg_universal_133", "transfusion_json_v3", ENCORE_PROFILE}, "New protocols require a manager/tool update")
        if "upstream_version" in scheme:
            upstream = scheme["upstream_version"]
            require(isinstance(upstream, str) and len(upstream) <= 100 and re.fullmatch(
                r"\d{1,4}\.\d{1,4}\.\d{1,4}(?:-[A-Za-z0-9]+(?:[.-][A-Za-z0-9]+)*)?(?:\+[A-Za-z0-9]+(?:[.-][A-Za-z0-9]+)*)?", upstream)
                and re.split(r"[-+]", upstream, maxsplit=1)[0] == scheme["version"], "Invalid upstream package version")
        require(isinstance(scheme.get("defaults", {}), dict), "Invalid defaults")
        validate_defaults(scheme["profile"], scheme.get("defaults", {}))
        if "min_manager_version" in scheme:
            require(re.fullmatch(r"\d{1,4}\.\d{1,4}\.\d{1,4}", scheme["min_manager_version"]), "Invalid minimum manager version")
        if scheme["profile"] == ENCORE_PROFILE:
            minimum = scheme.get("min_manager_version", "0.0.0")
            require(tuple(map(int, minimum.split("."))) >= (4, 2, 9), "Encore requires manager 4.2.9 or newer")
        if "source_url" in scheme:
            require(re.fullmatch(r"https://github\.com/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", scheme["source_url"]), "Invalid upstream project URL")
        require(isinstance(scheme.get("archives"), list) and scheme["archives"], "Missing archives")
        if scheme["profile"] == ENCORE_PROFILE:
            require(len(scheme["archives"]) == 1, "Encore uses one canonical universal archive")
        for item in scheme["archives"]:
            name = archive_name(item)
            require(name not in names, "ZIP used by multiple scheme routes")
            names.add(name)
        if scheme.get("capabilities"):
            require(scheme["capabilities"] == ["delta_force_mfg_v1"] and scheme["profile"] == "upstream035", "Unknown capability")
    require(spec.get("default_scheme") in ids, "Default scheme is absent")
    require(len(names) <= 256, "Too many active archives")


def jsonc_loads(text):
    # Remove comments only outside strings; URLs and escaped quotes are literal.
    stripped = re.sub(r'"(?:\\.|[^"\\])*"|//[^\r\n]*|/\*[\s\S]*?\*/',
                      lambda m: m.group() if m.group().startswith('"') else ' ', text)
    def no_duplicates(items):
        result = {}
        for key, value in items:
            require(key not in result, "Duplicate JSON configuration key")
            result[key] = value
        return result
    return json.loads(stripped, object_pairs_hook=no_duplicates)


def encore_fields():
    """Share the reviewed native schema with the manager; never infer paths from prose."""
    path = Path(__file__).resolve().parents[1] / "rust/assets/encore-fields.json"
    fields = json.loads(path.read_text(encoding="utf-8-sig"))
    require(isinstance(fields, list) and fields, "Missing Encore field metadata")
    return fields


def encore_value_valid(field, value):
    if not isinstance(value, str):
        return False
    kind = field["kind"]
    if kind == "bool":
        return value in {"0", "1"}
    if kind == "choice":
        return value in {choice["value"] for choice in field["choices"]}
    if kind in {"integer", "decimal"}:
        try:
            if kind == "integer" and not re.fullmatch(r"[0-9]+", value):
                return False
            if kind == "decimal" and not re.fullmatch(r"[+-]?(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)(?:[eE][+-]?[0-9]+)?", value):
                return False
            number = int(value) if kind == "integer" else float(value)
            return (math.isfinite(number)
                    and (kind != "integer" or number <= 18446744073709551615)
                    and (kind != "decimal" or abs(number) <= 3.4028234663852886e38)
                    and (field["min"] is None or number >= float(field["min"]))
                    and (field["max"] is None or number <= float(field["max"])))
        except (ValueError, OverflowError):
            return False
    if kind == "hotkey":
        return encore_hotkey_valid(value)
    return False


def encore_hotkey_valid(value):
    if not value:
        return True
    if len(value.encode("utf-8")) > 512 or len(value.split(",")) > 16:
        return False
    named = {"pageup", "pagedown", "end", "home", "left", "right", "up", "down", "insert", "delete",
             "space", "tab", "backspace", "enter", "pause", "esc", "escape", "plus", "comma", "minus", "period",
             "nummultiply", "numadd", "numsubtract", "numdecimal", "numdivide"}
    for combination in value.split(","):
        parts = [part.strip().lower() for part in combination.split("+")]
        if not all(parts):
            return False
        modifiers = ["ctrl" if part == "control" else part for part in parts[:-1]]
        if any(part not in {"ctrl", "alt", "shift", "win"} for part in modifiers) or len(set(modifiers)) != len(modifiers):
            return False
        key = parts[-1]
        function = re.fullmatch(r"f([0-9]+)", key)
        if not ((len(key) == 1 and key.isascii() and key.isalnum()) or key in named
                or (function and 1 <= int(function[1]) <= 24) or re.fullmatch(r"num[0-9]", key)):
            return False
    return True


def validate_encore_config(config):
    require(isinstance(config, dict) and type(config.get("configVersion")) is int
            and config["configVersion"] == 4, "Encore requires JSONC v4")
    for field in encore_fields():
        value = config
        missing = False
        for part in field["path"]:
            require(isinstance(value, dict), "Invalid Encore section: " + ".".join(field["path"]))
            if part not in value:
                require(field.get("optional", False), "Missing Encore field: " + field["key"])
                missing = True
                break
            value = value[part]
        if missing:
            continue
        kind = field["kind"]
        if field["key"] == "tf_mode":
            require(isinstance(value, str) and value in {"game", "fixed", "dynamic"}, "Invalid Encore mode")
            continue
        if kind == "bool":
            valid_type = type(value) is bool
            encoded = "1" if value is True else "0"
        elif kind == "integer":
            valid_type = type(value) is int
            encoded = str(value)
        elif kind == "decimal":
            valid_type = type(value) in {int, float}
            encoded = str(value)
        elif kind == "choice" and all(re.fullmatch(r"[0-9]+", choice["value"]) for choice in field["choices"]):
            valid_type = type(value) is int
            encoded = str(value)
        else:
            valid_type = isinstance(value, str)
            encoded = value
        require(valid_type and encore_value_valid(field, encoded), "Invalid Encore field: " + field["key"])
    fg = config["frameGeneration"]
    require(type(fg.get("multiplier")) is int and 2 <= fg["multiplier"] <= 6, "Invalid Encore multiplier")
    require(fg["mode"] == "game", "Cloud Encore must follow the game by default")
    require(config["general"]["gpuSeries"] == "auto", "Cloud Encore must auto-detect GPU")
    require(config["smoothMotion"]["smoothMotionEnabled"] is False, "Cloud Encore must not enable driver Smooth Motion")
    require(config["neuralRendering"]["core"]["nrEnabled"] is False, "Cloud Encore must not enable Neural Rendering")


def package_from_zip(scheme, item, data):
    name = archive_name(item)
    require(0 < len(data) <= MAX_ZIP, "Invalid ZIP size")
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        entries = archive.infolist()
        encore = scheme["profile"] == ENCORE_PROFILE
        expected_count = 3 if encore else 2
        require(len(entries) == expected_count and len({e.filename for e in entries}) == expected_count,
                "Encore ZIP must contain exactly DLL, configuration and notices" if encore else "ZIP must contain exactly DLL and its configuration")
        dlls = [e.filename for e in entries if e.filename in PROXIES]
        config_name = {"rtxmfg_universal_133": "RTXMFG-Universal.json", "transfusion_json_v3": "DLSSG-Transfusion.json", ENCORE_PROFILE: ENCORE_CONFIG}.get(scheme["profile"], "dlssg_sm86.ini")
        require(len(dlls) == 1, "ZIP must contain one supported DLL")
        expected_names = {dlls[0], config_name} | ({ENCORE_NOTICES} if encore else set())
        require({e.filename for e in entries} == expected_names, "Unexpected ZIP path")
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
                text = content.decode("utf-8-sig")
                if entry.filename == ENCORE_NOTICES:
                    require(text.strip(), "Empty Encore third-party notices")
                elif encore:
                    validate_encore_config(jsonc_loads(text))
                elif scheme["profile"] == "transfusion_json_v3":
                    config = jsonc_loads(text)
                    require(isinstance(config, dict) and config.get("configVersion") == 3, "Transfusion requires JSONC v3")
                    fg = config.get("frameGeneration", {})
                    require(isinstance(fg, dict) and fg.get("mode") in {"game", "fixed", "dynamic"}, "Invalid Transfusion mode")
                    require(type(fg.get("multiplier")) is int and 2 <= fg["multiplier"] <= 6, "Invalid Transfusion multiplier")
                    require(type(fg.get("dynamicTargetFrameRate")) is int and 0 <= fg["dynamicTargetFrameRate"] <= 1000, "Invalid Transfusion target")
                    require(type(fg.get("dynamicExperimental56")) is bool, "Invalid Transfusion dynamic56")
                    require(config.get("general", {}).get("gpuArchitecture") == "auto", "Cloud Transfusion config must auto-detect GPU")
                    require(config.get("compatibility", {}).get("smoothMotionSm86") is False, "Cloud Transfusion must not enable experimental driver Smooth Motion")
                elif scheme["profile"] == "rtxmfg_universal_133":
                    config = json.loads(text)
                    require(isinstance(config, dict), "RTXMFG config must be a JSON object")
                    require(type(config.get("multiplier")) is int and 1 <= config["multiplier"] <= 6, "Invalid RTXMFG multiplier")
                    require(type(config.get("followGame", False)) is bool, "Invalid RTXMFG followGame")
                    require(config.get("mode", "fixed") in {"fixed", "dynamic", "follow"}, "Invalid RTXMFG mode")
                    require(config.get("mode") != "follow" or config.get("followGame") is True, "RTXMFG follow mode requires followGame")
                    for key, maximum in (("dynamicTargetFrameRate", 1000), ("dlssgPreset", 2), ("vsyncMode", 2), ("reflexFrameLimitFps", 1000)):
                        if key in config:
                            require(type(config[key]) is int and 0 <= config[key] <= maximum, "Invalid RTXMFG field: " + key)
            files.append({"name": entry.filename, "bytes": len(content), "sha256": digest(content)})
    profile = scheme["profile"]
    if profile == "initial":
        require(isinstance(item, dict) and item.get("gpu") in {"rtx20", "rtx30"}, "Initial archive needs explicit GPU route")
        backends = [item["gpu"]]
        require(proxy == "version.dll", "Initial scheme only supplies version.dll")
    elif profile == "rtxmfg_universal_133":
        backends = ["rtx40mfg"]
        require(proxy == "version.dll", "Universal archive must use canonical version.dll")
    elif profile == "transfusion_json_v3":
        backends = ["transfusion"]
        require(proxy in {"version.dll", "dinput8.dll", "dxgi.dll", "winmm.dll"}, "Transfusion proxy mismatch")
    elif profile == ENCORE_PROFILE:
        backends = ["encore"]
        require(proxy == "version.dll", "Encore archive must use canonical version.dll")
    elif profile == "native026":
        backends = ["native20", "native30"]
        require(proxy not in {"d3d12.dll", "dbghelp.dll"}, "Native 0.2.6 proxy mismatch")
    else:
        backends = ["upstream_sm86"]
        require(proxy != "winhttp.dll", "Upstream proxy mismatch")
        require(profile != "mfg_vulkan_sm86_7" or proxy == "version.dll", "MFG protocol only supplies version.dll")
    package = {"id": item.get("id", name[:-4]) if isinstance(item, dict) else name[:-4],
            "scheme_id": scheme["id"], "label": scheme["name"], "labels": scheme.get("names", {}),
            "version": scheme["version"], "backends": backends, "proxy": proxy, "archive": name,
            "bytes": len(data), "sha256": digest(data), "files": sorted(files, key=lambda f: f["name"])}
    if "upstream_version" in scheme:
        package["upstream_version"] = scheme["upstream_version"]
    return package


def build_documents(spec, archives):
    validate_spec(spec)
    packages, schemes, routes = [], [], set()
    for scheme in spec["schemes"]:
        compact = {k: scheme[k] for k in ("id", "name", "names", "profile", "defaults", "capabilities", "min_manager_version", "source_url") if k in scheme}
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
    require(host in {"github.com", "api.github.com", "uploads.github.com", "raw.githubusercontent.com", "objects.githubusercontent.com", "release-assets.githubusercontent.com", "gitee.com", "gitee.cn", "raw.giteeusercontent.com"} or host.endswith(".gitee.com"), "Unofficial transfer host")
    return url


def public_location(url):
    """Never print URL queries, fragments, userinfo or response bodies."""
    parsed = urllib.parse.urlsplit(url)
    path = parsed.path[:240]
    return (parsed.hostname or "unknown") + path


def safe_error_fields(body, secrets):
    try:
        value = json.loads(body)
    except (ValueError, UnicodeDecodeError):
        return {"message": "Non-JSON API error body omitted"}
    if not isinstance(value, dict):
        return {"message": "Unexpected API error body omitted"}
    result = {}
    for key in ("message", "error", "errors"):
        item = value.get(key)
        if item is None:
            continue
        text = json.dumps(item, ensure_ascii=True)
        for secret in secrets:
            if secret:
                text = text.replace(secret, "[REDACTED]")
        text = re.sub(r"https?://[^\s\"<>]+", lambda match: public_location(match.group()), text)
        result[key] = text[:600]
    return result or {"message": "No public error fields"}


def safe_exception(error):
    if isinstance(error, urllib.error.HTTPError):
        return f"HTTP {error.code} at {public_location(error.url)}"
    # Our validation exceptions contain controlled diagnostic facts. Sanitize
    # anyway so a future caller cannot expose a token or signed redirect URL.
    if isinstance(error, RuntimeError):
        secrets = tuple(os.environ.get(key) for key in ("GH_TOKEN", "GITHUB_TOKEN", "GITEE_TOKEN"))
        fields = safe_error_fields(canonical({"message": str(error)}), secrets)
        return type(error).__name__ + ": " + fields["message"]
    return type(error).__name__


class TransientResponse(RuntimeError):
    """A bounded read returned a web/error page instead of the requested file."""


class SafeRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        official_url(newurl)
        redirected = super().redirect_request(req, fp, code, msg, headers, newurl)
        if redirected and urllib.parse.urlsplit(req.full_url).hostname != urllib.parse.urlsplit(newurl).hostname:
            redirected.remove_header("Authorization")
        return redirected


HTTP_STATE = threading.local()
GITEE_READ_SLOTS = threading.BoundedSemaphore(2)


def http_opener():
    # urllib handlers keep request state; never share one handler chain between
    # concurrent assets. Authentication remains scoped to each request.
    if not hasattr(HTTP_STATE, "opener"):
        HTTP_STATE.opener = urllib.request.build_opener(SafeRedirect())
    return HTTP_STATE.opener


def read_url(url, limit=MAX_JSON, headers=None, attempts=3, timeout=90):
    require(isinstance(attempts, int) and 1 <= attempts <= 3 and 1 <= timeout <= 90,
            "Invalid bounded read options")
    request = urllib.request.Request(official_url(url), headers={"User-Agent": "RTXFG-cloud-publisher", **(headers or {})})
    parsed = urllib.parse.urlsplit(url)
    is_zip = parsed.path.lower().endswith(".zip")
    is_json = "/api/" in parsed.path or parsed.path.lower().endswith(".json")
    print("[read] GET " + public_location(url), flush=True)
    for retry in range(attempts):
        delay = 5 + retry * 10
        try:
            with GITEE_READ_SLOTS if (parsed.hostname or "").endswith("gitee.com") else nullcontext():
                with http_opener().open(request, timeout=timeout) as response:
                    official_url(response.url)
                    content_type = re.sub(r"[^A-Za-z0-9/;=_. -]", "", response.headers.get("Content-Type", ""))[:80]
                    data = response.read(limit + 1)
            prefix = data.lstrip()[:64].lower()
            html = "text/html" in content_type.lower() or prefix.startswith((b"<!doctype html", b"<html"))
            json_for_zip = is_zip and prefix.startswith((b"{", b"["))
            if (html and (is_json or is_zip)) or json_for_zip:
                raise TransientResponse(f"Received {'HTML' if html else 'JSON'} instead of {'ZIP' if is_zip else 'JSON'}; "
                                        f"content_type={content_type}, bytes={len(data)}")
            require(len(data) <= limit, f"Remote file exceeds size limit: limit={limit}, received_at_least={len(data)}")
            return data
        except urllib.error.HTTPError as error:
            if error.code not in (429, 500, 502, 503, 504) or retry == attempts - 1:
                raise
            reason = safe_exception(error)
            retry_after = error.headers.get("Retry-After", "") if error.headers else ""
            if retry_after.isdigit():
                delay = min(60, max(delay, int(retry_after)))
        except (TimeoutError, urllib.error.URLError, TransientResponse,
                http.client.RemoteDisconnected, http.client.IncompleteRead,
                ConnectionResetError, ConnectionAbortedError, BrokenPipeError) as error:
            if retry == attempts - 1:
                raise
            reason = safe_exception(error)
        print(f"[read-retry] {public_location(url)}: {reason}; retry {retry + 2}/{attempts} after {delay}s", flush=True)
        time.sleep(delay)
    raise RuntimeError("Download retry limit reached")


class Publisher:
    def __init__(self):
        self.github_token = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")
        self.gitee_token = os.environ.get("GITEE_TOKEN")
        require(self.github_token and self.gitee_token, "Both repository tokens must be supplied through the environment")

    def public_gitee_resource_repository(self):
        return self.api("gitee", GT_RESOURCES)

    def ensure_gitee_resource_repository(self):
        print("[stage] Check isolated Gitee resource repository", flush=True)
        try:
            # Public discovery must not be filtered by token-scoped visibility
            # or a cached authenticated null response. Writes still require the
            # configured token; this does not broaden its permissions.
            repository = self.public_gitee_resource_repository()
        except urllib.error.HTTPError as error:
            if error.code != 404:
                raise
            repository = None
        if repository is None:
            print("[stage] Verify Gitee token owner before creating the resource repository", flush=True)
            owner = self.api("gitee", "https://gitee.com/api/v5/user")
            require(isinstance(owner, dict) and owner.get("login", "").lower() == "pandaligx", "Gitee token owner does not match the resource namespace")
            print("[stage] Create the public resource repository", flush=True)
            try:
                repository = self.api("gitee", "https://gitee.com/api/v5/user/repos", {
                    "name": "RTX-FG-Manager-payloads", "path": "RTX-FG-Manager-payloads",
                    "description": "Immutable DLL ZIPs and build tools for RTX-FG-Manager. Manager updates remain in the main repository.",
                    "private": "false", "auto_init": "true"})
            except urllib.error.HTTPError as error:
                if error.code == 403:
                    raise RuntimeError("Gitee denied API repository creation (HTTP 403). Create the public pandaligx/RTX-FG-Manager-payloads repository once in the signed-in Gitee website, initialize README, then rerun. Do not change token or repository permissions automatically.") from None
                raise
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
        if host == "gitee" and url == "https://gitee.com/api/v5/user":
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
            try:
                return json.loads(read_url(url, headers=headers))
            except urllib.error.HTTPError as error:
                parsed, failed = urllib.parse.urlsplit(url), urllib.parse.urlsplit(error.url)
                roots = (urllib.parse.urlsplit(GT).path, urllib.parse.urlsplit(GT_RESOURCES).path)
                inventory = any(parsed.path in (root, root + "/releases")
                                or parsed.path.startswith(root + "/releases/")
                                for root in roots)
                eligible = (host == "gitee" and method in (None, "GET") and file is None
                            and "Authorization" not in headers and error.code == 403
                            and parsed.scheme == failed.scheme == "https"
                            and parsed.netloc == failed.netloc == "gitee.com"
                            and failed.path == parsed.path and inventory)
                if not eligible:
                    raise
                # This is a single authorized fallback for a proven anonymous
                # API quota error, not a general retry or permission workaround.
                evidence = str(error.reason).encode("utf-8") + error.read(16384)
                if b"rate limit exceeded" not in evidence.lower():
                    raise
                print("[api-auth-fallback] GET " + public_location(url)
                      + " reason=anonymous-rate-limit", flush=True)
                return json.loads(read_url(url, headers=dict(headers, Authorization="Bearer " + self.gitee_token),
                                           attempts=1))
        # Never automatically repeat an uncertain POST. A subsequent invocation
        # re-reads assets/releases and checks immutable contents before resuming.
        try:
            with http_opener().open(urllib.request.Request(url, data=data, headers=headers, method=method),
                                    timeout=900 if file else 90) as response:
                body = response.read(MAX_JSON + 1)
                require(len(body) <= MAX_JSON, "API response too large")
                return json.loads(body) if body else None
        except urllib.error.HTTPError as error:
            details = safe_error_fields(error.read(16384), (self.github_token, self.gitee_token))
            print("[api-error] " + json.dumps({"status": error.code, "location": public_location(error.url),
                                             "details": details}), flush=True)
            raise

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

    def ensure_release(self, host, tag=RESOURCE_TAG, create=True):
        require(tag in {RESOURCE_TAG, "build-tools"}, "Unexpected resource tag")
        print(f"[stage] Check {host} manager latest before resource release {tag}", flush=True)
        previous_latest = self.api(host, "/releases/latest")
        require(isinstance(previous_latest, dict) and re.fullmatch(r"v\d{1,4}\.\d{1,4}\.\d{1,4}", previous_latest.get("tag_name", "")), "Legacy latest endpoint is not a manager release; stop migration")
        branch = self.ensure_gitee_resource_repository()["default_branch"] if host == "gitee" else "main"
        current = self.release(host, tag)
        if current is None:
            require(create, "Expected existing resource release was not returned by the API; creation is disabled")
            print(f"[stage] Create {host} resource release {tag} at branch {branch}", flush=True)
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
        require(len(actual) == len(data) and digest(actual) == digest(data),
                f"Remote same-name asset differs; refusing overwrite: expected_bytes={len(data)}, "
                f"actual_bytes={len(actual)}, expected_sha256={digest(data)}, actual_sha256={digest(actual)}")

    def ensure_asset(self, host, release, path, snapshot=None):
        print(f"[stage] Check {host} attachment {path.name}", flush=True)
        # One immutable inventory per publication avoids a pagination API call
        # for every existing ZIP. It is never shared across publication runs.
        current = self.assets(host, release) if snapshot is None else snapshot
        data = path.read_bytes()
        if path.name not in current:
            # Re-check immediately before any POST: an uncertain prior upload
            # or an external publisher may have added the immutable name.
            current = self.assets(host, release) if snapshot is not None else current
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


def known_archive_identities(root):
    """Keep filenames immutable even if an attachment was replaced manually."""
    paths = list((root / "cloud/indexes").glob("*.json"))
    require(len(paths) <= 512, "Too many historical indexes to validate")
    baseline = root / "rust/cloud-catalog.json"
    if baseline.exists():
        paths.append(baseline)
    identities = {}
    for path in paths:
        require(path.stat().st_size <= MAX_JSON, "Historical index exceeds size limit")
        document = json.loads(path.read_text(encoding="utf-8-sig"))
        packages = document.get("packages")
        require(isinstance(packages, list) and len(packages) <= 256, "Historical index packages are invalid")
        for package in packages:
            name = filename(package["archive"])
            identity = (package["bytes"], package["sha256"])
            require(type(identity[0]) is int and 0 < identity[0] <= MAX_ZIP and
                    re.fullmatch(r"[0-9a-f]{64}", identity[1]), "Historical package identity is invalid")
            require(name not in identities or identities[name] == identity,
                    "Historical indexes conflict for immutable archive: " + name)
            identities[name] = identity
    return identities


def verify_known_archives(archives, known):
    for name, data in archives.items():
        require(name not in known or known[name] == (len(data), digest(data)),
                "Existing archive content changed; upload a new filename: " + name)


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


def verify_published_metadata(url, expected):
    """Only call after a successful push of this exact immutable index/catalog.

    Newly pushed Gitee raw files can briefly retain a negative-cache 404. Wait
    for visibility within a small fixed budget, never accept different bytes.
    Existing-catalog admission checks deliberately do not use this helper.
    """
    require(url.startswith((GT_RAW + "cloud/", GH_RAW + "cloud/")) and
            urllib.parse.urlsplit(url).path.endswith(".json"), "Unexpected metadata readback URL")
    delays = (3, 8, 15, 30)
    for attempt in range(len(delays) + 1):
        try:
            actual = read_url(url, MAX_JSON, attempts=1, timeout=20)
        except urllib.error.HTTPError as error:
            if error.code not in (404, 429, 500, 502, 503, 504) or attempt == len(delays):
                raise
            reason = safe_exception(error)
        except (TimeoutError, urllib.error.URLError, TransientResponse) as error:
            if attempt == len(delays):
                raise
            reason = safe_exception(error)
        else:
            require(actual == expected, f"Catalog/index mirror readback mismatch: expected_bytes={len(expected)}, "
                    f"actual_bytes={len(actual)}, expected_sha256={digest(expected)}, actual_sha256={digest(actual)}")
            print(f"[metadata-verified] {public_location(url)} bytes={len(actual)} sha256={digest(actual)}", flush=True)
            return
        print(f"[metadata-wait] {public_location(url)}: {reason}; "
              f"retry {attempt + 2}/5 after {delays[attempt]}s", flush=True)
        time.sleep(delays[attempt])


class GitFailure(RuntimeError):
    """A fixed diagnostic category, never subprocess output or command arguments."""
    def __init__(self, reason, transient=False):
        self.reason, self.transient = reason, transient
        super().__init__("Git operation failed: " + reason)


def git_failure(stderr):
    """Classify stderr in memory; never expose credentials, URLs or raw text."""
    text = stderr.lower()
    permanent = (
        ("authentication", ("authentication failed", "access denied", "permission denied",
                            "invalid username or password", "could not read username",
                            "returned error: 401", "returned error: 403", "http 401", "http 403",
                            "write access to repository not granted")),
        ("remote-rejected", ("non-fast-forward", "fetch first", "[rejected]", "[remote rejected]",
                             "pre-receive hook declined", "protected branch")),
        ("repository-access", ("repository not found", "does not appear to be a git repository")),
        ("tls-certificate", ("certificate verify failed", "certificate verification failed",
                             "ssl certificate problem", "self-signed certificate",
                             "error in the certificate", "certificate has expired")),
    )
    for reason, patterns in permanent:
        if any(pattern in text for pattern in patterns):
            return GitFailure(reason)
    transient = (
        ("dns", ("could not resolve host", "could not resolve proxy", "name or service not known",
                 "temporary failure in name resolution")),
        ("connection-timeout", ("connection timed out", "operation timed out", "connection timeout",
                                "failed to connect", "couldn't connect to server")),
        ("connection-reset", ("connection reset", "remote end hung up unexpectedly", "early eof",
                              "empty reply from server", "broken pipe", "http/2 stream",
                              "http2 framing layer")),
        ("tls-transport", ("gnutls_handshake() failed", "gnutls_recv error", "ssl_error_syscall",
                           "ssl connect error", "tls connection was non-properly terminated")),
    )
    for reason, patterns in transient:
        if any(pattern in text for pattern in patterns):
            return GitFailure(reason, transient=True)
    return GitFailure("unknown")


def git_result(root, *args, env=None):
    try:
        return subprocess.run(["git", "-C", str(root), *args], env=env, capture_output=True,
                              text=True, errors="replace", timeout=120)
    except subprocess.TimeoutExpired:
        raise GitFailure("operation-timeout", transient=True) from None
    except OSError:
        raise GitFailure("process-unavailable") from None


def git(root, *args, env=None):
    result = git_result(root, *args, env=env)
    if result.returncode:
        raise git_failure(result.stderr) from None
    return result.stdout.strip()


def remote_head(root, url, options, env):
    refs = git(root, *options, "ls-remote", "--refs", url, "refs/heads/main", env=env)
    if not refs:
        return None
    fields = refs.split()
    if len(fields) != 2 or fields[1] != "refs/heads/main" or not re.fullmatch(r"[0-9a-f]{40}", fields[0]):
        raise GitFailure("invalid-remote-ref")
    return fields[0]


def make_askpass(folder):
    helper = Path(folder) / "askpass.py"
    helper.write_text('#!/usr/bin/env python3\nimport os,sys\nprint(os.environ["RTXFG_PUSH_USER"] if "username" in sys.argv[1].lower() else os.environ["RTXFG_PUSH_TOKEN"])\n', encoding="utf-8")
    if os.name == "nt":
        # Git for Windows must use this process's Python, not a python3 app alias.
        # The files contain only program paths and environment variable names.
        executable = str(Path(sys.executable).resolve())
        require(not any(char in executable for char in ('"', '\r', '\n')), "Unsafe Python executable path")
        wrapper = Path(folder) / "askpass.cmd"
        wrapper.write_text('@echo off\n"' + executable.replace('%', '%%')
                           + '" "%~dp0askpass.py" %*\n', encoding="utf-8")
        return wrapper
    helper.chmod(0o700)
    return helper


def push(root, host, publisher):
    require(host in ("github", "gitee"), "Unexpected Git publication host")
    target = git(root, "rev-parse", "HEAD^{commit}")
    require(re.fullmatch(r"[0-9a-f]{40}", target), "Invalid local publication commit")
    with tempfile.TemporaryDirectory(prefix="rtxfg-git-") as folder:
        helper = make_askpass(folder)
        env = dict(os.environ, GIT_ASKPASS=str(helper), GIT_TERMINAL_PROMPT="0",
                   RTXFG_PUSH_USER="x-access-token" if host == "github" else REPO.split("/")[0],
                   RTXFG_PUSH_TOKEN=publisher.github_token if host == "github" else publisher.gitee_token)
        # HTTP/1.1 avoids an additional HTTP/2 reset path; it is a transport
        # precaution, not a claim about the cause of any earlier failed push.
        options = ("-c", "credential.helper=", "-c", "http.version=HTTP/1.1",
                   "-c", "http.lowSpeedLimit=1", "-c", "http.lowSpeedTime=60")
        url = f"https://{host}.com/{REPO}.git"
        for attempt in range(3):
            try:
                remote = remote_head(root, url, options, env)
                if remote == target:
                    print(f"[git-verified] {host} main already matches publication commit", flush=True)
                    return
                if remote:
                    exists = git_result(root, "cat-file", "-e", remote + "^{commit}")
                    if exists.returncode:
                        # Read unknown remote history without changing local
                        # branches. Recheck the fetched tip if it moved meanwhile.
                        git(root, *options, "fetch", "--no-tags", url, "refs/heads/main", env=env)
                        remote = git(root, "rev-parse", "FETCH_HEAD^{commit}")
                    relation = git_result(root, "merge-base", "--is-ancestor", remote, target)
                    if relation.returncode == 1:
                        raise GitFailure("non-fast-forward")
                    if relation.returncode:
                        raise git_failure(relation.stderr)
                # Pin the originally reviewed target; normal push still rejects
                # a concurrent divergent remote advance. Never force or reset.
                git(root, *options, "push", url, target + ":refs/heads/main", env=env)
                if remote_head(root, url, options, env) != target:
                    raise GitFailure("remote-head-changed")
                print(f"[git-verified] {host} main matches publication commit", flush=True)
                return
            except GitFailure as error:
                print(f"[git-transfer] {host} attempt={attempt + 1}/3 reason={error.reason}", flush=True)
                if error.transient and attempt == 2:
                    # The final push may have reached the server before its
                    # response was lost. One bounded read reconciles that case.
                    try:
                        if remote_head(root, url, options, env) == target:
                            print(f"[git-verified] {host} final uncertain response reconciled", flush=True)
                            return
                    except GitFailure:
                        pass
                if not error.transient or attempt == 2:
                    raise error from None
                delay = (5, 15)[attempt]
                print(f"[git-retry] {host} retry={attempt + 2}/3 after={delay}s", flush=True)
                time.sleep(delay)


def verify_asset_jobs(paths, verify, after_verified, workers=4):
    """Bound concurrent *different* archives and promote only after all succeed.

    An uncertain POST is never automatically retried. On error stop scheduling
    new work, finish the bounded in-flight requests and leave metadata alone.
    Re-running discovers already uploaded assets and checks their exact bytes.
    """
    require(isinstance(workers, int) and 1 <= workers <= 4, "Asset workers must be between 1 and 4")
    paths = list(paths)
    require(paths and len({path.name for path in paths}) == len(paths), "Asset jobs require unique filenames")
    remaining = iter(paths)
    failures, complete = [], 0
    with ThreadPoolExecutor(max_workers=workers, thread_name_prefix="rtxfg-asset") as pool:
        pending = {}
        def schedule():
            path = next(remaining, None)
            if path is not None:
                pending[pool.submit(verify, path)] = (path, time.monotonic())
        for _ in range(min(workers, len(paths))):
            schedule()
        while pending:
            done, _ = wait(pending, timeout=20, return_when=FIRST_COMPLETED)
            if not done:
                active = ", ".join(f"{path.name} ({int(time.monotonic() - started)}s)"
                                   for path, started in pending.values())
                print(f"[progress] Verified {complete}/{len(paths)} ZIPs; waiting: {active}", flush=True)
                continue
            for future in done:
                path, started = pending.pop(future)
                try:
                    future.result()
                    complete += 1
                    print(f"[progress] Verified {complete}/{len(paths)} ZIPs: {path.name} ({time.monotonic() - started:.1f}s)", flush=True)
                except Exception as error:
                    # Exception messages from HTTP clients may contain signed
                    # URLs; log only safe status/location or exception type.
                    reason = safe_exception(error)
                    failures.append(path.name)
                    print(f"[asset-error] {path.name}: {reason}; metadata will not be promoted", flush=True)
            if not failures:
                for _ in done:
                    schedule()
    require(not failures, "ZIP verification failed; rerun to resume immutable assets: " + ", ".join(failures))
    require(complete == len(paths), "Incomplete asset verification; metadata unchanged")
    return after_verified()


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
    asset_snapshots = {"github": github_assets, "gitee": publisher.assets("gitee", releases["gitee"])}
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
    verify_known_archives(archives, known_archive_identities(root))
    documents = build_documents(spec, archives)
    with tempfile.TemporaryDirectory(prefix="rtxfg-payloads-") as temp:
        paths = []
        for name, data in archives.items():
            path = Path(temp) / name
            path.write_bytes(data)
            paths.append(path)
        def verify(path):
            for host in ("github", "gitee"):
                publisher.ensure_asset(host, releases[host], path, asset_snapshots[host])
        verify_asset_jobs(paths, verify, lambda: publish_metadata(root, publisher, documents), args.workers)


def publish_metadata(root, publisher, documents):
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
            verify_published_metadata(base + relative, data)
    promote_documents(documents, write, promote)
    print("Cloud publication completed: " + json.loads(catalog)["revision"], flush=True)


def validate_probe_zip(name, data=None):
    require(isinstance(name, str) and name.endswith(".zip"), "Actions probes accept DLL ZIPs only; upload EXEs from the publisher machine")
    filename(name)
    if data is not None:
        require(0 < len(data) <= MAX_ZIP, "Invalid probe ZIP size")
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            entries = archive.infolist()
            names = {entry.filename for entry in entries}
            require(len(entries) == 2 and len(names) == 2 and "dlssg_sm86.ini" in names and
                    len(names & PROXIES) == 1, "Probe ZIP must contain only one proxy DLL and its INI")
            require(all(not entry.is_dir() and not entry.flag_bits & 1 and 0 < entry.file_size <= MAX_ZIP
                        and (entry.external_attr >> 16) & 0o170000 != 0o120000 for entry in entries),
                    "Invalid probe ZIP entry")


def probe(args):
    """Copy one immutable DLL ZIP to Gitee and verify its anonymous download.
    EXEs are uploaded locally, never through this Actions probe. No file is
    installed or executed and the active catalog remains unchanged."""
    require(args.source_tag and args.asset and args.expected_sha256, "Probe requires source tag, asset and expected SHA-256")
    validate_probe_zip(args.asset)
    require(args.destination_tag == RESOURCE_TAG, "DLL ZIP probes must use the payloads resource release")
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
    validate_probe_zip(args.asset, data)
    target = publisher.ensure_release("gitee", args.destination_tag, create=not args.existing_release_only)
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


def diagnose_resource_access(args):
    """Read-only contrast of token identity, repo visibility and target ref."""
    publisher = Publisher()
    report = {"read_only": True, "requests": []}
    targets = [("identity", "https://gitee.com/api/v5/user", True)]
    for name, base in (("manager", GT), ("resources", GT_RESOURCES)):
        targets.extend([(name + "_anonymous", base, False), (name + "_authenticated", base, True)])
    targets.extend([("resource_master_anonymous", GT_RESOURCES + "/branches/master", False),
                    ("resource_master_authenticated", GT_RESOURCES + "/branches/master", True)])
    for name, url, authenticated in targets:
        entry = {"name": name, "location": public_location(url), "authenticated": authenticated}
        headers = {"Authorization": "Bearer " + publisher.gitee_token} if authenticated else None
        try:
            value = json.loads(read_url(url, headers=headers))
            entry["response_type"] = type(value).__name__
            if isinstance(value, dict):
                for key in ("id", "login", "full_name", "private", "default_branch", "permissions", "name"):
                    if key in value:
                        entry[key] = value[key]
                if isinstance(value.get("commit"), dict):
                    entry["commit_sha"] = value["commit"].get("sha")
        except urllib.error.HTTPError as error:
            entry["status"] = error.code
            entry["details"] = safe_error_fields(error.read(16384), (publisher.github_token, publisher.gitee_token))
        report["requests"].append(entry)
        print("[diagnostic] " + json.dumps(entry, ensure_ascii=True), flush=True)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    (out / "resource-access.probe.json").write_bytes(canonical(report))


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

    def test_rtxmfg_requires_separate_json_and_canonical_universal_entry(self):
        spec, archives = self.fixture()
        scheme = spec["schemes"][0]
        scheme.update(profile="rtxmfg_universal_133", min_manager_version="4.2.5", defaults={"rtx_mode":"dynamic", "rtx_target":"200"})
        dll = zipfile.ZipFile(io.BytesIO(next(iter(archives.values())))).read("version.dll")
        def pack(config, name="version.dll"):
            stream = io.BytesIO()
            with zipfile.ZipFile(stream,"w") as z:
                z.writestr(name,dll)
                z.writestr("RTXMFG-Universal.json",json.dumps(config))
            return stream.getvalue()
        name = scheme["archives"][0]
        valid = {"mode":"follow", "followGame":True, "multiplier":2, "dynamicTargetFrameRate":0, "otherMenuSetting":119}
        p = package_from_zip(scheme,name,pack(valid))
        self.assertEqual(p["backends"],["rtx40mfg"])
        self.assertEqual({f["name"] for f in p["files"]},{"version.dll","RTXMFG-Universal.json"})
        with self.assertRaises(RuntimeError): package_from_zip(scheme,name,next(iter(archives.values())))
        with self.assertRaises(RuntimeError): package_from_zip(scheme,name,pack(valid,"dxgi.dll"))
        for invalid in [[], {"multiplier":7}, {"multiplier":2,"followGame":1}, {"multiplier":2,"mode":"follow"}, {"multiplier":2,"dynamicTargetFrameRate":1001}]:
            with self.assertRaises(RuntimeError): package_from_zip(scheme,name,pack(invalid))

    def test_reject_missing_or_ambiguous_archive(self):
        spec, archives = self.fixture()
        with self.assertRaises(RuntimeError): build_documents(spec, {})
        spec["schemes"][0]["archives"].append("test-r1-version.zip")
        with self.assertRaises(RuntimeError): build_documents(spec, archives)

    def test_transfusion_jsonc_and_exact_proxy_routes(self):
        spec, archives = self.fixture()
        scheme = spec["schemes"][0]
        scheme.update(profile="transfusion_json_v3", version="1.4.5", min_manager_version="4.2.6", defaults={"tf_mode":"game", "tf_target":"237"})
        dll = zipfile.ZipFile(io.BytesIO(next(iter(archives.values())))).read("version.dll")
        config = {"configVersion":3,"general":{"gpuArchitecture":"auto"},"frameGeneration":{"mode":"game","multiplier":4,"dynamicTargetFrameRate":0,"dynamicExperimental56":False},"compatibility":{"smoothMotionSm86":False}}
        def pack(name, data):
            stream = io.BytesIO()
            with zipfile.ZipFile(stream, "w") as z:
                z.writestr(name, dll)
                z.writestr("DLSSG-Transfusion.json", "// 中文 https://example.test/\n" + json.dumps(data))
            return stream.getvalue()
        validate_spec(spec)
        for proxy in ("version.dll", "dinput8.dll", "dxgi.dll", "winmm.dll"):
            p = package_from_zip(scheme, scheme["archives"][0], pack(proxy, config))
            self.assertEqual(p["proxy"], proxy)
            self.assertEqual(p["backends"], ["transfusion"])
        with self.assertRaises(RuntimeError): package_from_zip(scheme, scheme["archives"][0], pack("d3d12.dll", config))
        config["compatibility"]["smoothMotionSm86"] = True
        with self.assertRaises(RuntimeError): package_from_zip(scheme, scheme["archives"][0], pack("version.dll", config))
        self.assertEqual(jsonc_loads('{"url":"https://example.test/","quote":"a\\\"//b"}')["url"], "https://example.test/")
        with self.assertRaises(RuntimeError): jsonc_loads('{"mode":"game","mode":"fixed"}')

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
        official_url("https://raw.giteeusercontent.com/pandaligx/RTX-FG-Manager/raw/main/cloud/catalog.json")
        with self.assertRaises(RuntimeError):
            official_url("https://raw.giteeusercontent.com.evil.invalid/catalog.json")

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
            def public_gitee_resource_repository(self): return None
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
            def public_gitee_resource_repository(self): return self.api("gitee", GT_RESOURCES)
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

    def test_seed_cannot_reaccept_replaced_same_name_attachment(self):
        verify_known_archives({"existing.zip": b"original"}, {"existing.zip": (8, digest(b"original"))})
        with self.assertRaisesRegex(RuntimeError, "new filename"):
            verify_known_archives({"existing.zip": b"modified"}, {"existing.zip": (8, digest(b"original"))})
        verify_known_archives({"new-r2.zip": b"new signed bytes"}, {"existing.zip": (8, digest(b"original"))})

    def test_api_error_output_is_narrow_and_redacted(self):
        safe = safe_error_fields(b'{"message":"fake-secret https://gitee.com/api?token=fake-secret","access_token":"fake-secret","headers":{"private":"omit"}}', ("fake-secret",))
        self.assertEqual(set(safe), {"message"})
        text = json.dumps(safe)
        self.assertNotIn("fake-secret", text)
        self.assertNotIn("?token", text)
        self.assertNotIn("headers", text)

    def test_gitee_inventory_rate_limit_has_one_authorized_fallback(self):
        publisher = Publisher.__new__(Publisher)
        publisher.github_token = publisher.gitee_token = "fixture"
        for url in (GT + "/releases/latest", GT_RESOURCES):
            error = urllib.error.HTTPError(url, 403, "Forbidden", {}, io.BytesIO(b"Forbidden (Rate Limit Exceeded)"))
            with patch(__name__ + ".read_url", side_effect=[error, b'{"id":1}']) as read:
                self.assertEqual(publisher.api("gitee", url), {"id": 1})
                self.assertNotIn("Authorization", read.call_args_list[0].kwargs["headers"])
                self.assertEqual(read.call_args_list[1].kwargs["headers"]["Authorization"], "Bearer fixture")
                self.assertEqual(read.call_args_list[1].kwargs["attempts"], 1)
                self.assertEqual(read.call_count, 2)
        with patch(__name__ + ".read_url", return_value=b'{"id":1}') as read:
            self.assertEqual(publisher.public_gitee_resource_repository(), {"id": 1})
            self.assertEqual(read.call_count, 1)
            self.assertNotIn("Authorization", read.call_args.kwargs["headers"])

    def test_gitee_fallback_never_expands_to_permissions_other_hosts_or_payloads(self):
        publisher = Publisher.__new__(Publisher)
        publisher.github_token = publisher.gitee_token = "fixture"
        cases = [(GT + "/releases/latest", "Forbidden", b"permission denied"),
                 (GT_RAW + "cloud/catalog.json", "Rate Limit Exceeded", b""),
                 (GT + "/../../elsewhere", "Rate Limit Exceeded", b""),
                 (GH + "/releases/latest", "Rate Limit Exceeded", b"")]
        for url, reason, body in cases:
            error = urllib.error.HTTPError(url, 403, reason, {}, io.BytesIO(body))
            with patch(__name__ + ".read_url", side_effect=error) as read:
                with self.assertRaises(urllib.error.HTTPError): publisher.api("gitee", url)
                self.assertEqual(read.call_count, 1)
                self.assertNotIn("Authorization", read.call_args.kwargs["headers"])
        url = GT + "/releases/latest"
        errors = [urllib.error.HTTPError(url, 403, "Rate Limit Exceeded", {}, io.BytesIO()),
                  urllib.error.HTTPError(url, 403, "Forbidden", {}, io.BytesIO())]
        with patch(__name__ + ".read_url", side_effect=errors) as read:
            with self.assertRaises(urllib.error.HTTPError): publisher.api("gitee", url)
            self.assertEqual(read.call_count, 2)

    def test_probe_rejects_exe_and_non_payload_archives(self):
        with self.assertRaisesRegex(RuntimeError, "DLL ZIPs only"):
            validate_probe_zip("aria2c-1.37.0-example.exe")
        _, archives = self.fixture()
        for name, data in archives.items():
            validate_probe_zip(name, data)
        out = io.BytesIO()
        with zipfile.ZipFile(out, "w") as archive:
            archive.writestr("tool.exe", b"MZ")
            archive.writestr("dlssg_sm86.ini", b"[Logging]\nLevel=1\n")
        with self.assertRaisesRegex(RuntimeError, "only one proxy DLL"):
            validate_probe_zip("disguised-tool.zip", out.getvalue())

    def test_concurrent_assets_are_bounded_and_all_precede_promotion(self):
        paths = [Path(f"test-{index}.zip") for index in range(8)]
        lock, barrier = threading.Lock(), threading.Barrier(4)
        active, maximum, verified, promoted = 0, 0, [], []
        def verify(path):
            nonlocal active, maximum
            with lock:
                active += 1
                maximum = max(maximum, active)
            barrier.wait(timeout=5)
            with lock:
                active -= 1
                verified.append(path.name)
        def promote():
            self.assertEqual(len(verified), len(paths))
            self.assertEqual(active, 0)
            promoted.append(True)
        verify_asset_jobs(paths, verify, promote)
        self.assertEqual(maximum, 4)
        self.assertEqual(sorted(verified), sorted(path.name for path in paths))
        self.assertEqual(promoted, [True])

    def test_concurrent_failure_never_promotes_and_can_resume(self):
        paths = [Path(f"test-{index}.zip") for index in range(4)]
        barrier, lock, verified, promoted = threading.Barrier(4), threading.Lock(), set(), []
        def verify(path):
            barrier.wait(timeout=5)
            if path == paths[1]:
                raise RuntimeError("uncertain upload result")
            with lock:
                verified.add(path.name)
        with self.assertRaisesRegex(RuntimeError, "rerun to resume"):
            verify_asset_jobs(paths, verify, lambda: promoted.append(True))
        self.assertEqual(promoted, [])
        self.assertEqual(len(verified), 3)
        # A rerun checks every immutable asset, including the uncertain upload;
        # only the successful run reaches the metadata callback.
        verify_asset_jobs(paths, lambda path: None, lambda: promoted.append(True))
        self.assertEqual(promoted, [True])

    def test_concurrent_duplicate_or_unbounded_work_is_rejected(self):
        calls = []
        for paths, workers in (([Path("same.zip"), Path("same.zip")], 4),
                               ([Path("test.zip")], 5)):
            with self.assertRaises(RuntimeError):
                verify_asset_jobs(paths, lambda path: calls.append(path), lambda: calls.append("promote"), workers)
        self.assertEqual(calls, [])

    def test_asset_snapshot_avoids_repeated_listing_and_never_skips_verification(self):
        class Fake(Publisher):
            def __init__(self): self.list_calls, self.verified = 0, []
            def assets(self, host, release):
                self.list_calls += 1
                raise AssertionError("Existing snapshot must not query the API again")
            def verify_asset(self, asset, data): self.verified.append((asset["name"], data))
        with tempfile.TemporaryDirectory() as folder:
            paths = [Path(folder) / f"test-{index}.zip" for index in range(3)]
            for path in paths:
                path.write_bytes(b"fixture")
            snapshot = {path.name: {"name": path.name} for path in paths}
            publisher = Fake()
            for path in paths:
                publisher.ensure_asset("gitee", {"id": 1}, path, snapshot)
            self.assertEqual(publisher.list_calls, 0)
            self.assertEqual(len(publisher.verified), 3)
            self.assertEqual(len(snapshot), 3)

    def test_snapshot_missing_name_is_rechecked_before_post(self):
        class Fake(Publisher):
            def __init__(self): self.list_calls, self.verified = 0, False
            def assets(self, host, release):
                self.list_calls += 1
                return {"test.zip": {"name": "test.zip"}}
            def verify_asset(self, asset, data): self.verified = True
            def api(self, *args, **kwargs): raise AssertionError("Do not upload an existing same-name asset")
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "test.zip"
            path.write_bytes(b"fixture")
            publisher = Fake()
            publisher.ensure_asset("gitee", {"id": 1}, path, {})
            self.assertEqual(publisher.list_calls, 1)
            self.assertTrue(publisher.verified)

    def test_real_asset_mismatch_is_not_retried_or_overwritten(self):
        _, archives = self.fixture()
        expected = next(iter(archives.values()))
        different = bytearray(expected)
        different[-1] ^= 1
        with patch(__name__ + ".read_url", return_value=bytes(different)) as read:
            with self.assertRaisesRegex(RuntimeError, "refusing overwrite.*expected_sha256=.*actual_sha256="):
                Publisher.verify_asset({"url": "https://gitee.com/test.zip"}, expected)
            self.assertEqual(read.call_count, 1)

    def test_html_challenge_is_bounded_and_never_accepted_as_zip(self):
        class Response:
            url = "https://gitee.com/test.zip"
            def __init__(self, data, content_type):
                self.data, self.headers = data, {"Content-Type": content_type}
            def __enter__(self): return self
            def __exit__(self, *args): pass
            def read(self, limit): return self.data[:limit]
        class Opener:
            def __init__(self, responses): self.responses, self.calls = iter(responses), 0
            def open(self, *args, **kwargs):
                self.calls += 1
                return next(self.responses)
        _, archives = self.fixture()
        data = next(iter(archives.values()))
        html = lambda: Response(b"<!DOCTYPE html><html>challenge</html>", "text/html")
        opener = Opener([html(), Response(data, "application/zip")])
        with patch(__name__ + ".http_opener", return_value=opener), patch(__name__ + ".time.sleep") as sleep:
            self.assertEqual(read_url("https://gitee.com/test.zip", len(data)), data)
            sleep.assert_called_once_with(5)
        opener = Opener([html(), html(), html()])
        with patch(__name__ + ".http_opener", return_value=opener), patch(__name__ + ".time.sleep"):
            with self.assertRaisesRegex(TransientResponse, "Received HTML instead of ZIP"):
                read_url("https://gitee.com/test.zip", len(data))
            self.assertEqual(opener.calls, 3)

    def test_runtime_error_diagnostic_redacts_tokens_and_queries(self):
        with patch.dict(os.environ, {"GITEE_TOKEN": "test-token"}):
            result = safe_exception(RuntimeError("test-token https://gitee.com/path?signature=private"))
        self.assertNotIn("test-token", result)
        self.assertNotIn("signature=", result)
        self.assertIn("gitee.com/path", result)

    def test_disconnected_reads_retry_three_times_without_weakening_checks(self):
        class Response:
            url = "https://gitee.com/test.zip"
            headers = {"Content-Type": "application/zip"}
            def __init__(self, value): self.value = value
            def __enter__(self): return self
            def __exit__(self, *args): pass
            def read(self, limit):
                if isinstance(self.value, Exception): raise self.value
                return self.value[:limit]
        for error in (http.client.RemoteDisconnected("private"), http.client.IncompleteRead(b"partial"),
                      ConnectionResetError("private"), ConnectionAbortedError("private")):
            with self.subTest(error=type(error).__name__), \
                    patch(__name__ + ".http_opener") as opener, patch(__name__ + ".time.sleep") as sleep:
                opener.return_value.open.side_effect = [Response(error), Response(b"data")]
                self.assertEqual(read_url(Response.url, 4), b"data")
                sleep.assert_called_once_with(5)
                opener.return_value.open.side_effect = [Response(error)] * 3
                with self.assertRaises(type(error)): read_url(Response.url, 4)
                self.assertEqual(opener.return_value.open.call_count, 5)
        with patch(__name__ + ".http_opener") as opener, patch(__name__ + ".time.sleep"):
            opener.return_value.open.side_effect = [Response(http.client.RemoteDisconnected()), Response(b"wrong")]
            with self.assertRaisesRegex(RuntimeError, "refusing overwrite"):
                Publisher.verify_asset({"url": Response.url}, b"right")
            self.assertEqual(opener.return_value.open.call_count, 2)
            opener.return_value.open.return_value = Response(b"oversized")
            opener.return_value.open.side_effect = None
            with self.assertRaisesRegex(RuntimeError, "exceeds size limit"): read_url(Response.url, 4)
            opener.return_value.open.return_value.url = "http://gitee.com/test.zip"
            with self.assertRaisesRegex(RuntimeError, "Unsafe URL"): read_url(Response.url, 20)

    def test_git_failure_categories_never_include_raw_stderr(self):
        for stderr, reason, transient in (
            ("fatal: Authentication failed secret-token", "authentication", False),
            ("[rejected] main (non-fast-forward)", "remote-rejected", False),
            ("SSL certificate problem: secret-token", "tls-certificate", False),
            ("gnutls_handshake() failed: TLS connection was non-properly terminated", "tls-transport", True),
            ("Could not resolve host: gitee.com", "dns", True),
            ("Connection reset by peer secret-token", "connection-reset", True),
            ("unrecognized https://gitee.com/x?access_token=secret-token", "unknown", False),
        ):
            error = git_failure(stderr)
            self.assertEqual((error.reason, error.transient), (reason, transient))
            self.assertNotIn("secret-token", str(error))
            self.assertNotIn("https://", str(error))

    def test_git_process_timeout_and_unknown_error_are_sanitized(self):
        with patch(__name__ + ".subprocess.run", side_effect=subprocess.TimeoutExpired(["private"], 120)):
            with self.assertRaisesRegex(GitFailure, "operation-timeout") as caught:
                git(".", "push", "private")
            self.assertTrue(caught.exception.transient)
        result = subprocess.CompletedProcess([], 1, "", "unknown secret-token https://private.invalid")
        with patch(__name__ + ".subprocess.run", return_value=result) as run:
            with self.assertRaisesRegex(GitFailure, "unknown") as caught: git(".", "push")
            self.assertNotIn("secret-token", str(caught.exception))
            self.assertEqual(run.call_args.kwargs["timeout"], 120)

    @unittest.skipUnless(os.name == "nt", "Windows Git askpass integration")
    def test_windows_askpass_uses_current_python_without_storing_credentials(self):
        with tempfile.TemporaryDirectory(prefix="rtxfg askpass ") as folder:
            helper = make_askpass(folder)
            self.assertEqual(helper.suffix, ".cmd")
            env = dict(os.environ, GIT_ASKPASS=str(helper), GIT_TERMINAL_PROMPT="0",
                       RTXFG_PUSH_USER="fixture-user", RTXFG_PUSH_TOKEN="fixture-pass")
            # credential fill only invokes the local helper; it makes no HTTP request.
            result = subprocess.run(["git", "-c", "credential.helper=", "credential", "fill"],
                                    input="protocol=https\nhost=example.invalid\n\n", env=env,
                                    capture_output=True, text=True, timeout=30)
            self.assertEqual(result.returncode, 0, "Local Windows askpass failed")
            fields = dict(line.split("=", 1) for line in result.stdout.splitlines() if "=" in line)
            self.assertEqual(fields.get("username"), "fixture-user")
            self.assertEqual(fields.get("password"), "fixture-pass")
            for path in Path(folder).iterdir():
                text = path.read_text("utf-8")
                self.assertNotIn("fixture-user", text)
                self.assertNotIn("fixture-pass", text)

    def test_git_push_retries_only_transient_failures_and_pins_target(self):
        class PublisherFixture:
            github_token = gitee_token = "fixture"
        target, previous = "a" * 40, "b" * 40
        for reason, transient, attempts in (("tls-transport", True, 3), ("authentication", False, 1),
                                            ("remote-rejected", False, 1), ("unknown", False, 1)):
            calls = []
            def fake_git(root, *args, **kwargs):
                if args == ("rev-parse", "HEAD^{commit}"): return target
                if "push" in args:
                    calls.append(args)
                    raise GitFailure(reason, transient)
                raise AssertionError("Unexpected Git operation")
            with patch(__name__ + ".git", side_effect=fake_git), \
                    patch(__name__ + ".remote_head", return_value=previous), \
                    patch(__name__ + ".git_result", return_value=subprocess.CompletedProcess([], 0, "", "")), \
                    patch(__name__ + ".time.sleep") as sleep:
                with self.assertRaisesRegex(GitFailure, reason): push(Path("."), "gitee", PublisherFixture())
                self.assertEqual(len(calls), attempts)
                self.assertEqual(sleep.call_count, attempts - 1)
                self.assertTrue(all(args[-1] == target + ":refs/heads/main" and "--force" not in args for args in calls))

    def test_uncertain_push_success_and_divergence_are_reconciled(self):
        class PublisherFixture:
            github_token = gitee_token = "fixture"
        target, previous = "a" * 40, "b" * 40
        calls = []
        def fake_git(root, *args, **kwargs):
            if args == ("rev-parse", "HEAD^{commit}"): return target
            calls.append(args)
            if "push" in args: raise GitFailure("operation-timeout", True)
            raise AssertionError("Unexpected Git operation")
        with patch(__name__ + ".git", side_effect=fake_git), \
                patch(__name__ + ".remote_head", side_effect=[previous, target]), \
                patch(__name__ + ".git_result", return_value=subprocess.CompletedProcess([], 0, "", "")), \
                patch(__name__ + ".time.sleep"):
            push(Path("."), "gitee", PublisherFixture())
            self.assertEqual(len(calls), 1)
        calls.clear()
        with patch(__name__ + ".git", side_effect=fake_git), \
                patch(__name__ + ".remote_head", return_value=previous), \
                patch(__name__ + ".git_result", side_effect=[subprocess.CompletedProcess([], 0, "", ""),
                                                           subprocess.CompletedProcess([], 1, "", "")]), \
                patch(__name__ + ".time.sleep") as sleep:
            with self.assertRaisesRegex(GitFailure, "non-fast-forward"): push(Path("."), "gitee", PublisherFixture())
            self.assertFalse(calls)
            sleep.assert_not_called()

    def test_published_metadata_waits_for_negative_cache_but_keeps_hash_gate(self):
        url = GT_RAW + "cloud/catalog.json"
        missing = urllib.error.HTTPError(url, 404, "not visible yet", {}, None)
        with patch(__name__ + ".read_url", side_effect=[missing, b"expected"]) as read, \
                patch(__name__ + ".time.sleep") as sleep:
            verify_published_metadata(url, b"expected")
            self.assertEqual(read.call_count, 2)
            sleep.assert_called_once_with(3)
            self.assertEqual(read.call_args.kwargs, {"attempts": 1, "timeout": 20})
        with patch(__name__ + ".read_url", return_value=b"different") as read, \
                patch(__name__ + ".time.sleep") as sleep:
            with self.assertRaisesRegex(RuntimeError, "readback mismatch"):
                verify_published_metadata(url, b"expected")
            self.assertEqual(read.call_count, 1)
            sleep.assert_not_called()

    def test_published_metadata_wait_is_bounded_and_never_accepts_auth_failure(self):
        url = GT_RAW + "cloud/catalog.json"
        for code, calls in ((404, 5), (403, 1)):
            error = urllib.error.HTTPError(url, code, "fixture", {}, None)
            with patch(__name__ + ".read_url", side_effect=error) as read, \
                    patch(__name__ + ".time.sleep") as sleep:
                with self.assertRaises(urllib.error.HTTPError):
                    verify_published_metadata(url, b"expected")
                self.assertEqual(read.call_count, calls)
                self.assertEqual(sleep.call_count, calls - 1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("prepare", "publish", "probe", "cleanup-probe", "diagnose-resource-access", "self-test"))
    parser.add_argument("--root", default=".")
    parser.add_argument("--schemes", default="cloud/schemes.json")
    parser.add_argument("--archives", action="append", default=[])
    parser.add_argument("--workers", type=int, choices=range(1, 5), default=4,
                        help="Maximum concurrent distinct DLL ZIPs (1-4)")
    parser.add_argument("--out", default="cloud-candidate")
    parser.add_argument("--seed-release", help="One-time import from an existing GitHub resource release")
    parser.add_argument("--source-tag")
    parser.add_argument("--asset")
    parser.add_argument("--destination-tag", default=RESOURCE_TAG, choices=(RESOURCE_TAG, "build-tools"))
    parser.add_argument("--expected-sha256")
    parser.add_argument("--existing-release-only", action="store_true")
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
    if args.command == "diagnose-resource-access":
        diagnose_resource_access(args)
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
        print(f"Cloud publication stopped: {safe_exception(error)}", file=sys.stderr)
        raise SystemExit(1) from None
