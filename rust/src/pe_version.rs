//! Pure-Rust reader for the PE `VS_VERSIONINFO` resource.
//!
//! The Windows backend gets `FileVersion`/`InternalName` from
//! `GetFileVersionInfoW`; this module reads the same two strings straight out of
//! the image so diagnostics and the updater keep working on Linux.
//!
//! Every offset is bounds-checked against the input buffer: a malformed or
//! hostile executable must produce an error, never a panic or a huge read.

use anyhow::{Context, Result, ensure};

const RT_VERSION: u32 = 16;
const VS_FFI_SIGNATURE: u32 = 0xfeef04bd;

/// The version metadata this manager uses.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct VersionInfo {
    /// Four-part dotted version, e.g. `4.2.9.0`.
    pub file_version: String,
    /// `InternalName` from the resource, e.g. `RTXManager`.
    pub internal_name: String,
    /// Product version, when present.
    pub product_version: String,
}

fn u16_at(data: &[u8], offset: usize) -> Result<u16> {
    let end = offset.checked_add(2).context("偏移溢出")?;
    let slice = data.get(offset..end).context("读取 u16 越界")?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn u32_at(data: &[u8], offset: usize) -> Result<u32> {
    let end = offset.checked_add(4).context("偏移溢出")?;
    let slice = data.get(offset..end).context("读取 u32 越界")?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

/// Locates the raw bytes of the `.rsrc` section inside a PE image.
fn rsrc_section(data: &[u8]) -> Result<&[u8]> {
    ensure!(data.len() >= 0x40, "文件过小，不是 PE 镜像");
    ensure!(&data[..2] == b"MZ", "缺少 MZ 头，不是 PE 镜像");
    let pe_offset = u32_at(data, 0x3c)? as usize;
    ensure!(pe_offset + 24 <= data.len(), "PE 头偏移越界");
    ensure!(&data[pe_offset..pe_offset + 4] == b"PE\0\0", "缺少 PE 签名");
    let coff = pe_offset + 4;
    let sections = u16_at(data, coff + 2)? as usize;
    let optional_size = u16_at(data, coff + 16)? as usize;
    ensure!(sections <= 96, "节区数量异常");
    let optional = coff + 20;
    ensure!(optional + optional_size <= data.len(), "可选头越界");
    let magic = u16_at(data, optional)?;
    ensure!(magic == 0x10b || magic == 0x20b, "可选头类型异常");
    let table = optional + optional_size;
    for index in 0..sections {
        let header = table + index * 40;
        ensure!(header + 40 <= data.len(), "节区表越界");
        let name = data.get(header..header + 8).context("节区名越界")?;
        if name.starts_with(b".rsrc") {
            let pointer = u32_at(data, header + 20)? as usize;
            let size = u32_at(data, header + 16)? as usize;
            let end = pointer.checked_add(size).context("节区尺寸溢出")?;
            ensure!(end <= data.len(), "资源节区超出文件范围");
            return Ok(&data[pointer..end]);
        }
    }
    anyhow::bail!("未找到 .rsrc 资源节区")
}

struct Dir {
    /// Absolute offset of this resource directory inside the section.
    base: usize,
}

impl Dir {
    fn entries(&self, section: &[u8]) -> Result<Vec<Entry>> {
        let count =
            u16_at(section, self.base + 12)? as usize + u16_at(section, self.base + 14)? as usize;
        ensure!(count <= 4096, "资源目录条目过多");
        let mut result = Vec::with_capacity(count);
        for index in 0..count {
            // Each entry is 8 bytes: name/id, offset, and the entry-relative data.
            let entry = self.base + 16 + index * 8;
            ensure!(entry + 8 <= section.len(), "资源目录越界");
            let name = u32_at(section, entry)?;
            let offset = u32_at(section, entry + 4)?;
            result.push(Entry { name, offset });
        }
        Ok(result)
    }
}

struct Entry {
    name: u32,
    offset: u32,
}

impl Entry {
    fn is_directory(&self) -> bool {
        self.offset & 0x8000_0000 != 0
    }
    fn id(&self) -> u32 {
        self.name & 0xffff
    }
    fn child_dir(&self) -> Dir {
        Dir {
            base: (self.offset & 0x7fff_ffff) as usize,
        }
    }
    /// Offset of the IMAGE_RESOURCE_DATA_ENTRY this entry points at.
    ///
    /// Per the PE specification `OffsetToData` is relative to the **resource
    /// directory root**, not to this entry. Adding the entry's own position
    /// (a tempting mistake) only works when the leaf happens to sit at the
    /// directory base, and reads garbage otherwise.
    fn data_entry(&self) -> usize {
        (self.offset & 0x7fff_ffff) as usize
    }
}

fn find_type(section: &[u8], id: u32) -> Result<Option<Entry>> {
    let root = Dir { base: 0 };
    for entry in root.entries(section)? {
        if entry.is_directory() && entry.id() == id {
            return Ok(Some(entry));
        }
    }
    Ok(None)
}

/// Reads the bytes a LEAF entry points at. The caller must have descended to a
/// non-directory entry; a directory entry here would mean the walk was wrong.
fn data_of<'a>(section: &'a [u8], entry: &Entry) -> Result<&'a [u8]> {
    ensure!(!entry.is_directory(), "资源条目不是数据叶子");
    let target = entry.data_entry();
    let offset = u32_at(section, target)? as usize;
    let size = u32_at(section, target + 4)? as usize;
    let end = offset.checked_add(size).context("资源尺寸溢出")?;
    ensure!(end <= section.len(), "资源数据越界");
    Ok(&section[offset..end])
}

/// Reads the version resource out of a complete PE image.
pub fn parse(data: &[u8]) -> Result<VersionInfo> {
    let section = rsrc_section(data)?;
    let kind = find_type(section, RT_VERSION)?.context("文件没有版本资源")?;
    // Descend Type -> Name -> Language to the leaf data entry. The intermediate
    // levels are DIRECTORY entries whose `Offset` points at the next directory;
    // passing one of those to `data_of` would read a directory header instead of
    // the version bytes, so each level must be walked explicitly.
    let mut leaves = Vec::new();
    for name_entry in kind.child_dir().entries(section)? {
        if !name_entry.is_directory() {
            leaves.push(name_entry);
            continue;
        }
        for lang_entry in name_entry.child_dir().entries(section)? {
            if !lang_entry.is_directory() {
                leaves.push(lang_entry);
            }
        }
    }
    let leaf = leaves.first().context("版本资源为空")?;
    let bytes = data_of(section, leaf)?;
    parse_version_block(bytes)
}

/// Walks one `VS_VERSIONINFO` block, collecting the string table we care about.
fn parse_version_block(bytes: &[u8]) -> Result<VersionInfo> {
    let mut info = VersionInfo::default();
    // Header: wLength(2) wValueLength(2) wType(2), then a NUL-terminated
    // UTF-16 key, aligned to 4 bytes.
    let key_end = utf16_key_end(bytes, 6)?;
    let value_offset = align4(key_end)?;
    // A VS_FIXEDFILEINFO follows (52 bytes) and is optional in theory.
    if bytes.len() >= value_offset + 4 && u32_at(bytes, value_offset)? == VS_FFI_SIGNATURE {
        ensure!(bytes.len() >= value_offset + 52, "固定版本信息不完整");
        let ms = u32_at(bytes, value_offset + 8)?;
        let ls = u32_at(bytes, value_offset + 12)?;
        info.file_version = format!("{}.{}.{}.{}", ms >> 16, ms & 0xffff, ls >> 16, ls & 0xffff);
        let pms = u32_at(bytes, value_offset + 16)?;
        let pls = u32_at(bytes, value_offset + 20)?;
        info.product_version = format!(
            "{}.{}.{}.{}",
            pms >> 16,
            pms & 0xffff,
            pls >> 16,
            pls & 0xffff
        );
        // Children (StringFileInfo) begin after the fixed info, 4-byte aligned.
        let children = align4(value_offset + 52)?;
        if children < bytes.len() {
            collect_children(&bytes[children..], &mut info)?;
        }
    }
    Ok(info)
}

/// Finds the end offset of a NUL-terminated UTF-16LE key starting at `start`.
fn utf16_key_end(bytes: &[u8], start: usize) -> Result<usize> {
    let mut offset = start;
    loop {
        ensure!(offset + 2 <= bytes.len(), "版本资源键越界");
        if bytes[offset] == 0 && bytes[offset + 1] == 0 {
            return Ok(offset + 2);
        }
        offset += 2;
    }
}

fn align4(offset: usize) -> Result<usize> {
    offset.checked_add(3).map(|v| v & !3).context("对齐溢出")
}

/// Iterates the children of `StringFileInfo` -> `StringTable` -> `String`.
fn collect_children(bytes: &[u8], info: &mut VersionInfo) -> Result<()> {
    let mut offset = 0;
    while offset + 6 <= bytes.len() {
        let length = u16_at(bytes, offset)? as usize;
        if length == 0 {
            break;
        }
        let end = offset.checked_add(length).context("子块长度溢出")?;
        if end > bytes.len() {
            break;
        }
        let block = &bytes[offset..end];
        let key_end = utf16_key_end(block, 6)?;
        let key = String::from_utf16_lossy(
            &block[6..key_end.saturating_sub(2)]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect::<Vec<_>>(),
        );
        let value_offset = align4(key_end)?;
        match key.as_str() {
            "StringFileInfo" => {
                if value_offset < block.len() {
                    collect_children(&block[value_offset..], info)?;
                }
            }
            // Any of the language tables may hold the strings we need.
            _ if key.len() == 8 && key.bytes().all(|b| b.is_ascii_hexdigit()) => {
                if value_offset < block.len() {
                    collect_children(&block[value_offset..], info)?;
                }
            }
            "FileVersion" | "InternalName" | "ProductVersion" => {
                let text = read_string_value(block, value_offset)?;
                match key.as_str() {
                    "FileVersion" => info.file_version = text,
                    "InternalName" => info.internal_name = text,
                    _ => info.product_version = text,
                }
            }
            _ => {}
        }
        offset = align4(end)?;
    }
    Ok(())
}

/// Reads a UTF-16 string value, whose length is counted in UTF-16 code units
/// (including the trailing NUL) when `wType` is 1.
fn read_string_value(block: &[u8], value_offset: usize) -> Result<String> {
    let value_length = u16_at(block, 2)? as usize;
    let value_type = u16_at(block, 4)?;
    let max_units = if value_type == 1 && value_length > 0 {
        value_length.saturating_sub(1)
    } else {
        // Fall back to reading until the terminator.
        (block.len().saturating_sub(value_offset)) / 2
    };
    let mut units = Vec::new();
    for index in 0..max_units {
        let at = value_offset + index * 2;
        if at + 2 > block.len() {
            break;
        }
        let unit = u16_at(block, at)?;
        if unit == 0 {
            break;
        }
        units.push(unit);
    }
    Ok(String::from_utf16_lossy(&units))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(text: &str) -> Vec<u8> {
        let mut out = Vec::new();
        for unit in text.encode_utf16() {
            out.extend_from_slice(&unit.to_le_bytes());
        }
        out
    }

    /// Builds a `String` block: header + UTF-16 key + NUL + value, 4-aligned.
    fn string_block(key: &str, value: &str) -> Vec<u8> {
        let key_bytes = utf16(key);
        let value_bytes = utf16(value);
        let mut block = Vec::new();
        block.extend_from_slice(&[0, 0, 0, 0, 1, 0]); // length/count filled below
        block.extend_from_slice(&key_bytes);
        block.extend_from_slice(&[0, 0]);
        while block.len() % 4 != 0 {
            block.push(0);
        }
        block.extend_from_slice(&value_bytes);
        block.extend_from_slice(&[0, 0]);
        while block.len() % 4 != 0 {
            block.push(0);
        }
        let length = block.len() as u16;
        block[0..2].copy_from_slice(&length.to_le_bytes());
        let units = (value.encode_utf16().count() + 1) as u16;
        block[2..4].copy_from_slice(&units.to_le_bytes());
        block
    }

    #[test]
    fn reads_file_version_and_internal_name_from_a_version_block() -> Result<()> {
        let mut table = Vec::new();
        table.extend_from_slice(&string_block("FileVersion", "4.2.9.0"));
        table.extend_from_slice(&string_block("InternalName", "RTXManager"));
        let mut table_block = Vec::new();
        table_block.extend_from_slice(&[0, 0, 0, 0, 1, 0]);
        table_block.extend_from_slice(&utf16("040904B0"));
        table_block.extend_from_slice(&[0, 0]);
        while table_block.len() % 4 != 0 {
            table_block.push(0);
        }
        table_block.extend_from_slice(&table);
        let length = table_block.len() as u16;
        table_block[0..2].copy_from_slice(&length.to_le_bytes());
        let mut string_file_info = Vec::new();
        string_file_info.extend_from_slice(&[0, 0, 0, 0, 1, 0]);
        string_file_info.extend_from_slice(&utf16("StringFileInfo"));
        string_file_info.extend_from_slice(&[0, 0]);
        while string_file_info.len() % 4 != 0 {
            string_file_info.push(0);
        }
        string_file_info.extend_from_slice(&table_block);
        let length = string_file_info.len() as u16;
        string_file_info[0..2].copy_from_slice(&length.to_le_bytes());

        let mut block = Vec::new();
        block.extend_from_slice(&[0, 0, 0, 0, 1, 0]); // wType = 1
        block.extend_from_slice(&utf16("VS_VERSION_INFO"));
        block.extend_from_slice(&[0, 0]);
        while block.len() % 4 != 0 {
            block.push(0);
        }
        block.extend_from_slice(&VS_FFI_SIGNATURE.to_le_bytes());
        block.extend_from_slice(&0u32.to_le_bytes()); // struc version
        block.extend_from_slice(&0x0004_0002u32.to_le_bytes()); // 4.2
        block.extend_from_slice(&0x0009_0000u32.to_le_bytes()); // 9.0
        block.extend_from_slice(&0x0004_0002u32.to_le_bytes());
        block.extend_from_slice(&0x0009_0000u32.to_le_bytes());
        // VS_FIXEDFILEINFO is exactly 52 bytes (13 DWORDs). Writing fewer shifts
        // the children offset and the walker would start inside a child block.
        block.extend_from_slice(&[0u8; 28]); // flags/OS/type/date fields
        while block.len() % 4 != 0 {
            block.push(0);
        }
        block.extend_from_slice(&string_file_info);
        let length = block.len() as u16;
        block[0..2].copy_from_slice(&length.to_le_bytes());

        let info = parse_version_block(&block)?;
        assert_eq!(info.file_version, "4.2.9.0");
        assert_eq!(info.internal_name, "RTXManager");
        Ok(())
    }

    /// Builds a structurally real PE32+ carrying a genuine VS_VERSIONINFO tree.
    ///
    /// This is deliberately constructed in-code rather than committed as a
    /// binary: `tools/export-source.ps1` rejects `*.exe` in the source allowlist,
    /// and a synthetic image keeps the test self-explanatory. It is the test that
    /// catches resource-tree mistakes — an earlier parser mis-nested the
    /// Type/Name/Language descent and treated `OffsetToData` as entry-relative,
    /// and both flaws only surface on a real image.
    fn versioned_pe32plus() -> Vec<u8> {
        /// UTF-16LE with a NUL terminator (no padding; blocks are aligned as a
        /// whole, because padding each piece independently misaligns the child
        /// value that follows the key).
        fn utf16z(text: &str) -> Vec<u8> {
            let mut out = utf16(text);
            out.extend_from_slice(&[0, 0]);
            out
        }
        fn align4(data: &mut Vec<u8>) {
            while !data.len().is_multiple_of(4) {
                data.push(0);
            }
        }
        /// One VS_VERSIONINFO child: header + key + NUL + value, 4-byte aligned.
        fn block(key: &str, value: &str) -> Vec<u8> {
            let mut body = vec![0, 0, 0, 0];
            body.extend_from_slice(&[1, 0]);
            body.extend_from_slice(&utf16z(key));
            align4(&mut body);
            body.extend_from_slice(&utf16z(value));
            align4(&mut body);
            let units = (value.encode_utf16().count() + 1) as u16;
            let mut out = (body.len() as u16).to_le_bytes().to_vec();
            out.extend_from_slice(&units.to_le_bytes());
            out.extend_from_slice(&body[4..]);
            out
        }
        fn container(key: &str, children: &[u8]) -> Vec<u8> {
            let mut body = vec![0, 0, 0, 0, 1, 0];
            body.extend_from_slice(&utf16z(key));
            align4(&mut body);
            body.extend_from_slice(children);
            let mut out = (body.len() as u16).to_le_bytes().to_vec();
            out.extend_from_slice(&[0, 0]);
            out.extend_from_slice(&body[4..]);
            out
        }

        // The language table is itself a container whose children are the
        // String blocks; it must carry its own length header, so build the
        // children first and wrap them with the language key.
        let mut strings = block("FileVersion", "4.2.9.0");
        strings.extend_from_slice(&block("InternalName", "RTXManager"));
        let string_table = container("040904B0", &strings);
        let string_file_info = container("StringFileInfo", &string_table);

        let mut root_body = vec![0, 0, 0, 0, 1, 0];
        root_body.extend_from_slice(&utf16z("VS_VERSION_INFO"));
        align4(&mut root_body);
        root_body.extend_from_slice(&VS_FFI_SIGNATURE.to_le_bytes());
        root_body.extend_from_slice(&1u32.to_le_bytes());
        root_body.extend_from_slice(&0x0004_0002u32.to_le_bytes());
        root_body.extend_from_slice(&0x0009_0000u32.to_le_bytes());
        root_body.extend_from_slice(&[0u8; 36]);
        root_body.extend_from_slice(&string_file_info);
        let mut blob = (root_body.len() as u16).to_le_bytes().to_vec();
        blob.extend_from_slice(&26u16.to_le_bytes());
        blob.extend_from_slice(&root_body[4..]);

        // Resource tree: root -> RT_VERSION(16) -> id 1 -> lang 0x409 -> data.
        let dirent = |id: u32, offset: u32, is_dir: bool| {
            let mut out = id.to_le_bytes().to_vec();
            out.extend_from_slice(&(offset | if is_dir { 0x8000_0000 } else { 0 }).to_le_bytes());
            out
        };
        let dir_header = |count: u16| {
            let mut out = vec![0u8; 12];
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&count.to_le_bytes());
            out
        };
        let mut rsrc = dir_header(1);
        rsrc.extend_from_slice(&dirent(RT_VERSION, 0x20, true));
        rsrc.resize(0x20, 0);
        rsrc.extend_from_slice(&dir_header(1));
        rsrc.extend_from_slice(&dirent(1, 0x40, true));
        rsrc.resize(0x40, 0);
        rsrc.extend_from_slice(&dir_header(1));
        rsrc.extend_from_slice(&dirent(0x409, 0x58, false));
        rsrc.resize(0x58, 0);
        // IMAGE_RESOURCE_DATA_ENTRY: OffsetToData is relative to the RESOURCE
        // DIRECTORY ROOT (not to this entry), which is the trap this tests.
        rsrc.extend_from_slice(&0x80u32.to_le_bytes());
        rsrc.extend_from_slice(&(blob.len() as u32).to_le_bytes());
        rsrc.extend_from_slice(&0u32.to_le_bytes());
        rsrc.extend_from_slice(&0u32.to_le_bytes());
        rsrc.resize(0x80, 0);
        rsrc.extend_from_slice(&blob);
        while rsrc.len() % 8 != 0 {
            rsrc.push(0);
        }

        const SECTION_VA: u32 = 0x1000;
        let mut pe = vec![0u8; 0x200];
        pe[..2].copy_from_slice(b"MZ");
        pe[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        pe[0x80..0x84].copy_from_slice(b"PE\0\0");
        // COFF header (exactly 20 bytes): machine, sections, size of optional.
        let coff = 0x84;
        pe[coff..coff + 2].copy_from_slice(&0x8664u16.to_le_bytes());
        pe[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes());
        pe[coff + 16..coff + 18].copy_from_slice(&0xf0u16.to_le_bytes());
        // Optional header (PE32+): magic, alignment, resource directory (index 2).
        let opt = coff + 20;
        pe[opt..opt + 2].copy_from_slice(&0x20bu16.to_le_bytes());
        pe[opt + 32..opt + 36].copy_from_slice(&SECTION_VA.to_le_bytes());
        pe[opt + 36..opt + 40].copy_from_slice(&0x200u32.to_le_bytes());
        pe[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes());
        let resource_dir = opt + 112 + 2 * 8;
        pe[resource_dir..resource_dir + 4].copy_from_slice(&SECTION_VA.to_le_bytes());
        pe[resource_dir + 4..resource_dir + 8].copy_from_slice(&(rsrc.len() as u32).to_le_bytes());
        // One `.rsrc` section header, immediately after the optional header.
        let sh = opt + 0xf0;
        pe[sh..sh + 8].copy_from_slice(b".rsrc\0\0\0");
        pe[sh + 8..sh + 12].copy_from_slice(&(rsrc.len() as u32).to_le_bytes());
        pe[sh + 12..sh + 16].copy_from_slice(&SECTION_VA.to_le_bytes());
        pe[sh + 16..sh + 20].copy_from_slice(&(rsrc.len() as u32).to_le_bytes());
        pe[sh + 20..sh + 24].copy_from_slice(&0x200u32.to_le_bytes());
        pe.extend_from_slice(&rsrc);
        pe
    }

    #[test]
    fn reads_version_metadata_from_a_real_pe_image() -> Result<()> {
        let info = parse(&versioned_pe32plus())?;
        assert_eq!(info.file_version, "4.2.9.0");
        assert_eq!(info.internal_name, "RTXManager");
        Ok(())
    }

    #[test]
    fn rejects_non_pe_input_with_a_clear_error() {
        assert!(parse(b"not a pe file").is_err());
        assert!(parse(&[]).is_err());
        assert!(parse(b"MZ").is_err());
    }

    #[test]
    fn bounded_reads_never_panic_on_truncated_blocks() {
        // Every truncation of a valid-ish block must return, not panic.
        let block = vec![8, 0, 0, 0, 1, 0, b'X', 0, 0, 0];
        for end in 0..block.len() {
            let _ = parse_version_block(&block[..end]);
        }
    }
}
