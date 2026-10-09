//! Small, bounded JSONC editor. Edits values/keys without reformatting the document.
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{collections::BTreeMap, ops::Range};

#[derive(Debug)]
pub struct Node {
    pub range: Range<usize>,
    pub fields: BTreeMap<String, Node>,
    pub key_range: Option<Range<usize>>,
    pub object: bool,
}
pub struct Document {
    pub text: String,
    pub value: Value,
    pub root: Node,
    bom: bool,
}

fn uncomment(text: &str) -> Result<String> {
    let b = text.as_bytes();
    let mut out = b.to_vec();
    let (mut i, mut string, mut escaped) = (0, false, false);
    while i < b.len() {
        if string {
            if escaped {
                escaped = false;
            } else if b[i] == b'\\' {
                escaped = true;
            } else if b[i] == b'"' {
                string = false;
            }
            i += 1;
        } else if b[i] == b'"' {
            string = true;
            i += 1;
        } else if b[i..].starts_with(b"//") {
            while i < b.len() && !matches!(b[i], b'\r' | b'\n') {
                out[i] = b' ';
                i += 1;
            }
        } else if b[i..].starts_with(b"/*") {
            out[i] = b' ';
            out[i + 1] = b' ';
            i += 2;
            while i < b.len() && !b[i..].starts_with(b"*/") {
                if !matches!(b[i], b'\r' | b'\n') {
                    out[i] = b' ';
                }
                i += 1;
            }
            ensure!(i + 1 < b.len(), "Unterminated JSONC comment");
            out[i] = b' ';
            out[i + 1] = b' ';
            i += 2;
        } else {
            i += 1;
        }
    }
    String::from_utf8(out).context("Invalid UTF-8 JSONC")
}
fn ws(b: &[u8], p: &mut usize) {
    while *p < b.len() && b[*p].is_ascii_whitespace() {
        *p += 1;
    }
}
fn string_end(b: &[u8], start: usize) -> usize {
    let mut p = start + 1;
    while p < b.len() {
        if b[p] == b'\\' {
            p += 2;
        } else if b[p] == b'"' {
            return p + 1;
        } else {
            p += 1;
        }
    }
    b.len()
}
// serde_json validates syntax/depth first; this pass retains offsets and rejects
// duplicate object keys, including objects nested inside unknown arrays.
fn node(text: &str, p: &mut usize) -> Result<Node> {
    let b = text.as_bytes();
    ws(b, p);
    let start = *p;
    let object = b[*p] == b'{';
    let mut fields = BTreeMap::new();
    match b[*p] {
        b'{' => {
            *p += 1;
            ws(b, p);
            while b[*p] != b'}' {
                let key_start = *p;
                let end = string_end(b, *p);
                let key: String = serde_json::from_str(&text[*p..end])?;
                *p = end;
                ws(b, p);
                *p += 1;
                let mut child = node(text, p)?;
                child.key_range = Some(key_start..end);
                ensure!(fields.insert(key, child).is_none(), "Duplicate JSONC key");
                ws(b, p);
                if b[*p] == b',' {
                    *p += 1;
                    ws(b, p);
                }
            }
            *p += 1;
        }
        b'[' => {
            *p += 1;
            ws(b, p);
            while b[*p] != b']' {
                node(text, p)?;
                ws(b, p);
                if b[*p] == b',' {
                    *p += 1;
                    ws(b, p);
                }
            }
            *p += 1;
        }
        b'"' => *p = string_end(b, *p),
        _ => {
            while *p < b.len() && !b",}] \r\n\t".contains(&b[*p]) {
                *p += 1;
            }
        }
    }
    Ok(Node {
        range: start..*p,
        fields,
        key_range: None,
        object,
    })
}
impl Document {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() <= 4 * 1024 * 1024,
            "JSONC configuration exceeds 4 MiB"
        );
        let bom = bytes.starts_with(&[239, 187, 191]);
        let text = std::str::from_utf8(if bom { &bytes[3..] } else { bytes })?.to_owned();
        let stripped = uncomment(&text)?;
        let value: Value = serde_json::from_str(&stripped)?;
        ensure!(value.is_object(), "Configuration must be a JSON object");
        let root = node(&stripped, &mut 0)?;
        Ok(Self {
            text,
            value,
            root,
            bom,
        })
    }
    pub fn get(&self, path: &[String]) -> Result<Option<&Value>> {
        let mut v = &self.value;
        for key in path {
            let obj = v
                .as_object()
                .context("Configuration section must be an object")?;
            let Some(next) = obj.get(key) else {
                return Ok(None);
            };
            v = next;
        }
        Ok(Some(v))
    }
    fn output(self) -> Vec<u8> {
        let mut bytes = if self.bom {
            vec![239, 187, 191]
        } else {
            Vec::new()
        };
        bytes.extend(self.text.as_bytes());
        bytes
    }
    pub fn set(mut self, path: &[String], value: Value) -> Result<Vec<u8>> {
        ensure!(!path.is_empty(), "Empty JSON path");
        let mut n = &self.root;
        for (i, key) in path.iter().enumerate() {
            ensure!(n.object, "Configuration section must be an object: {key}");
            if let Some(child) = n.fields.get(key) {
                if i + 1 == path.len() {
                    self.text
                        .replace_range(child.range.clone(), &serde_json::to_string(&value)?);
                    return Ok(self.output());
                }
                n = child;
            } else {
                let mut added = value;
                for component in path[i + 1..].iter().rev() {
                    added = Value::Object(serde_json::Map::from_iter([(component.clone(), added)]));
                }
                let at = n.range.end - 1;
                let newline = if self.text.contains("\r\n") {
                    "\r\n"
                } else {
                    "\n"
                };
                // Newline before comma keeps an existing trailing // comment intact.
                let entry = format!(
                    "{newline}{}{}: {}{newline}",
                    if n.fields.is_empty() { "" } else { "," },
                    serde_json::to_string(key)?,
                    serde_json::to_string(&added)?
                );
                self.text.insert_str(at, &entry);
                return Ok(self.output());
            }
        }
        unreachable!()
    }
    pub fn remove(mut self, path: &[String]) -> Result<Vec<u8>> {
        ensure!(!path.is_empty(), "Empty JSON path");
        let mut parent = &self.root;
        for key in &path[..path.len() - 1] {
            let Some(child) = parent.fields.get(key) else {
                return Ok(self.output());
            };
            parent = child;
        }
        let Some(child) = parent.fields.get(path.last().unwrap()) else {
            return Ok(self.output());
        };
        let key = child.key_range.as_ref().context("Missing field key span")?;
        let stripped = uncomment(&self.text)?;
        let b = stripped.as_bytes();
        // Remove syntax tokens independently so comments between key, colon and
        // value, and trailing comments, remain byte-for-byte intact.
        let mut colon = key.end;
        ws(b, &mut colon);
        ensure!(b.get(colon) == Some(&b':'), "Missing JSON colon");
        let mut edits = vec![key.clone(), colon..colon + 1, child.range.clone()];
        let mut after = child.range.end;
        ws(b, &mut after);
        if b.get(after) == Some(&b',') {
            edits.push(after..after + 1);
        } else {
            let mut before = key.start;
            while before > parent.range.start && b[before - 1].is_ascii_whitespace() {
                before -= 1;
            }
            if before > 0 && b[before - 1] == b',' {
                edits.push(before - 1..before);
            }
        }
        edits.sort_by_key(|r| std::cmp::Reverse(r.start));
        for r in edits {
            self.text.replace_range(r, "");
        }
        let out = self.output();
        Self::parse(&out)?;
        Ok(out)
    }
}
