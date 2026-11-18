//! The container: the header, file blocks, and the structure catalogue (SDNA) that describes
//! every struct the file stores, so fields are found by name and never by fixed offsets.

use std::collections::HashMap;
use std::io::Read;

use kition_scene_model::SceneError;

/// The largest file read, after decompression (512 MiB).
const MAX_SIZE: u64 = 512 * 1024 * 1024;

pub fn invalid(message: impl Into<String>) -> SceneError {
    SceneError::InvalidFile(message.into())
}

/// One file block: a struct (or an array of them, or raw bytes) the file stored.
#[derive(Debug, Clone)]
pub struct Block {
    /// The block's position in the file's block list.
    pub index: usize,
    pub code: [u8; 4],
    /// The index of the struct in the catalogue.
    pub sdna: usize,
    /// The address the block had in memory when written; pointers name blocks by it.
    pub old: u64,
    pub start: usize,
    pub len: usize,
    pub count: usize,
}

/// One field of a struct.
#[derive(Debug, Clone)]
pub struct Field {
    pub type_name: String,
    pub offset: usize,
    pub pointer: bool,
    /// Elements in an array field (1 for a plain field).
    pub count: usize,
    /// The size of one element.
    pub size: usize,
}

#[derive(Debug, Clone)]
pub struct StructInfo {
    pub name: String,
    pub size: usize,
    pub fields: HashMap<String, Field>,
}

/// A whole file, decompressed, with its blocks and catalogue.
pub struct File {
    pub data: Vec<u8>,
    pub blocks: Vec<Block>,
    /// Block indices by old address, in file order (an address can be written more than once).
    by_old: HashMap<u64, Vec<usize>>,
    pub structs: Vec<StructInfo>,
    by_name: HashMap<String, usize>,
}

/// zstd frames one after another (the reference application writes several, then a seek
/// table in a skippable frame).
fn decompress(bytes: &[u8]) -> Result<Vec<u8>, SceneError> {
    let mut out = Vec::new();
    let mut cursor = std::io::Cursor::new(bytes);
    while (cursor.position() as usize) < bytes.len() {
        let at = cursor.position() as usize;
        let magic = bytes
            .get(at..at + 4)
            .and_then(|word| <[u8; 4]>::try_from(word).ok().map(u32::from_le_bytes))
            .ok_or_else(|| invalid("a zstd frame is cut short"))?;
        if (0x184D_2A50..=0x184D_2A5F).contains(&magic) {
            let size = bytes
                .get(at + 4..at + 8)
                .and_then(|word| <[u8; 4]>::try_from(word).ok().map(u32::from_le_bytes))
                .ok_or_else(|| invalid("a skippable frame is cut short"))?;
            cursor.set_position((at + 8) as u64 + u64::from(size));
            continue;
        }
        let mut decoder = ruzstd::decoding::StreamingDecoder::new(&mut cursor).map_err(|error| invalid(format!("not a zstd stream: {error}")))?;
        let room = MAX_SIZE.saturating_sub(out.len() as u64);
        let read =
            (&mut decoder).take(room + 1).read_to_end(&mut out).map_err(|error| invalid(format!("the compressed file is damaged: {error}")))?;
        if out.len() as u64 > MAX_SIZE {
            return Err(invalid("the file is too large"));
        }
        drop(decoder);
        if read == 0 && cursor.position() as usize == at {
            break;
        }
    }
    Ok(out)
}

fn read_u32(data: &[u8], at: usize) -> Option<u32> {
    let bytes = data.get(at..at.checked_add(4)?)?;
    <[u8; 4]>::try_from(bytes).ok().map(u32::from_le_bytes)
}

fn read_u64(data: &[u8], at: usize) -> Option<u64> {
    let bytes = data.get(at..at.checked_add(8)?)?;
    let mut array = [0u8; 8];
    array.copy_from_slice(bytes);
    Some(u64::from_le_bytes(array))
}

fn read_u16(data: &[u8], at: usize) -> Option<u16> {
    let bytes = data.get(at..at.checked_add(2)?)?;
    <[u8; 2]>::try_from(bytes).ok().map(u16::from_le_bytes)
}

impl File {
    pub fn parse(bytes: &[u8]) -> Result<Self, SceneError> {
        if bytes.len() as u64 > MAX_SIZE {
            return Err(invalid("the file is too large"));
        }
        let data = if bytes.starts_with(&[0x28, 0xB5, 0x2F, 0xFD]) { decompress(bytes)? } else { bytes.to_vec() };
        if !data.starts_with(b"BLENDER") {
            return Err(invalid("not a .blend file"));
        }
        // Two header layouts: the classic 12 bytes (`BLENDER-v405`) and the newer one with its
        // own size (`BLENDER17-01v0502`), whose block headers carry 64-bit lengths.
        let (header, large) = match data.get(7) {
            Some(b'-') => (12usize, false),
            Some(b'_') => return Err(invalid("32-bit .blend files are not supported")),
            Some(digit) if digit.is_ascii_digit() => {
                let size = std::str::from_utf8(data.get(7..9).unwrap_or_default())
                    .ok()
                    .and_then(|text| text.parse::<usize>().ok())
                    .ok_or_else(|| invalid("a damaged .blend header"))?;
                if data.get(9) != Some(&b'-') {
                    return Err(invalid("32-bit .blend files are not supported"));
                }
                if data.get(12) != Some(&b'v') {
                    return Err(invalid("big-endian .blend files are not supported"));
                }
                (size, true)
            }
            _ => return Err(invalid("a damaged .blend header")),
        };
        if !large && data.get(8) != Some(&b'v') {
            return Err(invalid("big-endian .blend files are not supported"));
        }
        let mut blocks = Vec::new();
        let mut at = header;
        loop {
            let code = data.get(at..at + 4).ok_or_else(|| invalid("the file ends without its last block"))?;
            let code: [u8; 4] = code.try_into().map_err(|_| invalid("a block header is cut short"))?;
            let (sdna, old, len, count, body) = if large {
                let sdna = read_u32(&data, at + 4).ok_or_else(|| invalid("a block header is cut short"))?;
                let old = read_u64(&data, at + 8).ok_or_else(|| invalid("a block header is cut short"))?;
                let len = read_u64(&data, at + 16).ok_or_else(|| invalid("a block header is cut short"))?;
                let count = read_u64(&data, at + 24).ok_or_else(|| invalid("a block header is cut short"))?;
                (sdna, old, len, count, at + 32)
            } else {
                let len = u64::from(read_u32(&data, at + 4).ok_or_else(|| invalid("a block header is cut short"))?);
                let old = read_u64(&data, at + 8).ok_or_else(|| invalid("a block header is cut short"))?;
                let sdna = read_u32(&data, at + 16).ok_or_else(|| invalid("a block header is cut short"))?;
                let count = u64::from(read_u32(&data, at + 20).ok_or_else(|| invalid("a block header is cut short"))?);
                (sdna, old, len, count, at + 24)
            };
            let len = usize::try_from(len).map_err(|_| invalid("a block is too large"))?;
            let end = body.checked_add(len).filter(|end| *end <= data.len()).ok_or_else(|| invalid("a block runs past the end of the file"))?;
            if &code == b"ENDB" {
                break;
            }
            blocks.push(Block {
                index: blocks.len(),
                code,
                sdna: sdna as usize,
                old,
                start: body,
                len,
                count: usize::try_from(count).unwrap_or(usize::MAX),
            });
            at = end;
            if blocks.len() > 10_000_000 {
                return Err(invalid("the file has too many blocks"));
            }
        }
        let dna = blocks.iter().find(|block| &block.code == b"DNA1").ok_or_else(|| invalid("the file has no structure catalogue"))?;
        let structs = parse_dna(data.get(dna.start..dna.start + dna.len).unwrap_or_default())?;
        let by_name = structs.iter().enumerate().map(|(index, info)| (info.name.clone(), index)).collect();
        let mut by_old: HashMap<u64, Vec<usize>> = HashMap::new();
        for block in blocks.iter().filter(|block| block.old != 0) {
            by_old.entry(block.old).or_default().push(block.index);
        }
        Ok(Self { data, blocks, by_old, structs, by_name })
    }

    /// The block a pointer in block `from` names. The writer may reuse an address for
    /// short-lived buffers, so a pointer means the first block with that address at or after
    /// the block holding the pointer (data is written right after what refers to it), else the
    /// last one before it.
    pub fn block(&self, pointer: u64, from: usize) -> Option<&Block> {
        if pointer == 0 {
            return None;
        }
        let candidates = self.by_old.get(&pointer)?;
        let after = candidates.iter().find(|index| **index >= from);
        let index = after.or_else(|| candidates.iter().rev().find(|index| **index < from))?;
        self.blocks.get(*index)
    }

    pub fn struct_named(&self, name: &str) -> Option<&StructInfo> {
        self.by_name.get(name).and_then(|index| self.structs.get(*index))
    }

    /// The name of the struct a block holds.
    pub fn struct_of(&self, block: &Block) -> &str {
        self.structs.get(block.sdna).map_or("", |info| info.name.as_str())
    }

    /// A struct of type `name` at byte `at`, inside block `block`.
    pub fn view<'a>(&'a self, name: &str, at: usize, block: usize) -> Option<View<'a>> {
        Some(View { file: self, info: self.struct_named(name)?, at, block })
    }

    /// The `slot`th struct of type `name` in a block.
    pub fn view_in<'a>(&'a self, name: &str, block: &Block, slot: usize) -> Option<View<'a>> {
        let info = self.struct_named(name)?;
        let at = slot.checked_mul(info.size)?;
        if at.checked_add(info.size)? > block.len {
            return None;
        }
        Some(View { file: self, info, at: block.start + at, block: block.index })
    }

    /// The bytes of a block.
    pub fn bytes(&self, block: &Block) -> &[u8] {
        self.data.get(block.start..block.start + block.len).unwrap_or_default()
    }
}

/// A struct in the file: fields read by name; a missing field reads as nothing.
#[derive(Clone, Copy)]
pub struct View<'a> {
    pub file: &'a File,
    pub info: &'a StructInfo,
    pub at: usize,
    /// The block the struct is in (pointers resolve from here).
    pub block: usize,
}

impl<'a> View<'a> {
    fn field(&self, name: &str) -> Option<&'a Field> {
        self.info.fields.get(name)
    }

    fn raw(&self, field: &Field, index: usize, size: usize) -> Option<&'a [u8]> {
        let start = self.at.checked_add(field.offset)?.checked_add(index.checked_mul(field.size)?)?;
        self.file.data.get(start..start.checked_add(size)?)
    }

    /// A number field (any integer or float type) as `f64`, element `index` of an array.
    pub fn number_at(&self, name: &str, index: usize) -> Option<f64> {
        let field = self.field(name)?;
        if field.pointer || index >= field.count {
            return None;
        }
        let bytes = self.raw(field, index, field.size)?;
        Some(match (field.type_name.as_str(), bytes) {
            ("float", [a, b, c, d]) => f64::from(f32::from_le_bytes([*a, *b, *c, *d])),
            ("double", _) => {
                let mut array = [0u8; 8];
                array.copy_from_slice(bytes.get(..8)?);
                f64::from_le_bytes(array)
            }
            ("int" | "int32_t", [a, b, c, d]) => f64::from(i32::from_le_bytes([*a, *b, *c, *d])),
            ("uint" | "uint32_t", [a, b, c, d]) => f64::from(u32::from_le_bytes([*a, *b, *c, *d])),
            ("short" | "int16_t", [a, b]) => f64::from(i16::from_le_bytes([*a, *b])),
            ("ushort" | "uint16_t", [a, b]) => f64::from(u16::from_le_bytes([*a, *b])),
            ("char" | "int8_t", [a]) => f64::from(i8::from_le_bytes([*a])),
            ("uchar" | "uint8_t", [a]) => f64::from(*a),
            ("int64_t", _) | ("uint64_t", _) => {
                let mut array = [0u8; 8];
                array.copy_from_slice(bytes.get(..8)?);
                i64::from_le_bytes(array) as f64
            }
            _ => return None,
        })
    }

    pub fn number(&self, name: &str) -> Option<f64> {
        self.number_at(name, 0)
    }

    pub fn f32(&self, name: &str) -> Option<f32> {
        self.number(name).map(|value| value as f32)
    }

    pub fn int(&self, name: &str) -> Option<i64> {
        self.number(name).map(|value| value as i64)
    }

    /// A number array field (`loc[3]`, `parentinv[4][4]`).
    pub fn floats(&self, name: &str) -> Option<Vec<f32>> {
        let field = self.field(name)?;
        (0..field.count).map(|index| self.number_at(name, index).map(|value| value as f32)).collect()
    }

    pub fn pointer(&self, name: &str) -> u64 {
        let Some(field) = self.field(name).filter(|field| field.pointer) else { return 0 };
        self.raw(field, 0, 8).and_then(|bytes| read_u64(bytes, 0)).unwrap_or(0)
    }

    /// A `char name[n]` field as text.
    pub fn text(&self, name: &str) -> Option<String> {
        let field = self.field(name)?;
        let bytes = self.raw(field, 0, field.count)?;
        let end = bytes.iter().position(|byte| *byte == 0).unwrap_or(bytes.len());
        Some(String::from_utf8_lossy(bytes.get(..end).unwrap_or_default()).into_owned())
    }

    /// An embedded struct field.
    pub fn nested(&self, name: &str) -> Option<View<'a>> {
        let field = self.field(name)?;
        if field.pointer {
            return None;
        }
        self.file.view(&field.type_name, self.at.checked_add(field.offset)?, self.block)
    }

    /// The block a pointer field names.
    pub fn target(&self, name: &str) -> Option<&'a Block> {
        self.file.block(self.pointer(name), self.block)
    }

    /// The struct a pointer field names, seen as the type its block declares.
    pub fn follow(&self, name: &str) -> Option<View<'a>> {
        let block = self.target(name)?;
        self.file.view_in(self.file.struct_of(block), block, 0)
    }
}

fn parse_dna(dna: &[u8]) -> Result<Vec<StructInfo>, SceneError> {
    let bad = || invalid("the structure catalogue is damaged");
    let mut at = 0usize;
    let expect = |at: usize, tag: &[u8]| if dna.get(at..at + 4) == Some(tag) { Ok(()) } else { Err(bad()) };
    let align = |at: usize| (at + 3) & !3;
    expect(at, b"SDNA")?;
    at += 4;
    expect(at, b"NAME")?;
    at += 4;
    let names_count = read_u32(dna, at).ok_or_else(bad)? as usize;
    at += 4;
    let strings = |at: &mut usize, count: usize| -> Result<Vec<String>, SceneError> {
        let mut out = Vec::with_capacity(count.min(100_000));
        for _ in 0..count {
            let rest = dna.get(*at..).ok_or_else(bad)?;
            let end = rest.iter().position(|byte| *byte == 0).ok_or_else(bad)?;
            out.push(String::from_utf8_lossy(rest.get(..end).unwrap_or_default()).into_owned());
            *at += end + 1;
        }
        Ok(out)
    };
    if names_count > 1_000_000 {
        return Err(bad());
    }
    let names = strings(&mut at, names_count)?;
    at = align(at);
    expect(at, b"TYPE")?;
    at += 4;
    let types_count = read_u32(dna, at).ok_or_else(bad)? as usize;
    at += 4;
    if types_count > 1_000_000 {
        return Err(bad());
    }
    let types = strings(&mut at, types_count)?;
    at = align(at);
    expect(at, b"TLEN")?;
    at += 4;
    let mut sizes = Vec::with_capacity(types_count);
    for index in 0..types_count {
        sizes.push(read_u16(dna, at + index * 2).ok_or_else(bad)? as usize);
    }
    at = align(at + types_count * 2);
    expect(at, b"STRC")?;
    at += 4;
    let struct_count = read_u32(dna, at).ok_or_else(bad)? as usize;
    at += 4;
    if struct_count > 1_000_000 {
        return Err(bad());
    }
    let mut out = Vec::with_capacity(struct_count);
    for _ in 0..struct_count {
        let type_index = read_u16(dna, at).ok_or_else(bad)? as usize;
        let field_count = read_u16(dna, at + 2).ok_or_else(bad)? as usize;
        at += 4;
        let mut fields = HashMap::new();
        let mut offset = 0usize;
        for _ in 0..field_count {
            let field_type = read_u16(dna, at).ok_or_else(bad)? as usize;
            let field_name = read_u16(dna, at + 2).ok_or_else(bad)? as usize;
            at += 4;
            let type_name = types.get(field_type).ok_or_else(bad)?.clone();
            let raw = names.get(field_name).ok_or_else(bad)?;
            let pointer = raw.starts_with('*') || raw.starts_with("(*");
            let mut count = 1usize;
            let mut rest = raw.as_str();
            while let Some(open) = rest.find('[') {
                let close = rest.get(open..).and_then(|tail| tail.find(']')).map(|close| open + close).ok_or_else(bad)?;
                let number: usize = rest.get(open + 1..close).and_then(|text| text.parse().ok()).ok_or_else(bad)?;
                count = count.checked_mul(number).ok_or_else(bad)?;
                rest = rest.get(close + 1..).unwrap_or_default();
            }
            let base: String = raw.chars().take_while(|ch| *ch != '[').filter(|ch| !matches!(ch, '*' | '(' | ')')).collect();
            let size = if pointer { 8 } else { *sizes.get(field_type).ok_or_else(bad)? };
            fields.insert(base, Field { type_name, offset, pointer, count, size });
            offset = offset.checked_add(size.checked_mul(count).ok_or_else(bad)?).ok_or_else(bad)?;
        }
        out.push(StructInfo { name: types.get(type_index).ok_or_else(bad)?.clone(), size: sizes.get(type_index).copied().unwrap_or(0), fields });
    }
    Ok(out)
}
