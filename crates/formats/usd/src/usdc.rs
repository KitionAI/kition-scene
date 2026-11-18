//! Binary USD layers ("crate" files, `.usdc`), read into the same [`Layer`] the USDA parser
//! makes. The file is a bootstrap header (`PXR-USDC`, a version, the table of contents' offset),
//! a table of sections (tokens, strings, fields, field sets, paths, specs), and the values the
//! fields point at. Values are 64-bit representations: a type, flags (array, inlined,
//! compressed), and either the value itself or its offset in the file.
//!
//! Written from the format's public description and from inspecting files; every offset and
//! count is checked, and nothing is read past the file.

use std::collections::HashMap;

use kition_scene_model::SceneError;

use crate::lz4;
use crate::usda::{Layer, Prim, Property, Specifier, Value, invalid};

mod write;
pub use write::write;

/// The magic at the start of a binary crate file.
pub const MAGIC: &[u8] = b"PXR-USDC";

/// The most items any one array, section, or dictionary may hold.
const MAX_ITEMS: u64 = 64 * 1024 * 1024;
/// Values nest (dictionaries, time samples) no deeper than this.
const MAX_DEPTH: usize = 32;

/// The value types a representation names (the whole table, which documents the format; kinds
/// the reader skips are named too).
#[allow(dead_code)]
mod kind {
    pub const BOOL: u8 = 1;
    pub const UCHAR: u8 = 2;
    pub const INT: u8 = 3;
    pub const UINT: u8 = 4;
    pub const INT64: u8 = 5;
    pub const UINT64: u8 = 6;
    pub const HALF: u8 = 7;
    pub const FLOAT: u8 = 8;
    pub const DOUBLE: u8 = 9;
    pub const STRING: u8 = 10;
    pub const TOKEN: u8 = 11;
    pub const ASSET: u8 = 12;
    pub const MATRIX2D: u8 = 13;
    pub const MATRIX3D: u8 = 14;
    pub const MATRIX4D: u8 = 15;
    pub const QUATD: u8 = 16;
    pub const QUATF: u8 = 17;
    pub const QUATH: u8 = 18;
    pub const VEC2D: u8 = 19;
    pub const VEC2F: u8 = 20;
    pub const VEC2H: u8 = 21;
    pub const VEC2I: u8 = 22;
    pub const VEC3D: u8 = 23;
    pub const VEC3F: u8 = 24;
    pub const VEC3H: u8 = 25;
    pub const VEC3I: u8 = 26;
    pub const VEC4D: u8 = 27;
    pub const VEC4F: u8 = 28;
    pub const VEC4H: u8 = 29;
    pub const VEC4I: u8 = 30;
    pub const DICTIONARY: u8 = 31;
    pub const TOKEN_LIST_OP: u8 = 32;
    pub const STRING_LIST_OP: u8 = 33;
    pub const PATH_LIST_OP: u8 = 34;
    pub const REFERENCE_LIST_OP: u8 = 35;
    pub const INT_LIST_OP: u8 = 36;
    pub const INT64_LIST_OP: u8 = 37;
    pub const UINT_LIST_OP: u8 = 38;
    pub const UINT64_LIST_OP: u8 = 39;
    pub const PATH_VECTOR: u8 = 40;
    pub const TOKEN_VECTOR: u8 = 41;
    pub const SPECIFIER: u8 = 42;
    pub const PERMISSION: u8 = 43;
    pub const VARIABILITY: u8 = 44;
    pub const VARIANT_SELECTION_MAP: u8 = 45;
    pub const TIME_SAMPLES: u8 = 46;
    pub const PAYLOAD: u8 = 47;
    pub const DOUBLE_VECTOR: u8 = 48;
    pub const LAYER_OFFSET_VECTOR: u8 = 49;
    pub const STRING_VECTOR: u8 = 50;
    pub const VALUE_BLOCK: u8 = 51;
    pub const VALUE: u8 = 52;
    pub const UNREGISTERED_VALUE: u8 = 53;
    pub const UNREGISTERED_VALUE_LIST_OP: u8 = 54;
    pub const PAYLOAD_LIST_OP: u8 = 55;
    pub const TIME_CODE: u8 = 56;
}

/// Spec types.
#[allow(dead_code)]
mod spec {
    pub const ATTRIBUTE: u32 = 1;
    pub const CONNECTION: u32 = 2;
    pub const PRIM: u32 = 6;
    pub const PSEUDO_ROOT: u32 = 7;
    pub const RELATIONSHIP: u32 = 8;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rep(u64);

impl Rep {
    fn is_array(self) -> bool {
        self.0 & (1 << 63) != 0
    }
    fn is_inlined(self) -> bool {
        self.0 & (1 << 62) != 0
    }
    fn is_compressed(self) -> bool {
        self.0 & (1 << 61) != 0
    }
    fn kind(self) -> u8 {
        ((self.0 >> 48) & 0xff) as u8
    }
    fn payload(self) -> u64 {
        self.0 & ((1 << 48) - 1)
    }
}

/// A scene path, split into its prim part and an optional property name.
#[derive(Debug, Clone, Default, PartialEq)]
struct Path {
    /// Prim names from the root (empty for the root itself).
    prims: Vec<String>,
    property: Option<String>,
}

impl Path {
    fn text(&self) -> String {
        let mut out = String::new();
        for name in &self.prims {
            out.push('/');
            out.push_str(name);
        }
        if out.is_empty() {
            out.push('/');
        }
        if let Some(property) = &self.property {
            out.push('.');
            out.push_str(property);
        }
        out
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    version: (u8, u8, u8),
    tokens: Vec<String>,
    strings: Vec<u32>,
    fields: Vec<(u32, Rep)>,
    field_sets: Vec<u32>,
    paths: Vec<Path>,
    specs: Vec<(u32, u32, u32)>,
}

fn u64_at(bytes: &[u8], at: u64) -> Result<u64, SceneError> {
    let at = usize::try_from(at).map_err(|_| invalid("an offset is out of range"))?;
    let raw = bytes.get(at..at.checked_add(8).ok_or_else(|| invalid("an offset overflows"))?).ok_or_else(|| invalid("the file ends early"))?;
    Ok(u64::from_le_bytes(crate::lz4::le(raw)))
}

/// A cursor over the file.
#[derive(Clone, Copy)]
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn at(bytes: &'a [u8], at: u64) -> Result<Self, SceneError> {
        let at = usize::try_from(at).map_err(|_| invalid("an offset is out of range"))?;
        if at > bytes.len() {
            return Err(invalid("an offset passes the file's end"));
        }
        Ok(Self { bytes, at })
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], SceneError> {
        let end = self.at.checked_add(count).ok_or_else(|| invalid("a size overflows"))?;
        let out = self.bytes.get(self.at..end).ok_or_else(|| invalid("the file ends early"))?;
        self.at = end;
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, SceneError> {
        Ok(*self.take(1)?.first().unwrap_or(&0))
    }

    fn u32(&mut self) -> Result<u32, SceneError> {
        let raw = self.take(4)?;
        Ok(u32::from_le_bytes(crate::lz4::le(raw)))
    }

    fn u64(&mut self) -> Result<u64, SceneError> {
        let raw = self.take(8)?;
        Ok(u64::from_le_bytes(crate::lz4::le(raw)))
    }

    fn i64(&mut self) -> Result<i64, SceneError> {
        self.u64().map(|value| value as i64)
    }

    fn count(&mut self) -> Result<usize, SceneError> {
        let count = self.u64()?;
        if count > MAX_ITEMS {
            return Err(invalid("a count is too large"));
        }
        usize::try_from(count).map_err(|_| invalid("a count is too large"))
    }

    /// Integer-compressed 32-bit integers: the compressed size, then the data.
    fn compressed_ints(&mut self, count: usize) -> Result<Vec<i32>, SceneError> {
        let size = self.count()?;
        let data = self.take(size)?;
        if count == 0 {
            return Ok(Vec::new());
        }
        lz4::ints32(data, count)
    }

    fn compressed_ints64(&mut self, count: usize) -> Result<Vec<i64>, SceneError> {
        let size = self.count()?;
        let data = self.take(size)?;
        if count == 0 {
            return Ok(Vec::new());
        }
        lz4::ints64(data, count)
    }
}

fn half(bits: u16) -> f64 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from((bits >> 10) & 0x1f);
    let fraction = f64::from(bits & 0x3ff);
    match exponent {
        0 => sign * fraction * 2f64.powi(-24),
        31 => {
            if fraction == 0.0 {
                sign * f64::INFINITY
            } else {
                f64::NAN
            }
        }
        _ => sign * (1.0 + fraction / 1024.0) * 2f64.powi(exponent - 15),
    }
}

/// Element shapes of the vector, matrix, and quaternion types: (scalar width, count, scalar).
#[derive(Clone, Copy)]
enum Scalar {
    Half,
    Float,
    Double,
    Int,
}

fn shape(kind_id: u8) -> Option<(Scalar, usize)> {
    Some(match kind_id {
        kind::VEC2H => (Scalar::Half, 2),
        kind::VEC3H => (Scalar::Half, 3),
        kind::VEC4H | kind::QUATH => (Scalar::Half, 4),
        kind::VEC2F => (Scalar::Float, 2),
        kind::VEC3F => (Scalar::Float, 3),
        kind::VEC4F | kind::QUATF => (Scalar::Float, 4),
        kind::VEC2D => (Scalar::Double, 2),
        kind::VEC3D => (Scalar::Double, 3),
        kind::VEC4D | kind::QUATD => (Scalar::Double, 4),
        kind::VEC2I => (Scalar::Int, 2),
        kind::VEC3I => (Scalar::Int, 3),
        kind::VEC4I => (Scalar::Int, 4),
        kind::MATRIX2D => (Scalar::Double, 4),
        kind::MATRIX3D => (Scalar::Double, 9),
        kind::MATRIX4D => (Scalar::Double, 16),
        _ => return None,
    })
}

fn scalar(cursor: &mut Cursor<'_>, scalar: Scalar) -> Result<f64, SceneError> {
    Ok(match scalar {
        Scalar::Half => {
            let raw = cursor.take(2)?;
            half(u16::from_le_bytes(crate::lz4::le(raw)))
        }
        Scalar::Float => f64::from(f32::from_bits(cursor.u32()?)),
        Scalar::Double => f64::from_bits(cursor.u64()?),
        Scalar::Int => f64::from(cursor.u32()? as i32),
    })
}

/// A vector, matrix, or quaternion as USDA writes it: a tuple, a matrix as a tuple of row
/// tuples, and a quaternion with its real part first (the file stores it last).
fn shaped(kind_id: u8, values: Vec<f64>) -> Value {
    let numbers = |values: &[f64]| Value::Tuple(values.iter().map(|value| Value::Number(*value)).collect());
    match kind_id {
        kind::MATRIX2D | kind::MATRIX3D | kind::MATRIX4D => {
            let side = match kind_id {
                kind::MATRIX2D => 2,
                kind::MATRIX3D => 3,
                _ => 4,
            };
            Value::Tuple(values.chunks(side).map(numbers).collect())
        }
        kind::QUATD | kind::QUATF | kind::QUATH => {
            let mut reordered = values.clone();
            if let Some(real) = values.get(3).copied() {
                reordered =
                    vec![real, values.first().copied().unwrap_or(0.0), values.get(1).copied().unwrap_or(0.0), values.get(2).copied().unwrap_or(0.0)];
            }
            numbers(&reordered)
        }
        _ => numbers(&values),
    }
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self, SceneError> {
        if !bytes.starts_with(MAGIC) {
            return Err(invalid("not a binary USD file"));
        }
        let version = (*bytes.get(8).unwrap_or(&0), *bytes.get(9).unwrap_or(&0), *bytes.get(10).unwrap_or(&0));
        if version < (0, 4, 0) {
            return Err(SceneError::Unsupported(format!(
                "binary USD version {}.{}.{} is older than this reader knows (0.4.0)",
                version.0, version.1, version.2
            )));
        }
        let toc = u64_at(bytes, 16)?;
        let mut cursor = Cursor::at(bytes, toc)?;
        let count = cursor.count()?;
        let mut sections = HashMap::new();
        for _ in 0..count {
            let name = cursor.take(16)?;
            let name = String::from_utf8_lossy(name.split(|byte| *byte == 0).next().unwrap_or(&[])).to_string();
            let start = cursor.u64()?;
            let size = cursor.u64()?;
            if start.checked_add(size).is_none_or(|end| end > bytes.len() as u64) {
                return Err(invalid(format!("section {name} passes the file's end")));
            }
            sections.insert(name, start);
        }
        let section = |name: &str| sections.get(name).copied().ok_or_else(|| invalid(format!("the file has no {name} section")));
        let mut reader = Self {
            bytes,
            version,
            tokens: Vec::new(),
            strings: Vec::new(),
            fields: Vec::new(),
            field_sets: Vec::new(),
            paths: Vec::new(),
            specs: Vec::new(),
        };

        // Tokens: their count, the uncompressed and compressed sizes, then NUL-separated text.
        let mut cursor = Cursor::at(bytes, section("TOKENS")?)?;
        let count = cursor.count()?;
        let uncompressed = cursor.count()?;
        let compressed = cursor.count()?;
        let text = lz4::fast(cursor.take(compressed)?, uncompressed)?;
        reader.tokens = text.split(|byte| *byte == 0).take(count).map(|token| String::from_utf8_lossy(token).to_string()).collect();
        if reader.tokens.len() != count {
            return Err(invalid("the token table is short"));
        }

        // Strings: token indices.
        let mut cursor = Cursor::at(bytes, section("STRINGS")?)?;
        let count = cursor.count()?;
        reader.strings = (0..count).map(|_| cursor.u32()).collect::<Result<_, _>>()?;

        // Fields: token indices (compressed), then their value representations (compressed).
        let mut cursor = Cursor::at(bytes, section("FIELDS")?)?;
        let count = cursor.count()?;
        let names = cursor.compressed_ints(count)?;
        let size = cursor.count()?;
        let reps = lz4::fast(cursor.take(size)?, count.checked_mul(8).ok_or_else(|| invalid("too many fields"))?)?;
        reader.fields = names.iter().zip(reps.as_chunks::<8>().0).map(|(name, rep)| (*name as u32, Rep(u64::from_le_bytes(*rep)))).collect();

        // Field sets: field indices, each set ended by !0.
        let mut cursor = Cursor::at(bytes, section("FIELDSETS")?)?;
        let count = cursor.count()?;
        reader.field_sets = cursor.compressed_ints(count)?.into_iter().map(|value| value as u32).collect();

        // Paths: a tree walked in order, with jumps to siblings.
        let mut cursor = Cursor::at(bytes, section("PATHS")?)?;
        let total = cursor.count()?;
        let encoded = cursor.count()?;
        let indices = cursor.compressed_ints(encoded)?;
        let elements = cursor.compressed_ints(encoded)?;
        let jumps = cursor.compressed_ints(encoded)?;
        reader.paths = vec![Path::default(); total];
        reader.build_paths(&indices, &elements, &jumps)?;

        // Specs: path, field set, and spec type, each compressed.
        let mut cursor = Cursor::at(bytes, section("SPECS")?)?;
        let count = cursor.count()?;
        let paths = cursor.compressed_ints(count)?;
        let sets = cursor.compressed_ints(count)?;
        let types = cursor.compressed_ints(count)?;
        reader.specs = paths.iter().zip(&sets).zip(&types).map(|((path, set), kind)| (*path as u32, *set as u32, *kind as u32)).collect();
        Ok(reader)
    }

    fn build_paths(&mut self, indices: &[i32], elements: &[i32], jumps: &[i32]) -> Result<(), SceneError> {
        // (next index, parent path), walked with an explicit stack instead of recursion.
        let mut stack: Vec<(usize, Option<Path>)> = vec![(0, None)];
        let mut visited = 0usize;
        while let Some((mut index, mut parent)) = stack.pop() {
            loop {
                visited += 1;
                if visited > indices.len().saturating_mul(2) + 2 {
                    return Err(invalid("the path tree loops"));
                }
                let this = index;
                index += 1;
                let (Some(&slot), Some(&element), Some(&jump)) = (indices.get(this), elements.get(this), jumps.get(this)) else {
                    return Err(invalid("the path tree points past its end"));
                };
                let path = match &parent {
                    None => Path::default(),
                    Some(parent) => {
                        let token =
                            self.tokens.get(element.unsigned_abs() as usize).cloned().ok_or_else(|| invalid("a path names a missing token"))?;
                        let mut path = parent.clone();
                        if element < 0 {
                            path.property = Some(token);
                        } else {
                            path.prims.push(token);
                        }
                        path
                    }
                };
                let target = self
                    .paths
                    .get_mut(usize::try_from(slot).map_err(|_| invalid("a negative path index"))?)
                    .ok_or_else(|| invalid("a path index is out of range"))?;
                *target = path.clone();
                let has_child = jump > 0 || jump == -1;
                let has_sibling = jump >= 0;
                if has_child {
                    if has_sibling {
                        let sibling = this
                            .checked_add(usize::try_from(jump).map_err(|_| invalid("a bad path jump"))?)
                            .ok_or_else(|| invalid("a bad path jump"))?;
                        stack.push((sibling, parent.clone()));
                    }
                    parent = Some(path);
                } else if !has_sibling {
                    break;
                }
            }
        }
        Ok(())
    }

    fn token(&self, index: u64) -> Result<String, SceneError> {
        self.tokens
            .get(usize::try_from(index).map_err(|_| invalid("a bad token index"))?)
            .cloned()
            .ok_or_else(|| invalid("a token index is out of range"))
    }

    fn string(&self, index: u64) -> Result<String, SceneError> {
        let token = self
            .strings
            .get(usize::try_from(index).map_err(|_| invalid("a bad string index"))?)
            .ok_or_else(|| invalid("a string index is out of range"))?;
        self.token(u64::from(*token))
    }

    fn path_text(&self, index: u64) -> Result<String, SceneError> {
        self.paths
            .get(usize::try_from(index).map_err(|_| invalid("a bad path index"))?)
            .map(Path::text)
            .ok_or_else(|| invalid("a path index is out of range"))
    }

    /// The fields of a field set starting at `start`.
    fn field_set(&self, start: u32) -> Result<Vec<(String, Rep)>, SceneError> {
        let mut out = Vec::new();
        let mut at = start as usize;
        loop {
            let index = *self.field_sets.get(at).ok_or_else(|| invalid("a field set runs past its table"))?;
            if index == u32::MAX {
                return Ok(out);
            }
            let (name, rep) = *self.fields.get(index as usize).ok_or_else(|| invalid("a field index is out of range"))?;
            out.push((self.token(u64::from(name))?, rep));
            at += 1;
        }
    }

    /// A value from its representation.
    fn value(&self, rep: Rep, depth: usize) -> Result<Value, SceneError> {
        if depth > MAX_DEPTH {
            return Err(invalid("values nest too deep"));
        }
        if rep.is_array() {
            return self.array(rep);
        }
        let payload = rep.payload();
        if rep.is_inlined() {
            return self.inlined(rep.kind(), payload);
        }
        let mut cursor = Cursor::at(self.bytes, payload)?;
        let kind_id = rep.kind();
        Ok(match kind_id {
            kind::BOOL => Value::Bool(cursor.u8()? != 0),
            kind::UCHAR => Value::Number(f64::from(cursor.u8()?)),
            kind::INT => Value::Number(f64::from(cursor.u32()? as i32)),
            kind::UINT => Value::Number(f64::from(cursor.u32()?)),
            kind::INT64 => Value::Number(cursor.i64()? as f64),
            kind::UINT64 => Value::Number(cursor.u64()? as f64),
            kind::HALF => Value::Number(scalar(&mut cursor, Scalar::Half)?),
            kind::FLOAT => Value::Number(scalar(&mut cursor, Scalar::Float)?),
            kind::DOUBLE | kind::TIME_CODE => Value::Number(scalar(&mut cursor, Scalar::Double)?),
            kind::STRING => Value::Text(self.string(u64::from(cursor.u32()?))?),
            kind::TOKEN => Value::Text(self.token(u64::from(cursor.u32()?))?),
            kind::ASSET => Value::Asset(self.token(u64::from(cursor.u32()?))?),
            _ if shape(kind_id).is_some() => {
                let (scalar_kind, count) = shape(kind_id).unwrap_or((Scalar::Double, 0));
                let values = (0..count).map(|_| scalar(&mut cursor, scalar_kind)).collect::<Result<Vec<_>, _>>()?;
                shaped(kind_id, values)
            }
            kind::DICTIONARY => self.dictionary(&mut cursor, depth)?,
            kind::TOKEN_VECTOR => {
                let count = cursor.count()?;
                Value::List(
                    (0..count).map(|_| cursor.u32().and_then(|token| self.token(u64::from(token))).map(Value::Text)).collect::<Result<_, _>>()?,
                )
            }
            kind::PATH_VECTOR => {
                let count = cursor.count()?;
                Value::List(
                    (0..count).map(|_| cursor.u32().and_then(|path| self.path_text(u64::from(path))).map(Value::Path)).collect::<Result<_, _>>()?,
                )
            }
            kind::STRING_VECTOR => {
                let count = cursor.count()?;
                Value::List(
                    (0..count).map(|_| cursor.u32().and_then(|string| self.string(u64::from(string))).map(Value::Text)).collect::<Result<_, _>>()?,
                )
            }
            kind::DOUBLE_VECTOR => {
                let count = cursor.count()?;
                Value::List((0..count).map(|_| scalar(&mut cursor, Scalar::Double).map(Value::Number)).collect::<Result<_, _>>()?)
            }
            kind::TOKEN_LIST_OP | kind::PATH_LIST_OP | kind::STRING_LIST_OP => self.list_op(&mut cursor, kind_id)?,
            kind::TIME_SAMPLES => self.time_samples(payload, depth)?,
            kind::VALUE_BLOCK => Value::Ident("None".into()),
            // Other kinds (references, payloads, variant maps, layer offsets, unregistered
            // values) carry composition the reader does not follow.
            _ => Value::List(Vec::new()),
        })
    }

    /// A value stored in the representation itself: small scalars, and vectors and matrices
    /// whose components are small whole numbers (one signed byte each; a matrix's diagonal).
    fn inlined(&self, kind_id: u8, payload: u64) -> Result<Value, SceneError> {
        let low = payload as u32;
        let bytes = low.to_le_bytes();
        Ok(match kind_id {
            kind::BOOL => Value::Bool(low != 0),
            kind::UCHAR | kind::UINT | kind::SPECIFIER | kind::PERMISSION | kind::VARIABILITY => Value::Number(f64::from(low)),
            kind::INT => Value::Number(f64::from(low as i32)),
            kind::INT64 | kind::UINT64 => Value::Number(low as i32 as f64),
            kind::HALF => Value::Number(half(low as u16)),
            kind::FLOAT => Value::Number(f64::from(f32::from_bits(low))),
            // A double that fits a float is stored as one.
            kind::DOUBLE | kind::TIME_CODE => Value::Number(f64::from(f32::from_bits(low))),
            kind::STRING => Value::Text(self.string(u64::from(low))?),
            kind::TOKEN => Value::Text(self.token(u64::from(low))?),
            kind::ASSET => Value::Asset(self.token(u64::from(low))?),
            kind::MATRIX2D | kind::MATRIX3D | kind::MATRIX4D => {
                let side = match kind_id {
                    kind::MATRIX2D => 2,
                    kind::MATRIX3D => 3,
                    _ => 4,
                };
                let mut values = vec![0.0; side * side];
                for index in 0..side {
                    if let Some(slot) = values.get_mut(index * side + index) {
                        *slot = f64::from(bytes.get(index).copied().unwrap_or(0) as i8);
                    }
                }
                shaped(kind_id, values)
            }
            _ if shape(kind_id).is_some() => {
                let (_, count) = shape(kind_id).unwrap_or((Scalar::Double, 0));
                let values = (0..count).map(|index| f64::from(bytes.get(index).copied().unwrap_or(0) as i8)).collect();
                shaped(kind_id, values)
            }
            kind::DICTIONARY => Value::Dict(Vec::new()),
            kind::TOKEN_VECTOR | kind::PATH_VECTOR | kind::STRING_VECTOR | kind::DOUBLE_VECTOR => Value::List(Vec::new()),
            kind::VALUE_BLOCK => Value::Ident("None".into()),
            _ => Value::List(Vec::new()),
        })
    }

    fn array(&self, rep: Rep) -> Result<Value, SceneError> {
        let kind_id = rep.kind();
        if rep.payload() == 0 {
            return Ok(Value::List(Vec::new()));
        }
        let mut cursor = Cursor::at(self.bytes, rep.payload())?;
        if self.version < (0, 5, 0) {
            cursor.u32()?;
        }
        let count = if self.version < (0, 7, 0) { cursor.u32()? as usize } else { cursor.count()? };
        if count as u64 > MAX_ITEMS {
            return Err(invalid("an array is too large"));
        }
        let numbers = |values: Vec<f64>| Value::List(values.into_iter().map(Value::Number).collect());
        let compressed = rep.is_compressed();
        Ok(match kind_id {
            kind::INT | kind::UINT if compressed => numbers(
                cursor
                    .compressed_ints(count)?
                    .into_iter()
                    .map(|value| if kind_id == kind::UINT { f64::from(value as u32) } else { f64::from(value) })
                    .collect(),
            ),
            kind::INT64 | kind::UINT64 if compressed => numbers(cursor.compressed_ints64(count)?.into_iter().map(|value| value as f64).collect()),
            kind::HALF | kind::FLOAT | kind::DOUBLE if compressed => {
                let code = cursor.u8()?;
                let values: Vec<f64> = match code {
                    b'i' => cursor.compressed_ints(count)?.into_iter().map(f64::from).collect(),
                    b't' => {
                        let size = cursor.u32()? as usize;
                        if size as u64 > MAX_ITEMS {
                            return Err(invalid("a lookup table is too large"));
                        }
                        let scalar_kind = match kind_id {
                            kind::HALF => Scalar::Half,
                            kind::FLOAT => Scalar::Float,
                            _ => Scalar::Double,
                        };
                        let table = (0..size).map(|_| scalar(&mut cursor, scalar_kind)).collect::<Result<Vec<_>, _>>()?;
                        cursor
                            .compressed_ints(count)?
                            .into_iter()
                            .map(|index| table.get(index as u32 as usize).copied().ok_or_else(|| invalid("a lookup index is out of range")))
                            .collect::<Result<_, _>>()?
                    }
                    _ => return Err(invalid("an unknown compressed number array")),
                };
                numbers(values)
            }
            kind::BOOL | kind::UCHAR => numbers((0..count).map(|_| cursor.u8().map(f64::from)).collect::<Result<_, _>>()?),
            kind::INT => numbers((0..count).map(|_| cursor.u32().map(|value| f64::from(value as i32))).collect::<Result<_, _>>()?),
            kind::UINT => numbers((0..count).map(|_| cursor.u32().map(f64::from)).collect::<Result<_, _>>()?),
            kind::INT64 => numbers((0..count).map(|_| cursor.i64().map(|value| value as f64)).collect::<Result<_, _>>()?),
            kind::UINT64 => numbers((0..count).map(|_| cursor.u64().map(|value| value as f64)).collect::<Result<_, _>>()?),
            kind::HALF => numbers((0..count).map(|_| scalar(&mut cursor, Scalar::Half)).collect::<Result<_, _>>()?),
            kind::FLOAT => numbers((0..count).map(|_| scalar(&mut cursor, Scalar::Float)).collect::<Result<_, _>>()?),
            kind::DOUBLE | kind::TIME_CODE => numbers((0..count).map(|_| scalar(&mut cursor, Scalar::Double)).collect::<Result<_, _>>()?),
            kind::TOKEN => Value::List(
                (0..count).map(|_| cursor.u32().and_then(|token| self.token(u64::from(token))).map(Value::Text)).collect::<Result<_, _>>()?,
            ),
            kind::STRING => Value::List(
                (0..count).map(|_| cursor.u32().and_then(|string| self.string(u64::from(string))).map(Value::Text)).collect::<Result<_, _>>()?,
            ),
            kind::ASSET => Value::List(
                (0..count).map(|_| cursor.u32().and_then(|token| self.token(u64::from(token))).map(Value::Asset)).collect::<Result<_, _>>()?,
            ),
            _ => match shape(kind_id) {
                Some((scalar_kind, width)) => {
                    let mut items = Vec::with_capacity(count);
                    for _ in 0..count {
                        let values = (0..width).map(|_| scalar(&mut cursor, scalar_kind)).collect::<Result<Vec<_>, _>>()?;
                        items.push(shaped(kind_id, values));
                    }
                    Value::List(items)
                }
                None => Value::List(Vec::new()),
            },
        })
    }

    /// A dictionary: its size, then for each entry a string key and a relative jump to the
    /// entry's value representation.
    fn dictionary(&self, cursor: &mut Cursor<'_>, depth: usize) -> Result<Value, SceneError> {
        let count = cursor.count()?;
        let mut entries = Vec::with_capacity(count.min(1024));
        for _ in 0..count {
            let key = self.string(u64::from(cursor.u32()?))?;
            let rep = self.jump_rep(cursor)?;
            entries.push((key, self.value(rep, depth + 1)?));
        }
        Ok(Value::Dict(entries))
    }

    /// A relative jump (from where it is read) to a value representation, which the cursor
    /// then sits after.
    fn jump_rep(&self, cursor: &mut Cursor<'_>) -> Result<Rep, SceneError> {
        let from = cursor.at;
        let jump = cursor.i64()?;
        let target = (from as i64).checked_add(jump).ok_or_else(|| invalid("a jump overflows"))?;
        let target = u64::try_from(target).map_err(|_| invalid("a jump points before the file"))?;
        let mut there = Cursor::at(self.bytes, target)?;
        let rep = Rep(there.u64()?);
        cursor.at = there.at;
        Ok(rep)
    }

    /// Time samples: a jump to the times' representation, then a jump to the count of values
    /// and their representations.
    fn time_samples(&self, offset: u64, depth: usize) -> Result<Value, SceneError> {
        let mut cursor = Cursor::at(self.bytes, offset)?;
        let times_rep = self.jump_rep(&mut cursor)?;
        let times = self.value(times_rep, depth + 1)?;
        let from = cursor.at;
        let jump = cursor.i64()?;
        let target = u64::try_from((from as i64).checked_add(jump).ok_or_else(|| invalid("a jump overflows"))?)
            .map_err(|_| invalid("a jump points before the file"))?;
        let mut values = Cursor::at(self.bytes, target)?;
        let count = values.count()?;
        let times = times.items().to_vec();
        if times.len() != count {
            return Err(invalid("time samples have more times than values"));
        }
        let mut entries = Vec::with_capacity(count);
        for time in times {
            let rep = Rep(values.u64()?);
            let key = time.as_f64().unwrap_or(0.0).to_string();
            entries.push((key, self.value(rep, depth + 1)?));
        }
        Ok(Value::Dict(entries))
    }

    /// A list operation: a header byte of flags, then each list it carries (explicit, added,
    /// deleted, ordered, prepended, appended), as a count and items. The items the operation
    /// leaves in place (explicit, else prepended + added + appended) come back.
    fn list_op(&self, cursor: &mut Cursor<'_>, kind_id: u8) -> Result<Value, SceneError> {
        let header = cursor.u8()?;
        let mut kept = Vec::new();
        let mut explicit = None;
        for (bit, keep) in [(1u8 << 1, 0u8), (1 << 2, 1), (1 << 3, 2), (1 << 4, 2), (1 << 5, 1), (1 << 6, 1)] {
            if header & bit == 0 {
                continue;
            }
            let count = cursor.count()?;
            let mut items = Vec::with_capacity(count.min(4096));
            for _ in 0..count {
                let index = u64::from(cursor.u32()?);
                items.push(match kind_id {
                    kind::PATH_LIST_OP => Value::Path(self.path_text(index)?),
                    kind::STRING_LIST_OP => Value::Text(self.string(index)?),
                    _ => Value::Text(self.token(index)?),
                });
            }
            match keep {
                0 => explicit = Some(items),
                1 => kept.extend(items),
                _ => {}
            }
        }
        Ok(Value::List(explicit.unwrap_or(kept)))
    }

    /// The layer the specs describe.
    fn layer(&self) -> Result<Layer, SceneError> {
        let mut layer = Layer::default();
        // Prims by path text, built first; properties attach to them; then the tree is assembled
        // in each prim's child order.
        let mut prims: HashMap<String, Prim> = HashMap::new();
        let mut children: HashMap<String, Vec<String>> = HashMap::new();
        let mut property_order: HashMap<String, Vec<String>> = HashMap::new();
        let mut properties: Vec<(String, Property)> = Vec::new();
        for (path_index, set, spec_type) in &self.specs {
            let path = self.paths.get(*path_index as usize).ok_or_else(|| invalid("a spec's path is out of range"))?;
            let fields = self.field_set(*set)?;
            match *spec_type {
                spec::PSEUDO_ROOT => {
                    for (name, rep) in fields {
                        if name == "primChildren" {
                            if let Value::List(items) = self.value(rep, 0)? {
                                children.insert("/".into(), items.iter().filter_map(|item| item.as_str().map(str::to_string)).collect());
                            }
                            continue;
                        }
                        layer.metadata.push((name, self.value(rep, 0)?));
                    }
                }
                spec::PRIM => {
                    let text = path.text();
                    let mut prim = Prim {
                        specifier: Specifier::Def,
                        type_name: String::new(),
                        name: path.prims.last().cloned().unwrap_or_default(),
                        metadata: Vec::new(),
                        properties: Vec::new(),
                        children: Vec::new(),
                    };
                    for (name, rep) in fields {
                        let value = self.value(rep, 0)?;
                        match name.as_str() {
                            "specifier" => {
                                prim.specifier = match value.as_f64().unwrap_or(0.0) as u32 {
                                    1 => Specifier::Over,
                                    2 => Specifier::Class,
                                    _ => Specifier::Def,
                                }
                            }
                            "typeName" => prim.type_name = value.as_str().unwrap_or_default().to_string(),
                            "primChildren" => {
                                children.insert(text.clone(), value.items().iter().filter_map(|item| item.as_str().map(str::to_string)).collect());
                            }
                            "properties" => {
                                property_order
                                    .insert(text.clone(), value.items().iter().filter_map(|item| item.as_str().map(str::to_string)).collect());
                            }
                            _ => prim.metadata.push((name, value)),
                        }
                    }
                    prims.insert(text, prim);
                }
                spec::ATTRIBUTE | spec::RELATIONSHIP => {
                    let Some(name) = path.property.clone() else { continue };
                    let owner = Path { prims: path.prims.clone(), property: None }.text();
                    let mut property = Property { name, relationship: *spec_type == spec::RELATIONSHIP, ..Property::default() };
                    for (field, rep) in fields {
                        let value = self.value(rep, 0)?;
                        match field.as_str() {
                            "typeName" => property.type_name = value.as_str().unwrap_or_default().to_string(),
                            "default" => property.value = Some(value),
                            "timeSamples" => {
                                if let Value::Dict(entries) = value {
                                    let mut samples: Vec<(f64, Value)> = entries
                                        .into_iter()
                                        .filter_map(|(time, value)| {
                                            time.parse::<f64>().ok().filter(|time| time.is_finite()).map(|time| (time, value))
                                        })
                                        .collect();
                                    samples.sort_by(|a, b| a.0.total_cmp(&b.0));
                                    property.samples = samples;
                                }
                            }
                            "connectionPaths" => property.connect = Some(single_or_list(value)),
                            "targetPaths" => property.value = Some(single_or_list(value)),
                            "variability" => property.uniform = value.as_f64() == Some(1.0),
                            "custom" => property.custom = value.as_f64() == Some(1.0),
                            _ => property.metadata.push((field, value)),
                        }
                    }
                    properties.push((owner, property));
                }
                // Connections and other spec kinds carry nothing the reader uses.
                spec::CONNECTION => {}
                _ => {}
            }
        }
        for (owner, property) in properties {
            if let Some(prim) = prims.get_mut(&owner) {
                prim.properties.push(property);
            }
        }
        for (path, order) in &property_order {
            if let Some(prim) = prims.get_mut(path) {
                prim.properties.sort_by_key(|property| order.iter().position(|name| *name == property.name).unwrap_or(usize::MAX));
            }
        }
        // Assemble children bottom-up: deepest paths first.
        let mut paths: Vec<String> = prims.keys().cloned().collect();
        paths.sort_by_key(|path| std::cmp::Reverse(path.matches('/').count()));
        for path in paths {
            let order = children.get(&path).cloned().unwrap_or_default();
            let mut kids = Vec::new();
            for name in order {
                let child = if path == "/" { format!("/{name}") } else { format!("{path}/{name}") };
                if let Some(prim) = prims.remove(&child) {
                    kids.push(prim);
                }
            }
            if let Some(prim) = prims.get_mut(&path) {
                prim.children = kids;
            }
        }
        for name in children.get("/").cloned().unwrap_or_default() {
            if let Some(prim) = prims.remove(&format!("/{name}")) {
                layer.prims.push(prim);
            }
        }
        Ok(layer)
    }
}

/// A relationship's or connection's targets: one path, or a list of them.
fn single_or_list(value: Value) -> Value {
    match value {
        Value::List(mut items) if items.len() == 1 => items.pop().unwrap_or(Value::List(Vec::new())),
        other => other,
    }
}

/// Reads a binary USD layer.
pub fn parse(bytes: &[u8]) -> Result<Layer, SceneError> {
    Reader::new(bytes)?.layer()
}
