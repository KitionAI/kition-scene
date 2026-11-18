//! USDZ packages: uncompressed zip archives whose files' data starts on 64-byte boundaries, the
//! first file the root layer.

use kition_scene_model::SceneError;

use crate::usda::invalid;

/// Where every file's data starts a multiple of.
const ALIGNMENT: usize = 64;
/// The extra field id that pads a local header so its data is aligned.
const PADDING_ID: u16 = 0x1986;
/// More files than this in a package are refused.
const MAX_FILES: usize = 65_535;

/// The CRC-32 (IEEE) of `bytes`.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn push16(out: &mut Vec<u8>, value: usize) -> Result<(), SceneError> {
    out.extend(u16::try_from(value).map_err(|_| invalid("a package field is too large"))?.to_le_bytes());
    Ok(())
}

fn push32(out: &mut Vec<u8>, value: usize) -> Result<(), SceneError> {
    out.extend(u32::try_from(value).map_err(|_| invalid("the package is too large"))?.to_le_bytes());
    Ok(())
}

/// A package of `files` (path, bytes), in order: stored, each file's data 64-byte aligned.
pub fn write(files: &[(String, &[u8])]) -> Result<Vec<u8>, SceneError> {
    if files.len() > MAX_FILES {
        return Err(invalid("too many files for a package"));
    }
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let offset = out.len();
        let crc = crc32(data);
        // Pad the extra field so the data after the header starts aligned (an extra field
        // record is at least 4 bytes).
        let header = 30 + name.len();
        let mut padding = (ALIGNMENT - (offset + header + 4) % ALIGNMENT) % ALIGNMENT;
        padding += 4;
        out.extend(0x0403_4b50u32.to_le_bytes());
        for value in [20, 0, 0, 0, 0x21] {
            push16(&mut out, value)?;
        }
        out.extend(crc.to_le_bytes());
        push32(&mut out, data.len())?;
        push32(&mut out, data.len())?;
        push16(&mut out, name.len())?;
        push16(&mut out, padding)?;
        out.extend(name.as_bytes());
        out.extend(PADDING_ID.to_le_bytes());
        push16(&mut out, padding - 4)?;
        out.resize(out.len() + padding - 4, 0);
        debug_assert_eq!(out.len() % ALIGNMENT, 0);
        out.extend_from_slice(data);
        central.extend(0x0201_4b50u32.to_le_bytes());
        for value in [20, 20, 0, 0, 0, 0x21] {
            push16(&mut central, value)?;
        }
        central.extend(crc.to_le_bytes());
        push32(&mut central, data.len())?;
        push32(&mut central, data.len())?;
        push16(&mut central, name.len())?;
        for _ in 0..4 {
            push16(&mut central, 0)?;
        }
        push32(&mut central, 0)?;
        push32(&mut central, offset)?;
        central.extend(name.as_bytes());
    }
    let start = out.len();
    out.extend(&central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    for value in [0, 0, files.len(), files.len()] {
        push16(&mut out, value)?;
    }
    push32(&mut out, central.len())?;
    push32(&mut out, start)?;
    push16(&mut out, 0)?;
    Ok(out)
}

/// The stored files of a package, in order, read through their local headers (compressed files
/// are left out; a package must not compress its files).
pub fn read(bytes: &[u8]) -> Result<Vec<(String, &[u8])>, SceneError> {
    let field = |at: usize, width: usize| -> Result<usize, SceneError> {
        let raw = bytes.get(at..at + width).ok_or_else(|| invalid("a damaged USDZ package"))?;
        Ok(raw.iter().rev().fold(0usize, |value, byte| (value << 8) | usize::from(*byte)))
    };
    let mut files = Vec::new();
    let mut at = 0usize;
    while bytes.get(at..at + 4) == Some(b"PK\x03\x04".as_slice()) {
        if files.len() >= MAX_FILES {
            return Err(invalid("a USDZ package with too many files"));
        }
        let flags = field(at + 6, 2)?;
        let method = field(at + 8, 2)?;
        let size = field(at + 18, 4)?;
        let name_length = field(at + 26, 2)?;
        let extra = field(at + 28, 2)?;
        // Sizes after the data (flag bit 3) cannot be walked without the central directory.
        if flags & 8 != 0 {
            return Err(invalid("a USDZ package with streamed files"));
        }
        let name = bytes.get(at + 30..at + 30 + name_length).ok_or_else(|| invalid("a damaged USDZ package"))?;
        let start = at + 30 + name_length + extra;
        let data = bytes
            .get(start..start.checked_add(size).ok_or_else(|| invalid("a damaged USDZ package"))?)
            .ok_or_else(|| invalid("a damaged USDZ package"))?;
        if method == 0 {
            files.push((String::from_utf8_lossy(name).into_owned(), data));
        } else if files.is_empty() {
            return Err(invalid("a USDZ package whose files are compressed"));
        }
        at = start + size;
    }
    if files.is_empty() {
        return Err(invalid("a damaged USDZ package"));
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_the_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn files_are_aligned_and_read_back() {
        let files = vec![("scene.usda".to_string(), b"#usda 1.0\n".as_slice()), ("textures/a.png".to_string(), [1u8, 2, 3].as_slice())];
        let package = write(&files).unwrap();
        let back = read(&package).unwrap();
        assert_eq!(back, files);
        for (_, data) in back {
            let offset = data.as_ptr() as usize - package.as_ptr() as usize;
            assert_eq!(offset % ALIGNMENT, 0);
        }
    }
}
