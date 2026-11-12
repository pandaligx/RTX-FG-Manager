//! Pure-Rust game icon extraction for Linux.
//!
//! A drop-in replacement for the Windows `SHDefExtractIconW` / `ExtractIconExW`
//! shell APIs: it reads a game's icon out of a PE executable (`.exe` / `.dll`)
//! or a standalone `.ico` file **without ever executing it**.
//!
//! Supported sources:
//! * standalone `.ico` files (`ICONDIR` / `ICONDIRENTRY`),
//! * PE resources `RT_GROUP_ICON` (type 3) + `RT_ICON` (type 14), where the
//!   icon payload is either a DIB (`BITMAPINFOHEADER`, 32/24/8/4/1 bpp) or an
//!   embedded PNG.
//!
//! The public contract is exactly a 128x128 RGBA image (`ICON_PIXELS`, same as
//! the Windows implementation). Anything else is scaled with Lanczos3. A
//! missing/undecodable icon is always an error, never a blank image.

use anyhow::{Context, Result, anyhow, bail};
use image::{ImageFormat, RgbaImage};

/// Side length, in pixels, of the image returned by [`extract`].
pub const ICON_PIXELS: u32 = 128;

/// Hard cap on any single icon payload we are willing to load/decode.
const MAX_ICON_BYTES: usize = 4 * 1024 * 1024;

/// PE resource type: icon directory group (GRPICONDIR).
const RT_GROUP_ICON: u32 = 3;
/// PE resource type: a single icon bitmap (DIB or PNG).
const RT_ICON: u32 = 14;

const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// Extract a 128x128 RGBA icon from `path`.
///
/// `path` may be a standalone `.ico` file or a PE image (`.exe` / `.dll`).
/// Errors with a descriptive message when the file has no icon, is not a PE /
/// ICO, or the icon cannot be decoded.
pub fn extract(path: &std::path::Path) -> Result<RgbaImage> {
    let data = std::fs::read(path)
        .with_context(|| format!("failed to read icon source {}", path.display()))?;

    let image = if data.starts_with(b"MZ") {
        extract_pe_icon(&data)
            .with_context(|| format!("failed to extract icon from PE file {}", path.display()))?
    } else if data.starts_with(&[0x00, 0x00, 0x01, 0x00]) || has_ico_extension(path) {
        extract_standalone_ico(&data)
            .with_context(|| format!("failed to decode .ico file {}", path.display()))?
    } else {
        bail!(
            "{} is neither a PE executable (missing 'MZ') nor an .ico file",
            path.display()
        );
    };

    finalize(image, path)
}

fn has_ico_extension(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("ico"))
}

/// Guarantee the exact 128x128 RGBA output contract.
fn finalize(image: RgbaImage, path: &std::path::Path) -> Result<RgbaImage> {
    if image.width() == 0 || image.height() == 0 {
        bail!("decoded icon from {} has zero size", path.display());
    }
    if image.width() == ICON_PIXELS && image.height() == ICON_PIXELS {
        return Ok(image);
    }
    Ok(image::imageops::resize(
        &image,
        ICON_PIXELS,
        ICON_PIXELS,
        image::imageops::FilterType::Lanczos3,
    ))
}

// ---------------------------------------------------------------------------
// Checked little-endian readers
// ---------------------------------------------------------------------------

fn read_u8(data: &[u8], off: usize) -> Result<u8> {
    data.get(off)
        .copied()
        .ok_or_else(|| anyhow!("unexpected end of data at offset {off}"))
}

fn read_u16(data: &[u8], off: usize) -> Result<u16> {
    let end = off
        .checked_add(2)
        .ok_or_else(|| anyhow!("offset overflow reading u16 at {off}"))?;
    let b = data
        .get(off..end)
        .ok_or_else(|| anyhow!("unexpected end of data reading u16 at offset {off}"))?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}

fn read_u32(data: &[u8], off: usize) -> Result<u32> {
    let end = off
        .checked_add(4)
        .ok_or_else(|| anyhow!("offset overflow reading u32 at {off}"))?;
    let b = data
        .get(off..end)
        .ok_or_else(|| anyhow!("unexpected end of data reading u32 at offset {off}"))?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn read_i32(data: &[u8], off: usize) -> Result<i32> {
    Ok(read_u32(data, off)? as i32)
}

fn cap_icon_size(size: usize) -> Result<usize> {
    if size == 0 {
        bail!("icon resource has zero length");
    }
    if size > MAX_ICON_BYTES {
        bail!("icon resource is too large ({size} bytes, limit {MAX_ICON_BYTES})");
    }
    Ok(size)
}

/// ICO/GRPICONDIR dimensions encode 256 as 0.
fn ico_dim(raw: u8) -> u32 {
    if raw == 0 { 256 } else { raw as u32 }
}

/// Pick the best entry among icon directory dimensions.
///
/// Preference: exactly 128x128, then the nearest width above 128, then the
/// largest width <= 256, and finally the largest available at all.
fn choose_best_index(dims: &[(u32, u32)]) -> Option<usize> {
    if dims.is_empty() {
        return None;
    }

    // 1. Exactly 128x128: the native output size, no resampling needed.
    for (i, &d) in dims.iter().enumerate() {
        if d.0 == 128 && d.1 == 128 {
            return Some(i);
        }
    }

    // 2. Nearest above 128 (and <= 256): downscales cleanly.
    let mut best: Option<(usize, (u32, u32))> = None;
    for (i, &d) in dims.iter().enumerate() {
        if d.0 <= 256 && d.1 <= 256 && d.0 > 128 && best.is_none_or(|(_, b)| d < b) {
            best = Some((i, d));
        }
    }
    if let Some((i, _)) = best {
        return Some(i);
    }

    // 3. Largest frame that is still <= 256.
    for (i, &d) in dims.iter().enumerate() {
        if d.0 <= 256 && d.1 <= 256 && best.is_none_or(|(_, b)| d > b) {
            best = Some((i, d));
        }
    }
    if let Some((i, _)) = best {
        return Some(i);
    }

    // 4. Anything available, largest first.
    for (i, &d) in dims.iter().enumerate() {
        if best.is_none_or(|(_, b)| d > b) {
            best = Some((i, d));
        }
    }
    best.map(|(i, _)| i)
}

// ---------------------------------------------------------------------------
// Standalone .ico
// ---------------------------------------------------------------------------

/// Decode a standalone `.ico` file, choosing the best frame.
fn extract_standalone_ico(data: &[u8]) -> Result<RgbaImage> {
    if data.len() < 6 {
        bail!(".ico file too small ({} bytes)", data.len());
    }
    let reserved = read_u16(data, 0)?;
    let kind = read_u16(data, 2)?;
    let count = read_u16(data, 4)? as usize;
    if reserved != 0 || kind != 1 {
        bail!("not an ICO file (reserved={reserved}, type={kind})");
    }
    if count == 0 {
        bail!(".ico file contains no images");
    }

    // ICONDIRENTRY is 16 bytes: width, height, colorCount, reserved,
    // planes(u16), bitCount(u16), bytesInRes(u32), imageOffset(u32).
    let need = 6usize
        .checked_add(count.saturating_mul(16))
        .ok_or_else(|| anyhow!(".ico directory size overflow"))?;
    if need > data.len() {
        bail!(
            ".ico directory truncated: {count} entries need {need} bytes, file has {}",
            data.len()
        );
    }

    let mut frames: Vec<(u32, u32, usize, usize)> = Vec::with_capacity(count);
    for i in 0..count {
        let e = 6 + i * 16;
        let w = ico_dim(read_u8(data, e)?);
        let h = ico_dim(read_u8(data, e + 1)?);
        let bytes_in_res = read_u32(data, e + 8)?;
        let offset = read_u32(data, e + 12)?;
        frames.push((w, h, offset as usize, bytes_in_res as usize));
    }

    let dims: Vec<(u32, u32)> = frames.iter().map(|f| (f.0, f.1)).collect();
    let idx =
        choose_best_index(&dims).ok_or_else(|| anyhow!(".ico file has no usable image entries"))?;
    let (_, _, offset, size) = frames[idx];
    let size = cap_icon_size(size)?;
    let end = offset
        .checked_add(size)
        .ok_or_else(|| anyhow!(".ico frame offset overflow"))?;
    let frame = data
        .get(offset..end)
        .ok_or_else(|| anyhow!(".ico frame {idx} extends past end of file"))?;

    decode_icon_resource(frame).with_context(|| format!(".ico frame {idx} could not be decoded"))
}

// ---------------------------------------------------------------------------
// PE resource walk
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Section {
    name: [u8; 8],
    virtual_address: u32,
    size_of_raw_data: u32,
    pointer_to_raw_data: u32,
}

fn section_name_is(name: &[u8; 8], want: &str) -> bool {
    let end = name.iter().position(|&c| c == 0).unwrap_or(8);
    &name[..end] == want.as_bytes()
}

/// Parsed PE layout: the section table plus the resource data-directory RVA.
struct PeLayout {
    sections: Vec<Section>,
    /// `DataDirectory[IMAGE_DIRECTORY_ENTRY_RESOURCE].VirtualAddress`.
    resource_rva: u32,
}

fn parse_pe_sections(data: &[u8]) -> Result<PeLayout> {
    if data.len() < 0x40 {
        bail!("file too small to be a PE image ({} bytes)", data.len());
    }
    if &data[0..2] != b"MZ" {
        bail!("missing DOS 'MZ' signature");
    }

    let lfanew = read_u32(data, 0x3C)? as usize;
    let pe_sig_end = lfanew
        .checked_add(4)
        .ok_or_else(|| anyhow!("PE header offset overflow"))?;
    match data.get(lfanew..pe_sig_end) {
        Some(sig) if sig == b"PE\0\0" => {}
        _ => bail!("missing PE signature at offset {lfanew:#x}"),
    }

    // COFF file header (20 bytes) at lfanew + 4.
    let coff = pe_sig_end;
    let number_of_sections = read_u16(data, coff + 2)? as usize;
    let size_of_optional_header = read_u16(data, coff + 16)? as usize;
    let optional_off = coff + 20;

    let magic = read_u16(data, optional_off)?;
    let (dir_count_off, dir_base) = match magic {
        0x10B => (optional_off + 92, optional_off + 96), // PE32
        0x20B => (optional_off + 108, optional_off + 112), // PE32+
        other => bail!("unsupported PE optional header magic {other:#06X}"),
    };
    if number_of_sections == 0 {
        bail!("PE file declares zero sections");
    }
    if number_of_sections > 4096 {
        bail!("implausible PE section count {number_of_sections}");
    }

    let dir_count = read_u32(data, dir_count_off)?;
    if dir_count <= 2 {
        bail!("PE file declares no resource data directory");
    }
    // Data directory entry 2 is IMAGE_DIRECTORY_ENTRY_RESOURCE.
    let resource_rva = read_u32(data, dir_base + 16)?;
    if resource_rva == 0 {
        bail!("PE file has a null resource data directory RVA");
    }

    let section_table = optional_off
        .checked_add(size_of_optional_header)
        .ok_or_else(|| anyhow!("PE optional header size overflow"))?;

    let mut sections = Vec::with_capacity(number_of_sections);
    for i in 0..number_of_sections {
        let p = section_table
            .checked_add(i * 40)
            .ok_or_else(|| anyhow!("PE section table offset overflow"))?;
        let name_slice = data
            .get(p..p + 8)
            .ok_or_else(|| anyhow!("PE section header {i} lies past end of file"))?;
        let mut name = [0u8; 8];
        name.copy_from_slice(name_slice);
        sections.push(Section {
            name,
            virtual_address: read_u32(data, p + 12)?,
            size_of_raw_data: read_u32(data, p + 16)?,
            pointer_to_raw_data: read_u32(data, p + 20)?,
        });
    }

    Ok(PeLayout {
        sections,
        resource_rva,
    })
}

/// The `.rsrc` section plus the location of the resource directory root.
///
/// Directory and entry `Offset` fields are relative to the **resource
/// directory root** (the `DataDirectory[RESOURCE]` RVA), which is not
/// necessarily the first byte of the section.
#[derive(Clone, Copy)]
struct RsrcSection {
    /// File offset of the section's raw data.
    file_base: usize,
    /// Relative virtual address of the section.
    va: u32,
    /// File offset of the resource directory root inside the section.
    root: usize,
}

#[derive(Clone, Copy)]
struct ResEntry {
    named: bool,
    id: u32,
    is_dir: bool,
    offset: u32,
}

/// Read one `IMAGE_RESOURCE_DIRECTORY` (plus its entries). `dir_off` is
/// relative to the resource directory root, per PE semantics.
fn read_resource_dir(data: &[u8], root_base: usize, dir_off: usize) -> Result<Vec<ResEntry>> {
    let dir = root_base
        .checked_add(dir_off)
        .ok_or_else(|| anyhow!("resource directory offset overflow"))?;
    let named = read_u16(data, dir + 12)? as usize;
    let ids = read_u16(data, dir + 14)? as usize;
    let total = named + ids;
    let entries_off = dir + 16;
    let need = entries_off
        .checked_add(
            total
                .checked_mul(8)
                .ok_or_else(|| anyhow!("resource entry count overflow"))?,
        )
        .ok_or_else(|| anyhow!("resource directory size overflow"))?;
    if need > data.len() {
        bail!(
            "resource directory exceeds file size (needs {need} bytes, file has {})",
            data.len()
        );
    }

    let mut out = Vec::with_capacity(total);
    for i in 0..total {
        let e = entries_off + i * 8;
        let name = read_u32(data, e)?;
        let off = read_u32(data, e + 4)?;
        out.push(ResEntry {
            named: name & 0x8000_0000 != 0,
            id: name & 0x7FFF_FFFF,
            is_dir: off & 0x8000_0000 != 0,
            offset: off & 0x7FFF_FFFF,
        });
    }
    Ok(out)
}

/// Resolve `(file_offset, size)` of the resource data for `type_id`/`res_id`.
fn find_resource_data(
    data: &[u8],
    sec: &RsrcSection,
    type_id: u32,
    res_id: u32,
) -> Result<Option<(usize, usize)>> {
    let root = read_resource_dir(data, sec.root, 0)?;
    let Some(type_entry) = root
        .iter()
        .find(|e| !e.named && e.id == type_id && e.is_dir)
    else {
        return Ok(None);
    };

    let level1 = read_resource_dir(data, sec.root, type_entry.offset as usize)?;
    let Some(name_entry) = level1.iter().find(|e| !e.named && e.id == res_id) else {
        return Ok(None);
    };

    // Language level (usually a subdirectory; tolerate direct data entries).
    let data_entry_off = if name_entry.is_dir {
        let level2 = read_resource_dir(data, sec.root, name_entry.offset as usize)?;
        let Some(lang_entry) = level2.iter().find(|e| !e.is_dir) else {
            return Ok(None);
        };
        lang_entry.offset as usize
    } else {
        name_entry.offset as usize
    };

    // IMAGE_RESOURCE_DATA_ENTRY: OffsetToData(RVA), Size, CodePage, Reserved.
    let entry_pos = sec
        .root
        .checked_add(data_entry_off)
        .ok_or_else(|| anyhow!("resource data entry offset overflow"))?;
    let rva = read_u32(data, entry_pos)?;
    let size = read_u32(data, entry_pos + 4)? as usize;
    let size = cap_icon_size(size)?;

    if rva < sec.va {
        bail!("resource RVA {rva:#x} lies before .rsrc ({:#x})", sec.va);
    }
    let delta = (rva - sec.va) as usize;
    let file_off = sec
        .file_base
        .checked_add(delta)
        .ok_or_else(|| anyhow!("resource file offset overflow"))?;
    let end = file_off
        .checked_add(size)
        .ok_or_else(|| anyhow!("resource end offset overflow"))?;
    if end > data.len() {
        bail!(
            "resource data ({size} bytes at {file_off:#x}) extends past end of file ({})",
            data.len()
        );
    }
    Ok(Some((file_off, size)))
}

/// `GRPICONDIRENTRY`: width, height, colorCount, reserved, planes(u16),
/// bitCount(u16), bytesInRes(u32), id(u16).
struct GroupEntry {
    width: u32,
    height: u32,
    bytes_in_res: u32,
    id: u16,
}

fn parse_group_icon(bytes: &[u8]) -> Result<Vec<GroupEntry>> {
    if bytes.len() < 6 {
        bail!("group icon resource too small ({} bytes)", bytes.len());
    }
    let reserved = read_u16(bytes, 0)?;
    let kind = read_u16(bytes, 2)?;
    let count = read_u16(bytes, 4)? as usize;
    if reserved != 0 || kind != 1 {
        bail!("invalid GRPICONDIR header (reserved={reserved}, type={kind})");
    }
    if count == 0 {
        bail!("group icon resource lists no entries");
    }
    let need = 6usize
        .checked_add(
            count
                .checked_mul(14)
                .ok_or_else(|| anyhow!("group icon count overflow"))?,
        )
        .ok_or_else(|| anyhow!("group icon directory size overflow"))?;
    if need > bytes.len() {
        bail!(
            "group icon directory truncated: {count} entries need {need} bytes, resource has {}",
            bytes.len()
        );
    }

    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let p = 6 + i * 14;
        out.push(GroupEntry {
            width: ico_dim(read_u8(bytes, p)?),
            height: ico_dim(read_u8(bytes, p + 1)?),
            bytes_in_res: read_u32(bytes, p + 8)?,
            id: read_u16(bytes, p + 12)?,
        });
    }
    Ok(out)
}

/// Extract and decode the best icon from a PE image's resources.
fn extract_pe_icon(data: &[u8]) -> Result<RgbaImage> {
    let layout = parse_pe_sections(data)?;
    let resource_rva = layout.resource_rva;
    let sections = &layout.sections;
    let rsrc = sections
        .iter()
        .find(|s| {
            resource_rva >= s.virtual_address
                && resource_rva < s.virtual_address.saturating_add(s.size_of_raw_data.max(1))
        })
        .or_else(|| sections.iter().find(|s| section_name_is(&s.name, ".rsrc")))
        .ok_or_else(|| anyhow!("PE file has no section containing the resource directory"))?;
    if rsrc.pointer_to_raw_data == 0 || rsrc.size_of_raw_data == 0 {
        bail!("PE .rsrc section has no raw data");
    }

    let file_base = rsrc.pointer_to_raw_data as usize;
    if file_base >= data.len() {
        bail!("PE .rsrc section lies past end of file");
    }

    // The resource directory root lives at the section-relative offset of the
    // DataDirectory RVA; entry offsets are relative to it, not the section.
    let sec_va = rsrc.virtual_address;
    let root_delta = resource_rva.checked_sub(sec_va).ok_or_else(|| {
        anyhow!("resource directory RVA {resource_rva:#x} lies before section {sec_va:#x}")
    })? as usize;
    let root = file_base
        .checked_add(root_delta)
        .ok_or_else(|| anyhow!("resource directory offset overflow"))?;
    if root >= data.len() {
        bail!("resource directory lies past end of file");
    }

    let sec = RsrcSection {
        file_base,
        va: sec_va,
        root,
    };

    let root =
        read_resource_dir(data, sec.root, 0).context("failed to read PE resource directory")?;

    // Prefer RT_GROUP_ICON: it tells us which RT_ICON is the best size.
    let group_ids: Vec<u32> = match root
        .iter()
        .find(|e| !e.named && e.id == RT_GROUP_ICON && e.is_dir)
    {
        Some(te) => read_resource_dir(data, sec.root, te.offset as usize)?
            .iter()
            .filter(|e| !e.named)
            .map(|e| e.id)
            .collect(),
        None => Vec::new(),
    };

    for group_id in &group_ids {
        let Some((off, size)) = find_resource_data(data, &sec, RT_GROUP_ICON, *group_id)? else {
            continue;
        };
        let group_bytes = data
            .get(off..off + size)
            .ok_or_else(|| anyhow!("group icon resource lies past end of file"))?;
        let entries = parse_group_icon(group_bytes)
            .with_context(|| format!("failed to parse RT_GROUP_ICON id {group_id}"))?;
        let dims: Vec<(u32, u32)> = entries.iter().map(|e| (e.width, e.height)).collect();
        let Some(best) = choose_best_index(&dims) else {
            continue;
        };
        let entry = &entries[best];

        let Some((icon_off, icon_size)) = find_resource_data(data, &sec, RT_ICON, entry.id as u32)?
        else {
            continue;
        };
        let wanted = if entry.bytes_in_res > 0 {
            (entry.bytes_in_res as usize).min(icon_size)
        } else {
            icon_size
        };
        let wanted = cap_icon_size(wanted)?;
        let icon_bytes = data
            .get(icon_off..icon_off + wanted)
            .ok_or_else(|| anyhow!("RT_ICON resource lies past end of file"))?;
        return decode_icon_resource(icon_bytes)
            .with_context(|| format!("failed to decode RT_ICON resource id {}", entry.id));
    }

    // Fallback: no usable group directory — take the largest RT_ICON.
    if let Some(te) = root
        .iter()
        .find(|e| !e.named && e.id == RT_ICON && e.is_dir)
    {
        let icons = read_resource_dir(data, sec.root, te.offset as usize)?;
        let mut best: Option<(usize, usize)> = None;
        for entry in icons.iter().filter(|e| !e.named) {
            if let Some((off, size)) = find_resource_data(data, &sec, RT_ICON, entry.id)?
                && best.is_none_or(|(_, best_size)| size > best_size)
            {
                best = Some((off, size));
            }
        }
        if let Some((off, size)) = best {
            let icon_bytes = data
                .get(off..off + size)
                .ok_or_else(|| anyhow!("RT_ICON resource lies past end of file"))?;
            return decode_icon_resource(icon_bytes).context("failed to decode RT_ICON resource");
        }
    }

    bail!("PE file contains no usable icon resources (RT_GROUP_ICON / RT_ICON)")
}

// ---------------------------------------------------------------------------
// Icon payload decoding: PNG or DIB
// ---------------------------------------------------------------------------

/// Decode one `RT_ICON` / ICO frame payload: an embedded PNG, or a DIB.
fn decode_icon_resource(bytes: &[u8]) -> Result<RgbaImage> {
    if bytes.len() >= 8 && bytes[..8] == PNG_MAGIC {
        let image = image::load_from_memory_with_format(bytes, ImageFormat::Png)
            .context("failed to decode PNG payload inside icon resource")?;
        return Ok(image.to_rgba8());
    }
    decode_dib(bytes)
}

/// Row stride, in bytes, of a DIB XOR bitmap.
fn dib_row_stride(width: u32, bit_count: u16) -> Result<usize> {
    let w = width as usize;
    let stride = match bit_count {
        32 => w.checked_mul(4),
        24 => w.checked_mul(3).map(|v| (v + 3) & !3),
        8 => Some((w + 3) & !3),
        4 => Some((w.div_ceil(2) + 3) & !3),
        1 => Some((w.div_ceil(8) + 3) & !3),
        other => bail!("unsupported DIB bit depth {other}"),
    };
    stride
        .ok_or_else(|| anyhow!("DIB row stride overflow for width {width}, bit depth {bit_count}"))
}

/// Decode a `BITMAPINFOHEADER`-based DIB (as stored in a `.ico` frame or an
/// `RT_ICON` resource) into an RGBA image.
fn decode_dib(bytes: &[u8]) -> Result<RgbaImage> {
    if bytes.len() < 40 {
        bail!("DIB resource too small ({} bytes)", bytes.len());
    }
    let bi_size = read_u32(bytes, 0)? as usize;
    if bi_size < 40 {
        bail!("unsupported DIB header size {bi_size} (need BITMAPINFOHEADER)");
    }
    if bi_size > bytes.len() {
        bail!(
            "DIB header size {bi_size} exceeds resource length {}",
            bytes.len()
        );
    }

    let bi_width = read_i32(bytes, 4)?;
    let bi_height = read_i32(bytes, 8)?;
    if bi_width <= 0 {
        bail!("DIB has non-positive width {bi_width}");
    }
    let width = bi_width as u32;

    // In an icon DIB the height covers XOR + AND masks, so it is 2x the real
    // height. The combined value is always positive here.
    let combined = bi_height.unsigned_abs();
    if combined < 2 {
        bail!("DIB height {bi_height} is too small");
    }
    let height = combined / 2;
    if height == 0 {
        bail!("DIB has zero image height");
    }

    let planes = read_u16(bytes, 12)?;
    if planes != 1 {
        bail!("DIB has unsupported plane count {planes}");
    }
    let bit_count = read_u16(bytes, 14)?;
    let compression = read_u32(bytes, 16)?;
    if compression != 0 {
        bail!("DIB compression {compression} is not BI_RGB (0)");
    }
    let clr_used = read_u32(bytes, 32)? as usize;

    // Palette (only meaningful for paletted depths).
    let palette_entries = match bit_count {
        1 | 4 | 8 => {
            let n = if clr_used != 0 {
                clr_used
            } else {
                1usize << bit_count
            };
            n.min(256)
        }
        _ => 0,
    };
    let palette_off = bi_size;
    let palette_bytes = palette_entries
        .checked_mul(4)
        .ok_or_else(|| anyhow!("DIB palette size overflow"))?;
    let xor_off = palette_off
        .checked_add(palette_bytes)
        .ok_or_else(|| anyhow!("DIB palette offset overflow"))?;

    let row_stride = dib_row_stride(width, bit_count)?;
    let xor_len = row_stride
        .checked_mul(height as usize)
        .ok_or_else(|| anyhow!("DIB pixel data size overflow"))?;
    let xor_end = xor_off
        .checked_add(xor_len)
        .ok_or_else(|| anyhow!("DIB pixel data offset overflow"))?;
    if xor_end > bytes.len() {
        bail!(
            "DIB pixel data truncated: needs {xor_end} bytes, resource has {}",
            bytes.len()
        );
    }

    // AND mask: 1 bit per pixel, rows padded to 4 bytes.
    let and_row = (width as usize).div_ceil(32) * 4;
    let and_off = xor_end;
    let and_len = and_row
        .checked_mul(height as usize)
        .ok_or_else(|| anyhow!("DIB mask size overflow"))?;
    let and_end = and_off
        .checked_add(and_len)
        .ok_or_else(|| anyhow!("DIB mask offset overflow"))?;
    let mask_available = and_end <= bytes.len();
    if !mask_available && bit_count != 32 {
        bail!(
            "DIB AND mask truncated: needs {and_end} bytes, resource has {}",
            bytes.len()
        );
    }

    // Build the palette (stored as 4-byte BGRA entries).
    let mut palette: Vec<[u8; 4]> = Vec::with_capacity(palette_entries);
    for i in 0..palette_entries {
        let p = palette_off + i * 4;
        palette.push([bytes[p + 2], bytes[p + 1], bytes[p], 255]);
    }

    let mut out = RgbaImage::new(width, height);
    for file_row in 0..height as usize {
        let y = height - 1 - file_row as u32;
        let xor_row = xor_off + file_row * row_stride;
        let and_row_off = and_off + file_row * and_row;

        for x in 0..width as usize {
            let (r, g, b, mut a) = match bit_count {
                32 => {
                    let p = xor_row + x * 4;
                    (bytes[p + 2], bytes[p + 1], bytes[p], bytes[p + 3])
                }
                24 => {
                    let p = xor_row + x * 3;
                    (bytes[p + 2], bytes[p + 1], bytes[p], 255)
                }
                8 => {
                    let idx = bytes[xor_row + x] as usize;
                    let c = palette.get(idx).copied().unwrap_or([0, 0, 0, 255]);
                    (c[0], c[1], c[2], 255)
                }
                4 => {
                    let byte = bytes[xor_row + x / 2];
                    let idx = if x % 2 == 0 { byte >> 4 } else { byte & 0x0F } as usize;
                    let c = palette.get(idx).copied().unwrap_or([0, 0, 0, 255]);
                    (c[0], c[1], c[2], 255)
                }
                1 => {
                    let byte = bytes[xor_row + x / 8];
                    let idx = ((byte >> (7 - (x % 8))) & 1) as usize;
                    let c = palette.get(idx).copied().unwrap_or([0, 0, 0, 255]);
                    (c[0], c[1], c[2], 255)
                }
                _ => unreachable!("bit depth validated by dib_row_stride"),
            };

            if mask_available {
                let mask_byte = bytes[and_row_off + x / 8];
                if (mask_byte >> (7 - (x % 8))) & 1 == 1 {
                    a = 0;
                }
            }

            out.put_pixel(x as u32, y, image::Rgba([r, g, b, a]));
        }
    }

    Ok(out)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageEncoder, Rgba};

    /// Scratch file inside `TMPDIR` (kept writable in this sandbox).
    fn temp_file(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rtx_fg_pe_icons_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir.join(name)
    }

    /// 128x128 checkerboard: block (0,0) is dark with alpha 0, so pixel (2,2) is
    /// transparent; (10,2) falls in a light block.
    fn checkerboard_128() -> RgbaImage {
        let mut img = RgbaImage::new(128, 128);
        for y in 0..128u32 {
            for x in 0..128u32 {
                let light = ((x / 8) + (y / 8)) % 2 == 1;
                img.put_pixel(
                    x,
                    y,
                    if light {
                        Rgba([200, 200, 200, 255])
                    } else {
                        Rgba([10, 10, 10, 0])
                    },
                );
            }
        }
        img
    }

    fn half_dib(width: u32, height: u32, bit_count: u16, clr_used: u32) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(&40u32.to_le_bytes()); // biSize
        d.extend_from_slice(&(width as i32).to_le_bytes()); // biWidth
        d.extend_from_slice(&((height * 2) as i32).to_le_bytes()); // biHeight = 2x
        d.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
        d.extend_from_slice(&bit_count.to_le_bytes()); // biBitCount
        d.extend_from_slice(&0u32.to_le_bytes()); // biCompression = BI_RGB
        d.extend_from_slice(&0u32.to_le_bytes()); // biSizeImage
        d.extend_from_slice(&0i32.to_le_bytes()); // biXPelsPerMeter
        d.extend_from_slice(&0i32.to_le_bytes()); // biYPelsPerMeter
        d.extend_from_slice(&clr_used.to_le_bytes()); // biClrUsed
        d.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant
        d
    }

    #[test]
    fn standalone_ico_checkerboard_roundtrip() -> Result<()> {
        let img = checkerboard_128();
        let frame = image::codecs::ico::IcoFrame::as_png(
            img.as_raw(),
            128,
            128,
            image::ExtendedColorType::Rgba8,
        )?;

        let path = temp_file("checkerboard.ico");
        image::codecs::ico::IcoEncoder::new(std::fs::File::create(&path)?)
            .encode_images(&[frame])?;

        let out = extract(&path)?;
        let _ = std::fs::remove_file(&path);

        assert_eq!(out.dimensions(), (128, 128));
        assert_eq!(
            out.get_pixel(2, 2).0[3],
            0,
            "corner (2,2) must be transparent"
        );

        let light = out.get_pixel(10, 2).0;
        assert!(
            light[..3].iter().all(|&c| c >= 150),
            "light pixel should be bright, got {light:?}"
        );

        let dark = out.get_pixel(2, 2).0;
        assert!(
            dark[..3].iter().all(|&c| c <= 60),
            "dark pixel should be dark, got {dark:?}"
        );
        Ok(())
    }

    /// A 256x256 frame must not shadow the native 128x128 frame.
    #[test]
    fn standalone_ico_prefers_128_frame() -> Result<()> {
        let big = RgbaImage::from_pixel(256, 256, Rgba([0, 255, 0, 255]));
        let small = RgbaImage::from_pixel(128, 128, Rgba([255, 0, 0, 255]));
        let frames = [
            image::codecs::ico::IcoFrame::as_png(
                big.as_raw(),
                256,
                256,
                image::ExtendedColorType::Rgba8,
            )?,
            image::codecs::ico::IcoFrame::as_png(
                small.as_raw(),
                128,
                128,
                image::ExtendedColorType::Rgba8,
            )?,
        ];

        let path = temp_file("multi.ico");
        image::codecs::ico::IcoEncoder::new(std::fs::File::create(&path)?)
            .encode_images(&frames)?;
        let out = extract(&path)?;
        let _ = std::fs::remove_file(&path);

        assert_eq!(out.dimensions(), (128, 128));
        let center = out.get_pixel(64, 64).0;
        assert!(
            center[0] > 240 && center[1] < 15,
            "expected the red 128x128 frame, got {center:?}"
        );
        Ok(())
    }

    #[test]
    fn missing_file_returns_error() {
        let path = temp_file("definitely_missing.exe");
        let _ = std::fs::remove_file(&path);
        assert!(extract(&path).is_err());
    }

    #[test]
    fn non_pe_and_non_ico_return_errors() -> Result<()> {
        let path = temp_file("not_an_image.exe");
        std::fs::write(&path, b"definitely not a PE image\n".repeat(8))?;
        let err = extract(&path).unwrap_err();
        assert!(
            format!("{err:#}").contains("neither a PE executable"),
            "{err:#}"
        );
        let _ = std::fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn dib_32bit_decodes_with_alpha_and_mask() -> Result<()> {
        let mut d = half_dib(2, 2, 32, 0);
        // XOR bitmap is bottom-up: file row 0 is the bottom image row.
        d.extend_from_slice(&[255, 0, 0, 255]); // bottom-left: blue
        d.extend_from_slice(&[255, 255, 255, 255]); // bottom-right: white
        d.extend_from_slice(&[0, 0, 255, 255]); // top-left: red
        d.extend_from_slice(&[0, 255, 0, 128]); // top-right: green, alpha 128
        // AND mask, 2 rows x 4 bytes (stride padded); bit 6 of row 0 is the
        // bottom-right pixel -> forced transparent.
        d.extend_from_slice(&[0x40, 0, 0, 0]);
        d.extend_from_slice(&[0, 0, 0, 0]);

        let out = decode_dib(&d)?;
        assert_eq!(out.dimensions(), (2, 2));
        assert_eq!(out.get_pixel(0, 0).0, [255, 0, 0, 255]); // top-left red
        assert_eq!(out.get_pixel(1, 0).0, [0, 255, 0, 128]); // top-right green + alpha
        assert_eq!(out.get_pixel(0, 1).0, [0, 0, 255, 255]); // bottom-left blue
        assert_eq!(out.get_pixel(1, 1).0, [255, 255, 255, 0]); // AND mask wins
        Ok(())
    }

    #[test]
    fn dib_24bit_row_padding_and_mask() -> Result<()> {
        let mut d = half_dib(2, 2, 24, 0);
        // Bottom row (green) with 2 bytes of padding, then the top row (red, blue).
        d.extend_from_slice(&[0, 255, 0, 0, 255, 0, 0, 0]);
        d.extend_from_slice(&[0, 0, 255, 255, 0, 0, 0, 0]);
        // AND mask: bottom row clear, top row has bit 6 set at x=1.
        d.extend_from_slice(&[0, 0, 0, 0]);
        d.extend_from_slice(&[0x40, 0, 0, 0]);

        let out = decode_dib(&d)?;
        assert_eq!(out.get_pixel(0, 0).0, [255, 0, 0, 255]); // top-left red
        assert_eq!(out.get_pixel(1, 0).0, [0, 0, 255, 0]); // masked transparent
        assert_eq!(out.get_pixel(0, 1).0, [0, 255, 0, 255]); // bottom-left green
        Ok(())
    }

    #[test]
    fn dib_8bit_paletted_decodes() -> Result<()> {
        let mut d = half_dib(2, 2, 8, 2);
        // Palette entries are 4-byte BGRA; entry 0 -> red, entry 1 -> green.
        d.extend_from_slice(&[0, 0, 255, 0]);
        d.extend_from_slice(&[0, 255, 0, 0]);
        // XOR rows, stride 4: bottom = [idx0, idx1], top = [idx1, idx0].
        d.extend_from_slice(&[0, 1, 0, 0]);
        d.extend_from_slice(&[1, 0, 0, 0]);
        // AND mask rows, all clear.
        d.extend_from_slice(&[0, 0, 0, 0]);
        d.extend_from_slice(&[0, 0, 0, 0]);

        let out = decode_dib(&d)?;
        assert_eq!(out.get_pixel(0, 0).0, [0, 255, 0, 255]); // top-left green
        assert_eq!(out.get_pixel(1, 0).0, [255, 0, 0, 255]); // top-right red
        assert_eq!(out.get_pixel(0, 1).0, [255, 0, 0, 255]); // bottom-left red
        assert_eq!(out.get_pixel(1, 1).0, [0, 255, 0, 255]); // bottom-right green
        Ok(())
    }

    #[test]
    fn dib_1bit_paletted_decodes() -> Result<()> {
        let mut d = half_dib(8, 1, 1, 2);
        d.extend_from_slice(&[0, 0, 255, 0]); // entry 0 -> red
        d.extend_from_slice(&[255, 0, 0, 0]); // entry 1 -> blue
        d.extend_from_slice(&[0b1010_0000, 0, 0, 0]);
        d.extend_from_slice(&[0, 0, 0, 0]); // AND mask

        let out = decode_dib(&d)?;
        assert_eq!(out.get_pixel(0, 0).0, [0, 0, 255, 255]); // bit set -> blue
        assert_eq!(out.get_pixel(1, 0).0, [255, 0, 0, 255]); // clear -> red
        assert_eq!(out.get_pixel(2, 0).0, [0, 0, 255, 255]);
        Ok(())
    }

    #[test]
    fn dib_4bit_paletted_decodes() -> Result<()> {
        let mut d = half_dib(2, 1, 4, 2);
        d.extend_from_slice(&[0, 0, 255, 0]); // entry 0 -> red
        d.extend_from_slice(&[255, 0, 0, 0]); // entry 1 -> blue
        d.extend_from_slice(&[0x10, 0, 0, 0]); // x0 = high nibble 1, x1 = low nibble 0
        d.extend_from_slice(&[0, 0, 0, 0]);

        let out = decode_dib(&d)?;
        assert_eq!(out.get_pixel(0, 0).0, [0, 0, 255, 255]);
        assert_eq!(out.get_pixel(1, 0).0, [255, 0, 0, 255]);
        Ok(())
    }

    #[test]
    fn png_payload_inside_icon_resource() -> Result<()> {
        let img = RgbaImage::from_pixel(16, 16, Rgba([1, 2, 3, 4]));
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png).write_image(
            img.as_raw(),
            16,
            16,
            image::ExtendedColorType::Rgba8,
        )?;

        let out = decode_icon_resource(&png)?;
        assert_eq!(out.dimensions(), (16, 16));
        assert_eq!(out.get_pixel(0, 0).0, [1, 2, 3, 4]);
        Ok(())
    }

    #[test]
    fn malformed_inputs_error_without_panicking() {
        // Truncated / garbage DIBs.
        for len in [0usize, 5, 12, 39, 40, 44, 100] {
            let bytes = vec![0u8; len];
            assert!(decode_dib(&bytes).is_err(), "zeroed DIB of len {len}");
        }

        // 32bpp header claiming a 64x64 image but far too little pixel data.
        let mut d = half_dib(64, 64, 32, 0);
        d.extend_from_slice(&[0u8; 16]);
        assert!(decode_dib(&d).is_err());

        // Unsupported bit depth.
        let mut d = half_dib(2, 2, 16, 0);
        d.extend_from_slice(&[0u8; 64]);
        assert!(decode_dib(&d).is_err());

        // Unsupported compression.
        let mut d = half_dib(2, 2, 32, 0);
        d[16..20].copy_from_slice(&3u32.to_le_bytes()); // BI_BITFIELDS
        assert!(decode_dib(&d).is_err());

        // Frames that extend past the end of the ICO file.
        let mut ico = vec![0u8, 0, 1, 0, 1, 0];
        ico.extend_from_slice(&[128, 128, 0, 0, 1, 0, 32, 0]); // planes/bitcount
        ico.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // bytesInRes (absurd)
        ico.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // imageOffset (absurd)
        assert!(extract_standalone_ico(&ico).is_err());

        // PE with a bogus e_lfanew.
        let mut pe = vec![0u8; 0x100];
        pe[0..2].copy_from_slice(b"MZ");
        pe[0x3C..0x40].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        assert!(extract_pe_icon(&pe).is_err());

        // Truncated PE (valid MZ, no PE signature).
        let mut pe = vec![0u8; 0x100];
        pe[0..2].copy_from_slice(b"MZ");
        pe[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        assert!(extract_pe_icon(&pe).is_err());
    }

    #[test]
    fn oversized_icon_resource_is_rejected() {
        let mut ico = vec![0u8, 0, 1, 0, 1, 0];
        ico.extend_from_slice(&[128, 128, 0, 0, 1, 0, 32, 0]);
        ico.extend_from_slice(&(MAX_ICON_BYTES as u32 + 1).to_le_bytes());
        ico.extend_from_slice(&22u32.to_le_bytes());
        let err = extract_standalone_ico(&ico).unwrap_err();
        assert!(format!("{err:#}").contains("too large"), "{err:#}");
    }
    // -- Synthetic PE32+ with a real .rsrc directory ------------------------

    /// Absolute offsets *within* the .rsrc section. Directory/entry `Offset`
    /// fields are relative to the resource directory root (`ROOT_DIR`).
    const ROOT_DIR: usize = 0x100;
    const GROUP_DIR: usize = 0x050;
    const GROUP_LANG_DIR: usize = 0x080;
    const ICON_DIR: usize = 0x150;
    const ICON_LANG_DIR: usize = 0x180;
    const GROUP_DATA_ENTRY: usize = 0x300;
    const ICON_DATA_ENTRY: usize = 0x310;
    const GROUP_DATA: usize = 0x400;
    const SECTION_VA: u32 = 0x1000;
    const LANG_ID: u32 = 0x0804;

    fn put(buf: &mut [u8], off: usize, bytes: &[u8]) {
        buf[off..off + bytes.len()].copy_from_slice(bytes);
    }

    /// `entries` are `(name/id, child offset)`; offsets relative to ROOT_DIR.
    fn write_dir(rsrc: &mut [u8], off: usize, entries: &[(u32, u32)]) {
        put(rsrc, off, &[0u8; 16]);
        put(rsrc, off + 14, &(entries.len() as u16).to_le_bytes());
        for (i, (name, child)) in entries.iter().enumerate() {
            let e = off + 16 + i * 8;
            put(rsrc, e, &name.to_le_bytes());
            put(rsrc, e + 4, &child.to_le_bytes());
        }
    }

    fn write_data_entry(rsrc: &mut [u8], off: usize, data_rva: u32, size: u32) {
        put(rsrc, off, &data_rva.to_le_bytes());
        put(rsrc, off + 4, &size.to_le_bytes());
        put(rsrc, off + 8, &0u32.to_le_bytes()); // CodePage
        put(rsrc, off + 12, &0u32.to_le_bytes()); // Reserved
    }

    /// One-entry `GRPICONDIR` (6-byte header + one 14-byte entry).
    fn group_icon_data(width: u8, height: u8, bit_count: u16, bytes: u32, id: u16) -> Vec<u8> {
        let mut g = Vec::new();
        g.extend_from_slice(&0u16.to_le_bytes()); // reserved
        g.extend_from_slice(&1u16.to_le_bytes()); // type = icon
        g.extend_from_slice(&1u16.to_le_bytes()); // count
        g.push(width);
        g.push(height);
        g.push(0); // colorCount
        g.push(0); // reserved
        g.extend_from_slice(&1u16.to_le_bytes()); // planes
        g.extend_from_slice(&bit_count.to_le_bytes());
        g.extend_from_slice(&bytes.to_le_bytes());
        g.extend_from_slice(&id.to_le_bytes());
        g
    }

    /// 16x16 32bpp DIB. Top-left 4x4 reads red, everywhere else blue, and the
    /// pixel at (15,15) has alpha 0. `screen_pixel` returns BGRA.
    fn icon_16x16_dib(with_mask: bool) -> Vec<u8> {
        let w = 16u32;
        let h = 16u32;
        let mut d = half_dib(w, h, 32, 0);
        for row in 0..h {
            let screen_row = h - 1 - row;
            for x in 0..w {
                // The whole bottom-right quadrant (screen 8..15) is
                // transparent: it is far larger than Lanczos3's support, so
                // samples deep inside stay exactly alpha 0 after 8x scaling.
                let alpha = if x >= 8 && screen_row >= 8 { 0 } else { 255 };
                if x < 4 && screen_row < 4 {
                    d.extend_from_slice(&[0, 0, 255, alpha]); // red
                } else {
                    d.extend_from_slice(&[255, 0, 0, alpha]); // blue
                }
            }
        }
        if with_mask {
            let stride = (w.div_ceil(32) * 4) as usize;
            d.extend(std::iter::repeat_n(0u8, stride * h as usize));
        }
        d
    }

    /// Structurally valid PE32+ with one `.rsrc` section holding an
    /// `RT_GROUP_ICON` + `RT_ICON` pair.
    fn synthetic_pe(group: &[u8], icon: &[u8], icon_size: u32) -> Vec<u8> {
        let group_abs = ROOT_DIR + GROUP_DATA;
        let icon_abs = (group_abs + group.len() + 3) & !3;
        let rsrc_span = ((icon_abs + icon.len() + 3) & !3).max(0x800);

        let mut rsrc = vec![0u8; rsrc_span];
        write_dir(
            &mut rsrc,
            ROOT_DIR,
            &[
                (3, 0x8000_0000 | GROUP_DIR as u32), // RT_GROUP_ICON
                (14, 0x8000_0000 | ICON_DIR as u32), // RT_ICON
            ],
        );
        write_dir(
            &mut rsrc,
            ROOT_DIR + GROUP_DIR,
            &[(3, 0x8000_0000 | GROUP_LANG_DIR as u32)],
        );
        write_dir(
            &mut rsrc,
            ROOT_DIR + GROUP_LANG_DIR,
            &[(LANG_ID, GROUP_DATA_ENTRY as u32)],
        );
        write_dir(
            &mut rsrc,
            ROOT_DIR + ICON_DIR,
            &[(1, 0x8000_0000 | ICON_LANG_DIR as u32)],
        );
        write_dir(
            &mut rsrc,
            ROOT_DIR + ICON_LANG_DIR,
            &[(LANG_ID, ICON_DATA_ENTRY as u32)],
        );
        write_data_entry(
            &mut rsrc,
            ROOT_DIR + GROUP_DATA_ENTRY,
            SECTION_VA + group_abs as u32,
            group.len() as u32,
        );
        write_data_entry(
            &mut rsrc,
            ROOT_DIR + ICON_DATA_ENTRY,
            SECTION_VA + icon_abs as u32,
            icon_size,
        );
        put(&mut rsrc, group_abs, group);
        put(&mut rsrc, icon_abs, icon);

        let pe_offset = 0x80usize;
        let coff = pe_offset + 4;
        let optional = coff + 20;
        let optional_size = 240usize;
        let section_table = optional + optional_size;
        let raw_pointer = (section_table + 40 + 3) & !3;

        let mut pe = vec![0u8; raw_pointer + rsrc_span];
        put(&mut pe, 0, b"MZ");
        put(&mut pe, 0x3C, &(pe_offset as u32).to_le_bytes());
        put(&mut pe, pe_offset, b"PE\0\0");
        put(&mut pe, coff, &0x8664u16.to_le_bytes()); // Machine = x86-64
        put(&mut pe, coff + 2, &1u16.to_le_bytes()); // NumberOfSections
        put(&mut pe, coff + 16, &(optional_size as u16).to_le_bytes());
        put(&mut pe, coff + 18, &0x0022u16.to_le_bytes());
        // PE32+ optional header.
        put(&mut pe, optional, &0x020Bu16.to_le_bytes());
        put(&mut pe, optional + 24, &0x1_4000_0000u64.to_le_bytes()); // ImageBase
        put(&mut pe, optional + 32, &0x1000u32.to_le_bytes()); // SectionAlignment
        put(&mut pe, optional + 36, &0x200u32.to_le_bytes()); // FileAlignment
        put(&mut pe, optional + 56, &0x2000u32.to_le_bytes()); // SizeOfImage
        put(&mut pe, optional + 60, &(raw_pointer as u32).to_le_bytes());
        put(&mut pe, optional + 68, &2u16.to_le_bytes()); // Subsystem = GUI
        put(&mut pe, optional + 108, &16u32.to_le_bytes()); // NumberOfRvaAndSizes
        // Data directory entry 2 = IMAGE_DIRECTORY_ENTRY_RESOURCE.
        put(
            &mut pe,
            optional + 112 + 16,
            &(SECTION_VA + ROOT_DIR as u32).to_le_bytes(),
        );
        put(
            &mut pe,
            optional + 112 + 20,
            &(rsrc_span as u32).to_le_bytes(),
        );
        // Section header.
        put(&mut pe, section_table, b".rsrc\0\0\0");
        put(
            &mut pe,
            section_table + 8,
            &(rsrc_span as u32).to_le_bytes(),
        );
        put(&mut pe, section_table + 12, &SECTION_VA.to_le_bytes());
        put(
            &mut pe,
            section_table + 16,
            &(rsrc_span as u32).to_le_bytes(),
        );
        put(
            &mut pe,
            section_table + 20,
            &(raw_pointer as u32).to_le_bytes(),
        );
        put(&mut pe, section_table + 36, &0x4000_0040u32.to_le_bytes());
        put(&mut pe, raw_pointer, &rsrc);
        pe
    }

    #[test]
    fn synthetic_pe_extracts_and_scales_icon() -> Result<()> {
        let dib = icon_16x16_dib(true);
        let group = group_icon_data(16, 16, 32, (16 * 16 * 4) as u32, 3);
        let pe = synthetic_pe(&group, &dib, dib.len() as u32);

        let path = temp_file("synthetic.exe");
        std::fs::write(&path, &pe)?;
        let out = extract(&path)?;
        let _ = std::fs::remove_file(&path);

        assert_eq!(out.dimensions(), (128, 128), "PE path must output 128x128");
        // Top-left quadrant is red; if row order were flipped this would be blue.
        let red = out.get_pixel(16, 16).0;
        assert!(
            red[0] > 240 && red[1] < 15,
            "expected red quadrant, got {red:?}"
        );
        let blue = out.get_pixel(96, 96).0;
        assert!(
            blue[2] > 240 && blue[0] < 15,
            "expected blue area, got {blue:?}"
        );
        // The transparent source quadrant (screen 8..15, i.e. output 64..127)
        // must stay transparent after Lanczos scaling; sample deep inside it.
        assert_eq!(
            out.get_pixel(112, 112).0[3],
            0,
            "transparent quadrant must survive scaling"
        );
        Ok(())
    }

    #[test]
    fn synthetic_pe_without_and_mask_still_decodes() -> Result<()> {
        let dib = icon_16x16_dib(false);
        let group = group_icon_data(16, 16, 32, (16 * 16 * 4) as u32, 3);
        let pe = synthetic_pe(&group, &dib, dib.len() as u32);

        let path = temp_file("nomask.exe");
        std::fs::write(&path, &pe)?;
        let out = extract(&path)?;
        let _ = std::fs::remove_file(&path);

        assert_eq!(out.dimensions(), (128, 128));
        assert_eq!(out.get_pixel(16, 16).0[3], 255, "red quadrant opaque");
        Ok(())
    }

    #[test]
    fn pe_with_oversized_icon_size_errors() -> Result<()> {
        let dib = icon_16x16_dib(true);
        let group = group_icon_data(16, 16, 32, (16 * 16 * 4) as u32, 3);
        // bytesInRes claims far more data than the file actually holds.
        let pe = synthetic_pe(&group, &dib, 0x00FF_FFFF);
        let path = temp_file("bad_size.exe");
        std::fs::write(&path, &pe)?;
        let result = extract(&path);
        let _ = std::fs::remove_file(&path);
        assert!(result.is_err(), "oversized resource must error");
        Ok(())
    }

    /// Truncating the synthetic PE at many offsets must never panic.
    #[test]
    fn truncated_synthetic_pe_never_panics() {
        let dib = icon_16x16_dib(true);
        let group = group_icon_data(16, 16, 32, (16 * 16 * 4) as u32, 3);
        let pe = synthetic_pe(&group, &dib, dib.len() as u32);

        for cut in (0..pe.len()).step_by(37) {
            let slice = &pe[..cut];
            let _ = std::panic::catch_unwind(|| extract_pe_icon(slice));
        }
        // Also poke the section/dir readers directly at every truncated length
        // of the first 1 KiB (header + directory region).
        for cut in 0..pe.len().min(0x1200) {
            let slice = &pe[..cut];
            let _ = std::panic::catch_unwind(|| parse_pe_sections(slice));
        }
    }
}
