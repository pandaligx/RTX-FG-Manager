//! Parse, edit and atomically rewrite Wine prefix registry text files.
//!
//! Wine stores the Windows registry as plain text: `<prefix>/system.reg` holds
//! `REGISTRY\Machine` (HKLM) and `<prefix>/user.reg` holds `REGISTRY\User`
//! (HKCU). Keys inside a file are relative to that root.
//!
//! The parser keeps every original line verbatim (`Vec<String>` is the source
//! of truth) and only rewrites the exact value line a caller changes. Therefore
//! `RegFile::open(f)` followed by `RegFile::save()` reproduces the original
//! bytes exactly, even for multi-megabyte files with thousands of sections.
//!
//! Only `anyhow` and `std` are used.

use anyhow::{Context, anyhow};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// A decoded registry value.
#[derive(Debug, Clone, PartialEq)]
pub enum RegValue {
    Sz(String),
    ExpandSz(String),
    Dword(u32),
    Binary(Vec<u8>),
    MultiSz(Vec<String>),
}

/// Which registry hive a prefix file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hive {
    Machine,
    User,
}

/// Path of a hive's text file inside a Wine prefix.
pub fn hive_path(prefix: &Path, hive: Hive) -> PathBuf {
    match hive {
        Hive::Machine => prefix.join("system.reg"),
        Hive::User => prefix.join("user.reg"),
    }
}

/// One value line inside a section.
#[derive(Debug, Clone)]
struct Slot {
    /// Original decoded name; empty for the default value.
    display_name: String,
    /// Index into `RegFile::lines`.
    line_idx: usize,
    value: RegValue,
}

/// One `[Key]` section.
#[derive(Debug, Clone)]
struct Section {
    /// Original decoded key, casing preserved.
    display_key: String,
    /// Index of the `[Key] ...` header line.
    header_idx: usize,
    /// Lowercased value name -> slot (`""` is the default value).
    values: BTreeMap<String, Slot>,
}

/// A parsed Wine registry text file.
pub struct RegFile {
    path: PathBuf,
    /// Raw lines, without the `\n` separators (a trailing `\r` is kept as-is).
    lines: Vec<String>,
    /// Whether the original file ended with a newline.
    ends_with_newline: bool,
    /// Permission bits captured at open time (masked to 0o7777).
    mode: u32,
    /// Lowercased decoded key -> section.
    sections: BTreeMap<String, Section>,
}

impl RegFile {
    /// Parse an existing registry file. Fails if the file does not exist or is
    /// not valid UTF-8.
    pub fn open(path: &Path) -> anyhow::Result<RegFile> {
        let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let text = String::from_utf8(bytes)
            .with_context(|| format!("{} is not valid UTF-8", path.display()))?;
        let mode = file_mode(path);
        Ok(Self::from_text(path.to_path_buf(), text, mode))
    }

    /// An in-memory file for `path` that does not exist on disk yet.
    fn empty(path: &Path) -> RegFile {
        RegFile {
            path: path.to_path_buf(),
            lines: Vec::new(),
            ends_with_newline: false,
            mode: if path.exists() {
                file_mode(path)
            } else {
                0o644
            },
            sections: BTreeMap::new(),
        }
    }

    fn from_text(path: PathBuf, text: String, mode: u32) -> RegFile {
        let ends_with_newline = text.ends_with('\n');
        let mut lines: Vec<String> = text.split('\n').map(|s| s.to_string()).collect();
        if ends_with_newline {
            lines.pop();
        }
        let mut rf = RegFile {
            path,
            lines,
            ends_with_newline,
            mode,
            sections: BTreeMap::new(),
        };
        rf.rebuild_index();
        rf
    }

    /// Case-insensitive lookup. `name` of `""` or `"@"` means the default value.
    pub fn get(&self, key: &str, name: &str) -> Option<RegValue> {
        let kl = key.to_ascii_lowercase();
        let nl = canonical_name(name).to_ascii_lowercase();
        self.sections
            .get(&kl)
            .and_then(|sec| sec.values.get(&nl))
            .map(|slot| slot.value.clone())
    }

    /// Set a value, creating the key path if needed. Only this value's line is
    /// rewritten.
    pub fn set(&mut self, key: &str, name: &str, value: RegValue) -> anyhow::Result<()> {
        let kl = key.to_ascii_lowercase();
        let nm = canonical_name(name);
        let nl = nm.to_ascii_lowercase();
        let new_line = encode_value_line(&nm, &value);

        if let Some(sec) = self.sections.get_mut(&kl)
            && let Some(slot) = sec.values.get_mut(&nl)
        {
            self.lines[slot.line_idx] = new_line;
            slot.value = value;
            slot.display_name = nm;
            return Ok(());
        }

        if let Some(sec) = self.sections.get(&kl) {
            let pos = self.value_insert_pos(sec.header_idx);
            self.lines.insert(pos, new_line);
        } else {
            let key_line = format!("[{}]", encode_key(key));
            let last_blank = self
                .lines
                .last()
                .map(|l| content(l).trim().is_empty())
                .unwrap_or(true);
            if !self.lines.is_empty() && !last_blank {
                self.lines.push(String::new());
            }
            self.lines.push(key_line);
            self.lines.push(new_line);
        }
        self.rebuild_index();
        Ok(())
    }

    /// Remove one value. Removing a missing value is a no-op.
    pub fn remove(&mut self, key: &str, name: &str) -> anyhow::Result<()> {
        let kl = key.to_ascii_lowercase();
        let nl = canonical_name(name).to_ascii_lowercase();
        if let Some(sec) = self.sections.get(&kl)
            && let Some(slot) = sec.values.get(&nl)
        {
            let idx = slot.line_idx;
            self.lines.remove(idx);
            self.rebuild_index();
        }
        Ok(())
    }

    /// Remove an entire key (its header and every line up to the next header).
    /// Removing a missing key is a no-op.
    pub fn remove_key(&mut self, key: &str) -> anyhow::Result<()> {
        let kl = key.to_ascii_lowercase();
        let mut range: Option<(usize, usize)> = None;
        if let Some(sec) = self.sections.get(&kl) {
            let start = sec.header_idx;
            let end = self.next_header_after(start);
            range = Some((start, end));
        }
        if let Some((start, end)) = range {
            self.lines.drain(start..end);
            self.rebuild_index();
        }
        Ok(())
    }

    /// All values of one key; `""` is the default value's key in the map.
    pub fn values(&self, key: &str) -> BTreeMap<String, RegValue> {
        let kl = key.to_ascii_lowercase();
        let mut out = BTreeMap::new();
        if let Some(sec) = self.sections.get(&kl) {
            for (lower, slot) in &sec.values {
                let name = if lower.is_empty() {
                    String::new()
                } else {
                    slot.display_name.clone()
                };
                out.insert(name, slot.value.clone());
            }
        }
        out
    }

    /// Immediate child key names (not full paths) under `key`, deduplicated,
    /// original casing, in file order.
    pub fn subkeys(&self, key: &str) -> Vec<String> {
        let kl = key.to_ascii_lowercase();
        let prefix = if kl.is_empty() {
            String::new()
        } else {
            format!("{}\\", kl)
        };
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        for sec in self.sections.values() {
            let lower = sec.display_key.to_ascii_lowercase();
            let Some(rest) = lower.strip_prefix(&prefix) else {
                continue;
            };
            if rest.is_empty() {
                continue;
            }
            let comp_len = rest.find('\\').unwrap_or(rest.len());
            if comp_len == 0 {
                continue;
            }
            // Byte offsets are valid for the original casing because
            // `to_ascii_lowercase` preserves byte length.
            let off = sec.display_key.len() - rest.len();
            let child = sec.display_key[off..off + comp_len].to_string();
            if seen.insert(child.to_ascii_lowercase()) {
                out.push(child);
            }
        }
        out
    }

    /// Atomic rewrite: a temp file in the same directory, fsync, then rename
    /// over the original. The original permission bits are preserved.
    pub fn save(&self) -> anyhow::Result<()> {
        let mut data = self.lines.join("\n");
        if self.ends_with_newline {
            data.push('\n');
        }
        let bytes = data.into_bytes();

        let dir = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

        let fname = self
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("registry.reg");

        let mut tmp: Option<(PathBuf, fs::File)> = None;
        let mut last_err: Option<std::io::Error> = None;
        for attempt in 0..64u32 {
            let cand = dir.join(format!(
                ".{}.wrtmp{}.{}",
                fname,
                std::process::id(),
                attempt
            ));
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&cand)
            {
                Ok(f) => {
                    tmp = Some((cand, f));
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => {
                    last_err = Some(e);
                    break;
                }
            }
        }
        let (tmp_path, mut file) = tmp.ok_or_else(|| {
            anyhow!(
                "could not create a temp file next to {}: {}",
                self.path.display(),
                last_err
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "unknown error".to_string())
            )
        })?;

        let result = write_and_rename(&mut file, &bytes, &tmp_path, &self.path, self.mode);
        drop(file);
        if result.is_err() {
            let _ = fs::remove_file(&tmp_path);
        } else {
            #[cfg(unix)]
            {
                if let Ok(d) = fs::File::open(&dir) {
                    let _ = d.sync_all();
                }
            }
        }
        result
    }

    /// Path this file was opened from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    // ---- internals -------------------------------------------------------

    fn next_header_after(&self, idx: usize) -> usize {
        for i in (idx + 1)..self.lines.len() {
            if parse_header_key(content(&self.lines[i])).is_some() {
                return i;
            }
        }
        self.lines.len()
    }

    /// Where a new value line goes inside a section: before the blank lines
    /// that separate it from the next header.
    fn value_insert_pos(&self, header_idx: usize) -> usize {
        let end = self.next_header_after(header_idx);
        let mut pos = end;
        while pos > header_idx + 1 && content(&self.lines[pos - 1]).trim().is_empty() {
            pos -= 1;
        }
        pos
    }

    fn rebuild_index(&mut self) {
        self.sections.clear();
        let mut cur: Option<String> = None;
        for i in 0..self.lines.len() {
            let line = content(&self.lines[i]).to_string();
            if let Some(k) = parse_header_key(&line) {
                let kl = k.to_ascii_lowercase();
                self.sections.entry(kl.clone()).or_insert_with(|| Section {
                    display_key: k,
                    header_idx: i,
                    values: BTreeMap::new(),
                });
                cur = Some(kl);
                continue;
            }
            if let Some(kl) = &cur
                && let Some((name, value)) = parse_value_line(&line)
                && let Some(sec) = self.sections.get_mut(kl)
            {
                let nl = name.to_ascii_lowercase();
                sec.values.insert(
                    nl,
                    Slot {
                        display_name: name,
                        line_idx: i,
                        value,
                    },
                );
            }
        }
    }
}

/// Write the temp file, fsync it, restore permissions and rename it over the
/// target.
fn write_and_rename(
    file: &mut fs::File,
    bytes: &[u8],
    tmp: &Path,
    target: &Path,
    mode: u32,
) -> anyhow::Result<()> {
    file.write_all(bytes)
        .with_context(|| format!("writing {}", tmp.display()))?;
    file.sync_all()
        .with_context(|| format!("syncing {}", tmp.display()))?;
    set_file_mode(tmp, mode)?;
    fs::rename(tmp, target)
        .with_context(|| format!("renaming {} onto {}", tmp.display(), target.display()))?;
    Ok(())
}

#[cfg(unix)]
fn file_mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o7777)
        .unwrap_or(0o644)
}

#[cfg(not(unix))]
fn file_mode(_path: &Path) -> u32 {
    0o644
}

#[cfg(unix)]
fn set_file_mode(path: &Path, mode: u32) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o7777))
        .with_context(|| format!("setting permissions on {}", path.display()))
}

#[cfg(not(unix))]
fn set_file_mode(_path: &Path, _mode: u32) -> anyhow::Result<()> {
    Ok(())
}

// ---- convenience wrappers ------------------------------------------------

/// Read one value from a hive. A missing file, key or value yields `Ok(None)`.
pub fn get_value(
    prefix: &Path,
    hive: Hive,
    key: &str,
    name: &str,
) -> anyhow::Result<Option<RegValue>> {
    let path = hive_path(prefix, hive);
    if !path.exists() {
        return Ok(None);
    }
    let rf = RegFile::open(&path)?;
    Ok(rf.get(key, name))
}

/// Set one value, creating the key path and the file if absent.
pub fn set_value(
    prefix: &Path,
    hive: Hive,
    key: &str,
    name: &str,
    value: RegValue,
) -> anyhow::Result<()> {
    let path = hive_path(prefix, hive);
    let mut rf = if path.exists() {
        RegFile::open(&path)?
    } else {
        RegFile::empty(&path)
    };
    rf.set(key, name, value)?;
    rf.save()
}

/// Remove one value, rewriting the hive file afterwards.
pub fn remove_value(prefix: &Path, hive: Hive, key: &str, name: &str) -> anyhow::Result<()> {
    let path = hive_path(prefix, hive);
    if !path.exists() {
        return Ok(());
    }
    let mut rf = RegFile::open(&path)?;
    rf.remove(key, name)?;
    rf.save()
}

// ---- text helpers --------------------------------------------------------

/// Strip a trailing `\r` (the raw line still keeps it for re-emission).
fn content(line: &str) -> &str {
    line.strip_suffix('\r').unwrap_or(line)
}

/// `""` and `"@"` both address the default value.
fn canonical_name(name: &str) -> String {
    if name.is_empty() || name == "@" {
        String::new()
    } else {
        name.to_string()
    }
}

/// Undo Wine's backslash escaping (`\\` -> `\`).
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            if let Some(n) = it.next() {
                out.push(n);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Encode a key for a header line.
fn encode_key(key: &str) -> String {
    key.replace('\\', "\\\\")
}

/// Escape a string value for a `"..."` literal.
fn escape_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    out
}

/// Parse a `"..."` literal, returning the decoded string and the number of
/// bytes consumed including the closing quote.
fn parse_quoted(s: &str) -> Option<(String, usize)> {
    let mut it = s.char_indices();
    let (_, first) = it.next()?;
    if first != '"' {
        return None;
    }
    let mut out = String::new();
    let mut esc = false;
    for (i, c) in it {
        if esc {
            out.push(c);
            esc = false;
            continue;
        }
        match c {
            '\\' => esc = true,
            '"' => return Some((out, i + 1)),
            _ => out.push(c),
        }
    }
    None
}

/// Bytes of a `hex:AA,BB,...` payload.
fn parse_hex_bytes(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    for part in s.split(',') {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        if p.len() > 2 || !p.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        out.push(u8::from_str_radix(p, 16).ok()?);
    }
    Some(out)
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!("{:02x}", b));
    }
    out
}

fn utf16le_bytes(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 2);
    for u in s.encode_utf16() {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out
}

fn u16_units(bytes: &[u8]) -> Vec<u16> {
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let mut i = 0;
    while i + 1 < bytes.len() {
        out.push(u16::from_le_bytes([bytes[i], bytes[i + 1]]));
        i += 2;
    }
    out
}

/// Decode UTF-16LE, dropping the trailing NUL terminator Wine writes for
/// `REG_EXPAND_SZ` and encoded value names.
fn utf16le_to_string(bytes: &[u8]) -> String {
    let mut units = u16_units(bytes);
    while units.last() == Some(&0) {
        units.pop();
    }
    String::from_utf16_lossy(&units)
}

/// UTF-16LE plus the terminating NUL that Wine writes for `hex(2):` values.
fn expand_sz_bytes(s: &str) -> Vec<u8> {
    let mut out = utf16le_bytes(s);
    out.extend_from_slice(&[0, 0]);
    out
}

fn utf16le_to_multi(bytes: &[u8]) -> Vec<String> {
    let mut units = u16_units(bytes);
    while units.last() == Some(&0) {
        units.pop();
    }
    let mut out = Vec::new();
    let mut cur: Vec<u16> = Vec::new();
    for u in units {
        if u == 0 {
            out.push(String::from_utf16_lossy(&cur));
            cur.clear();
        } else {
            cur.push(u);
        }
    }
    if !cur.is_empty() {
        out.push(String::from_utf16_lossy(&cur));
    }
    out
}

/// `[Key\\Sub] timestamp` -> decoded key.
fn parse_header_key(line: &str) -> Option<String> {
    let t = line.trim_start();
    if !t.starts_with('[') {
        return None;
    }
    let end = t.find(']')?;
    if end <= 1 {
        return None;
    }
    Some(unescape(&t[1..end]))
}

/// Index of the `=` that separates name from value, ignoring those inside
/// quoted names.
fn find_eq(line: &str) -> Option<usize> {
    let mut in_q = false;
    let mut esc = false;
    for (i, c) in line.char_indices() {
        if esc {
            esc = false;
            continue;
        }
        match c {
            '\\' if in_q => esc = true,
            '"' => in_q = !in_q,
            '=' if !in_q => return Some(i),
            _ => {}
        }
    }
    None
}

/// A name written as `hex(2):61,00,...` (non-ASCII names).
fn parse_encoded_name(tok: &str) -> Option<String> {
    let open = tok.find('(')?;
    let close = tok[open..].find(')')? + open;
    let rest = tok[close + 1..].strip_prefix(':')?;
    let bytes = parse_hex_bytes(rest)?;
    Some(utf16le_to_string(&bytes))
}

/// Split the decoded body of a `str(7)` value on its `\0` separators.
fn split_multi_decoded(s: &str) -> Vec<String> {
    let mut out: Vec<String> = s.split("\\0").map(|p| p.to_string()).collect();
    while out.last().map(|p| p.is_empty()).unwrap_or(false) {
        out.pop();
    }
    out
}

/// Parse the quoted body of a `str(7)` value, keeping the two-character `\0`
/// separator intact while still honouring `\\` and `\"` escapes.
fn parse_quoted_str_multi(s: &str) -> Option<String> {
    let mut chars = s.chars();
    match chars.next()? {
        '"' => {}
        _ => return None,
    }
    let mut out = String::new();
    let mut esc = false;
    for c in chars {
        if esc {
            if c == '0' {
                out.push('\\');
                out.push('0');
            } else {
                out.push(c);
            }
            esc = false;
            continue;
        }
        match c {
            '\\' => esc = true,
            '"' => return Some(out),
            _ => out.push(c),
        }
    }
    None
}

fn parse_value(vtok: &str) -> Option<RegValue> {
    // Wine also writes `str(2)` / `str(7)`: a quoted literal where `\\`
    // escapes a backslash and the multi-string separator is the two
    // characters `\` `0`.
    if let Some(rest) = vtok.strip_prefix("str(2):") {
        let (s, _) = parse_quoted(rest)?;
        return Some(RegValue::ExpandSz(s));
    }
    if let Some(rest) = vtok.strip_prefix("str(7):") {
        let s = parse_quoted_str_multi(rest)?;
        return Some(RegValue::MultiSz(split_multi_decoded(&s)));
    }
    if vtok.starts_with('"') {
        return Some(match parse_quoted(vtok) {
            Some((s, _)) => RegValue::Sz(s),
            None => RegValue::Sz(vtok.to_string()),
        });
    }
    if let Some(h) = vtok.strip_prefix("dword:") {
        return u32::from_str_radix(h.trim(), 16).ok().map(RegValue::Dword);
    }
    if vtok.starts_with("hex(") {
        let open = vtok.find('(')?;
        let close = vtok[open..].find(')')? + open;
        let tag = &vtok[open + 1..close];
        let rest = vtok[close + 1..].strip_prefix(':')?;
        let bytes = parse_hex_bytes(rest)?;
        return Some(match tag {
            "2" => RegValue::ExpandSz(utf16le_to_string(&bytes)),
            "7" => RegValue::MultiSz(utf16le_to_multi(&bytes)),
            _ => RegValue::Binary(bytes),
        });
    }
    if let Some(h) = vtok.strip_prefix("hex:") {
        return parse_hex_bytes(h).map(RegValue::Binary);
    }
    // Unquoted string: the rest of the line is the value.
    Some(RegValue::Sz(vtok.to_string()))
}

/// Parse a value line into (display name, value). `None` means the line is not
/// a recognizable value line and must be preserved verbatim.
fn parse_value_line(line: &str) -> Option<(String, RegValue)> {
    if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
        return None;
    }
    let eq = find_eq(line)?;
    let name_tok = line[..eq].trim();
    let value_tok = &line[eq + 1..];
    let name = if name_tok == "@" {
        String::new()
    } else if name_tok.starts_with('"') {
        parse_quoted(name_tok)?.0
    } else if name_tok.starts_with("hex(") {
        parse_encoded_name(name_tok)?
    } else {
        return None;
    };
    let value = parse_value(value_tok)?;
    Some((name, value))
}

fn encode_name(name: &str) -> String {
    if name.is_ascii() {
        format!("\"{}\"", escape_str(name))
    } else {
        format!("hex(2):{}", hex_bytes(&utf16le_bytes(name)))
    }
}

fn encode_value(value: &RegValue) -> String {
    match value {
        RegValue::Sz(s) => format!("\"{}\"", escape_str(s)),
        RegValue::ExpandSz(s) => format!("hex(2):{}", hex_bytes(&expand_sz_bytes(s))),
        RegValue::MultiSz(items) => {
            let mut units: Vec<u16> = Vec::new();
            for s in items {
                units.extend(s.encode_utf16());
                units.push(0);
            }
            units.push(0);
            let mut bytes = Vec::with_capacity(units.len() * 2);
            for u in units {
                bytes.extend_from_slice(&u.to_le_bytes());
            }
            format!("hex(7):{}", hex_bytes(&bytes))
        }
        RegValue::Dword(d) => format!("dword:{:08x}", d),
        RegValue::Binary(b) => format!("hex:{}", hex_bytes(b)),
    }
}

fn encode_value_line(name: &str, value: &RegValue) -> String {
    let name_tok = if name.is_empty() {
        "@".to_string()
    } else {
        encode_name(name)
    };
    format!("{}={}", name_tok, encode_value(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    const SAMPLE: &str = r#"WINE REGISTRY Version 2
;; All keys relative to REGISTRY\\Machine

#arch=win64

[Software\\Classes\\.ai] 1791225416
#time=1dd54f87fd2e9d8
"Content Type"="application/postscript"

[Software\\Classes\\CLSID\\{1234}] 1791225416
@="default value"
"Value"=dword:00000001
"Big"=hex:01,02,03
"Multi"=hex(7):61,00,00,00,62,00,00,00,00,00
"Expand"=hex(2):25,00,4c,00,4f,00,43,00,41,00,4c,00,41,00,50,00,50,00,44,00,41,00,54,00,41,00,00,00
"Qword"=hex(b):01,00,00,00,00,00,00,00
"StrMulti"=str(7):"a\0b\0"
"StrExpand"=str(2):"C:\\Program Files\\x.exe,1"
"Esc"="a\"b\\c"
"odd" no equals sign
;comment
"#;

    fn write_temp(content: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let p =
            std::env::temp_dir().join(format!("wine_reg_unit_{}_{}.reg", std::process::id(), n));
        fs::write(&p, content).unwrap();
        p
    }

    fn read(p: &Path) -> Vec<u8> {
        fs::read(p).unwrap()
    }

    #[test]
    fn round_trip_is_byte_identical() {
        let p = write_temp(SAMPLE);
        {
            let rf = RegFile::open(&p).unwrap();
            assert_eq!(rf.path(), p.as_path());
            rf.save().unwrap();
        }
        assert_eq!(read(&p), SAMPLE.as_bytes());
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn round_trip_edge_cases() {
        for text in ["", "a", "a\n", "a\n\n", "x\r\ny\r\n"] {
            let p = write_temp(text);
            RegFile::open(&p).unwrap().save().unwrap();
            assert_eq!(read(&p), text.as_bytes(), "input {:?}", text);
            let _ = fs::remove_file(&p);
        }
    }

    #[test]
    fn case_insensitive_get_and_default() {
        let p = write_temp(SAMPLE);
        let rf = RegFile::open(&p).unwrap();
        assert_eq!(
            rf.get(r"software\classes\.ai", "CONTENT type"),
            Some(RegValue::Sz("application/postscript".into()))
        );
        assert_eq!(
            rf.get(r"SOFTWARE\CLASSES\.AI", "Content Type"),
            Some(RegValue::Sz("application/postscript".into()))
        );
        let k = r"Software\Classes\CLSID\{1234}";
        assert_eq!(rf.get(k, "@"), Some(RegValue::Sz("default value".into())));
        assert_eq!(rf.get(k, ""), Some(RegValue::Sz("default value".into())));
        assert_eq!(rf.get(k, "missing"), None);
        assert_eq!(rf.get(r"Software\Classes\Missing", "x"), None);
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn decodes_all_value_types() {
        let p = write_temp(SAMPLE);
        let rf = RegFile::open(&p).unwrap();
        let k = r"Software\Classes\CLSID\{1234}";
        assert_eq!(rf.get(k, "Value"), Some(RegValue::Dword(1)));
        assert_eq!(rf.get(k, "Big"), Some(RegValue::Binary(vec![1, 2, 3])));
        assert_eq!(
            rf.get(k, "Multi"),
            Some(RegValue::MultiSz(vec!["a".into(), "b".into()]))
        );
        assert_eq!(
            rf.get(k, "Expand"),
            Some(RegValue::ExpandSz("%LOCALAPPDATA".into()))
        );
        assert_eq!(
            rf.get(k, "Qword"),
            Some(RegValue::Binary(vec![1, 0, 0, 0, 0, 0, 0, 0]))
        );
        assert_eq!(rf.get(k, "Esc"), Some(RegValue::Sz("a\"b\\c".into())));
        assert_eq!(
            rf.get(k, "StrMulti"),
            Some(RegValue::MultiSz(vec!["a".into(), "b".into()]))
        );
        assert_eq!(
            rf.get(k, "StrExpand"),
            Some(RegValue::ExpandSz(r"C:\Program Files\x.exe,1".into()))
        );
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn values_and_subkeys() {
        let p = write_temp(SAMPLE);
        let rf = RegFile::open(&p).unwrap();
        let k = r"Software\Classes\CLSID\{1234}";
        let vals = rf.values(k);
        assert!(vals.contains_key(""));
        assert_eq!(vals.get(""), Some(&RegValue::Sz("default value".into())));
        assert_eq!(vals.get("Value"), Some(&RegValue::Dword(1)));
        assert_eq!(
            vals.get("Content Type"),
            None,
            "values must not leak across keys"
        );

        let subs = rf.subkeys(r"Software\Classes");
        assert!(subs.contains(&".ai".to_string()), "{:?}", subs);
        assert!(subs.contains(&"CLSID".to_string()), "{:?}", subs);
        assert_eq!(subs.len(), 2, "dedup + exact depth: {:?}", subs);
        assert!(rf.subkeys(r"Software\Classes\CLSID\{1234}").is_empty());
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn set_remove_and_new_key() {
        let p = write_temp(SAMPLE);
        let k = r"Software\Classes\CLSID\{1234}";
        {
            let mut rf = RegFile::open(&p).unwrap();
            rf.set(k, "Value", RegValue::Dword(2)).unwrap();
            assert_eq!(rf.get(k, "value"), Some(RegValue::Dword(2)));

            rf.set(r"Software\Classes\.ai", "Added", RegValue::Dword(7))
                .unwrap();
            assert_eq!(
                rf.get(r"software\classes\.ai", "added"),
                Some(RegValue::Dword(7))
            );
            // untouched line of a modified section stays identical
            assert_eq!(
                rf.get(r"Software\Classes\.ai", "Content Type"),
                Some(RegValue::Sz("application/postscript".into()))
            );

            rf.set(r"Brand\New", "x", RegValue::Sz("y".into())).unwrap();
            assert_eq!(rf.get(r"brand\new", "x"), Some(RegValue::Sz("y".into())));
            rf.set(r"Brand\New", "", RegValue::ExpandSz("%A%".into()))
                .unwrap();
            assert_eq!(
                rf.get(r"Brand\New", "@"),
                Some(RegValue::ExpandSz("%A%".into()))
            );

            rf.remove(r"Software\Classes\.ai", "Added").unwrap();
            assert_eq!(rf.get(r"Software\Classes\.ai", "Added"), None);
            rf.remove(r"Software\Classes\.ai", "DoesNotExist").unwrap();
            rf.remove_key(r"Brand\New").unwrap();
            assert_eq!(rf.get(r"Brand\New", "x"), None);

            rf.save().unwrap();
        }
        let rf = RegFile::open(&p).unwrap();
        assert_eq!(rf.get(k, "Value"), Some(RegValue::Dword(2)));
        assert_eq!(
            rf.get(r"Software\Classes\.ai", "Content Type"),
            Some(RegValue::Sz("application/postscript".into()))
        );
        assert_eq!(rf.get(r"Brand\New", "x"), None);
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn string_escaping_round_trips_through_save() {
        let p = write_temp(SAMPLE);
        let k = r"Software\Classes\CLSID\{1234}";
        {
            let mut rf = RegFile::open(&p).unwrap();
            rf.set(k, "Esc2", RegValue::Sz("q\"u\\o".into())).unwrap();
            rf.set(k, "Multi2", RegValue::MultiSz(vec!["p".into(), "q".into()]))
                .unwrap();
            rf.save().unwrap();
        }
        let rf = RegFile::open(&p).unwrap();
        assert_eq!(rf.get(k, "Esc2"), Some(RegValue::Sz("q\"u\\o".into())));
        assert_eq!(
            rf.get(k, "Multi2"),
            Some(RegValue::MultiSz(vec!["p".into(), "q".into()]))
        );
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn convenience_wrappers() {
        let dir = std::env::temp_dir().join(format!(
            "wine_reg_prefix_{}_{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = hive_path(&dir, Hive::Machine);
        assert_eq!(path, dir.join("system.reg"));
        assert_eq!(hive_path(&dir, Hive::User), dir.join("user.reg"));

        assert_eq!(
            get_value(&dir, Hive::Machine, r"Software\X", "v").unwrap(),
            None,
            "missing file must be Ok(None)"
        );
        assert_eq!(
            get_value(&dir, Hive::Machine, r"Software\X", "v").unwrap(),
            None
        );
        set_value(&dir, Hive::Machine, r"Software\X", "v", RegValue::Dword(9)).unwrap();
        assert!(path.exists());
        assert_eq!(
            get_value(&dir, Hive::Machine, r"software\x", "V").unwrap(),
            Some(RegValue::Dword(9))
        );
        assert_eq!(
            get_value(&dir, Hive::Machine, r"Software\X", "nope").unwrap(),
            None
        );
        remove_value(&dir, Hive::Machine, r"Software\X", "v").unwrap();
        assert_eq!(
            get_value(&dir, Hive::Machine, r"Software\X", "v").unwrap(),
            None
        );
        assert_eq!(
            remove_value(&dir, Hive::User, r"Missing", "v").unwrap(),
            (),
            "missing hive file is a no-op"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_preserves_permissions() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let p = write_temp(SAMPLE);
            fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
            RegFile::open(&p).unwrap().save().unwrap();
            let mode = fs::metadata(&p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
            let _ = fs::remove_file(&p);
        }
    }
}
