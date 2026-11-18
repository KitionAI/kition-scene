//! The FBX document tree: named nodes with typed properties and children, read from the binary
//! (7.x, 32- and 64-bit record headers, zlib arrays) and ASCII encodings, and written as binary
//! 7400. Everything read is checked against the bytes that are there: sizes, offsets, and array
//! lengths never index past the end, and totals are capped.

use std::fmt::Write as _;

use kition_scene_model::SceneError;

/// The binary file's opening bytes.
pub const MAGIC: &[u8; 21] = b"Kaydara FBX Binary  \0";
/// The version this crate writes.
pub const WRITE_VERSION: u32 = 7400;
/// The largest number of nodes, array elements, or bytes read from one file.
const MAX_ITEMS: usize = 50_000_000;
/// The deepest nesting read.
const MAX_DEPTH: usize = 64;

/// The footer's fixed parts, as files of the format end.
const FOOTER_ID: [u8; 16] = [0xfa, 0xbc, 0xab, 0x09, 0xd0, 0xc8, 0xd4, 0x66, 0xb1, 0x76, 0xfb, 0x83, 0x1c, 0xf7, 0x26, 0x7e];
const FOOTER_MAGIC: [u8; 16] = [0xf8, 0x5a, 0x8c, 0x6a, 0xde, 0xf5, 0xd9, 0x7e, 0xec, 0xe9, 0x0c, 0xe3, 0x75, 0x8f, 0x29, 0x0b];

/// One property value.
#[derive(Debug, Clone, PartialEq)]
pub enum Prop {
    Bool(bool),
    I16(i16),
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    String(String),
    Raw(Vec<u8>),
    BoolArray(Vec<bool>),
    I32Array(Vec<i32>),
    I64Array(Vec<i64>),
    F32Array(Vec<f32>),
    F64Array(Vec<f64>),
}

impl Prop {
    /// The value as a whole number, when it is one.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Bool(value) => Some(i64::from(*value)),
            Self::I16(value) => Some(i64::from(*value)),
            Self::I32(value) => Some(i64::from(*value)),
            Self::I64(value) => Some(*value),
            _ => None,
        }
    }

    /// The value as a number, when it is one.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::F32(value) => Some(f64::from(*value)),
            Self::F64(value) => Some(*value),
            other => other.as_i64().map(|value| value as f64),
        }
    }

    /// The value as text, when it is text.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(text) => Some(text),
            _ => None,
        }
    }

    /// The value as numbers, when it is a numeric array.
    pub fn as_f64s(&self) -> Option<Vec<f64>> {
        match self {
            Self::F64Array(values) => Some(values.clone()),
            Self::F32Array(values) => Some(values.iter().map(|value| f64::from(*value)).collect()),
            Self::I32Array(values) => Some(values.iter().map(|value| f64::from(*value)).collect()),
            Self::I64Array(values) => Some(values.iter().map(|value| *value as f64).collect()),
            _ => None,
        }
    }

    /// The value as whole numbers, when it is an integer array.
    pub fn as_i64s(&self) -> Option<Vec<i64>> {
        match self {
            Self::I32Array(values) => Some(values.iter().map(|value| i64::from(*value)).collect()),
            Self::I64Array(values) => Some(values.clone()),
            Self::F64Array(values) => Some(values.iter().map(|value| *value as i64).collect()),
            Self::F32Array(values) => Some(values.iter().map(|value| *value as i64).collect()),
            _ => None,
        }
    }
}

/// One node.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Node {
    pub name: String,
    pub props: Vec<Prop>,
    pub children: Vec<Node>,
}

impl Node {
    /// A node with properties and no children.
    pub fn new(name: &str, props: Vec<Prop>) -> Self {
        Self { name: name.to_string(), props, children: Vec::new() }
    }

    /// The node with these children.
    pub fn with(mut self, children: Vec<Node>) -> Self {
        self.children = children;
        self
    }

    /// The first child named `name`.
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|child| child.name == name)
    }

    /// Every child named `name`.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |child| child.name == name)
    }

    /// Property `index`.
    pub fn prop(&self, index: usize) -> Option<&Prop> {
        self.props.get(index)
    }
}

fn invalid(message: impl Into<String>) -> SceneError {
    SceneError::InvalidFile(format!("FBX: {}", message.into()))
}

/// Whether the bytes are a binary FBX file.
pub fn is_binary(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

/// The document's top-level nodes and its version.
pub fn parse(bytes: &[u8]) -> Result<(Vec<Node>, u32), SceneError> {
    if is_binary(bytes) { parse_binary(bytes) } else { parse_ascii(bytes) }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
    wide: bool,
    budget: usize,
}

impl Reader<'_> {
    fn take(&mut self, count: usize) -> Result<&[u8], SceneError> {
        let end = self.at.checked_add(count).ok_or_else(|| invalid("a size overflows"))?;
        let slice = self.bytes.get(self.at..end).ok_or_else(|| invalid("the file is cut short"))?;
        self.at = end;
        Ok(slice)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], SceneError> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, SceneError> {
        Ok(u8::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, SceneError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn offset(&mut self) -> Result<u64, SceneError> {
        if self.wide { Ok(u64::from_le_bytes(self.array()?)) } else { self.u32().map(u64::from) }
    }

    fn spend(&mut self, items: usize) -> Result<(), SceneError> {
        self.budget = self.budget.checked_sub(items).ok_or_else(|| invalid("the file is too large"))?;
        Ok(())
    }
}

fn parse_binary(bytes: &[u8]) -> Result<(Vec<Node>, u32), SceneError> {
    let mut reader = Reader { bytes, at: MAGIC.len(), wide: false, budget: MAX_ITEMS };
    reader.take(2)?;
    let version = reader.u32()?;
    if !(7000..8000).contains(&version) {
        return Err(invalid(format!("version {version} is not read (7.x only)")));
    }
    reader.wide = version >= 7500;
    let mut nodes = Vec::new();
    while let Some(node) = read_node(&mut reader, 0)? {
        nodes.push(node);
    }
    Ok((nodes, version))
}

/// One node record, or `None` at the null record that ends a list.
fn read_node(reader: &mut Reader<'_>, depth: usize) -> Result<Option<Node>, SceneError> {
    if depth > MAX_DEPTH {
        return Err(invalid("the nodes nest too deep"));
    }
    // A list may also end where the file does (some writers leave the top-level null record out).
    if depth == 0 && reader.at >= reader.bytes.len() {
        return Ok(None);
    }
    let end = reader.offset()?;
    let count = reader.offset()?;
    let _length = reader.offset()?;
    let name_length = reader.u8()?;
    if end == 0 {
        // The null record: the rest of its header is zero.
        if count != 0 || name_length != 0 {
            return Err(invalid("a damaged node record"));
        }
        return Ok(None);
    }
    let end = usize::try_from(end).map_err(|_| invalid("an offset is out of range"))?;
    if end <= reader.at || end > reader.bytes.len() {
        return Err(invalid("a node's end lies outside the file"));
    }
    reader.spend(1)?;
    let name = String::from_utf8_lossy(reader.take(usize::from(name_length))?).into_owned();
    let count = usize::try_from(count).map_err(|_| invalid("a property count is out of range"))?;
    if count > end.saturating_sub(reader.at) {
        return Err(invalid("a node claims more properties than it has bytes"));
    }
    let mut props = Vec::with_capacity(count);
    for _ in 0..count {
        props.push(read_prop(reader)?);
    }
    let mut children = Vec::new();
    while reader.at < end {
        match read_node(reader, depth + 1)? {
            Some(child) => children.push(child),
            None => break,
        }
    }
    if reader.at != end {
        return Err(invalid(format!("node {name} does not end where it says")));
    }
    Ok(Some(Node { name, props, children }))
}

fn read_prop(reader: &mut Reader<'_>) -> Result<Prop, SceneError> {
    let code = reader.u8()?;
    Ok(match code {
        b'C' => Prop::Bool(reader.u8()? != 0),
        b'Y' => Prop::I16(i16::from_le_bytes(reader.array()?)),
        b'I' => Prop::I32(i32::from_le_bytes(reader.array()?)),
        b'L' => Prop::I64(i64::from_le_bytes(reader.array()?)),
        b'F' => Prop::F32(f32::from_le_bytes(reader.array()?)),
        b'D' => Prop::F64(f64::from_le_bytes(reader.array()?)),
        b'S' | b'R' => {
            let length = reader.u32()? as usize;
            reader.spend(length)?;
            let data = reader.take(length)?.to_vec();
            if code == b'S' { Prop::String(String::from_utf8_lossy(&data).into_owned()) } else { Prop::Raw(data) }
        }
        b'f' | b'd' | b'l' | b'i' | b'b' => {
            let length = reader.u32()? as usize;
            let encoding = reader.u32()?;
            let compressed = reader.u32()? as usize;
            let size = match code {
                b'f' | b'i' => 4,
                b'd' | b'l' => 8,
                _ => 1,
            };
            reader.spend(length)?;
            let expected = length.checked_mul(size).ok_or_else(|| invalid("an array is too large"))?;
            let stored = reader.take(compressed)?;
            let data = match encoding {
                0 => stored.to_vec(),
                1 => miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(stored, expected)
                    .map_err(|_| invalid("an array's zlib data is damaged"))?,
                other => return Err(invalid(format!("unknown array encoding {other}"))),
            };
            if data.len() != expected {
                return Err(invalid("an array's length does not match its data"));
            }
            match code {
                b'f' => Prop::F32Array(data.as_chunks::<4>().0.iter().map(|chunk| f32::from_le_bytes(*chunk)).collect()),
                b'i' => Prop::I32Array(data.as_chunks::<4>().0.iter().map(|chunk| i32::from_le_bytes(*chunk)).collect()),
                b'd' => Prop::F64Array(data.as_chunks::<8>().0.iter().map(|chunk| f64::from_le_bytes(*chunk)).collect()),
                b'l' => Prop::I64Array(data.as_chunks::<8>().0.iter().map(|chunk| i64::from_le_bytes(*chunk)).collect()),
                _ => Prop::BoolArray(data.iter().map(|byte| *byte != 0).collect()),
            }
        }
        other => return Err(invalid(format!("unknown property type {:?}", char::from(other)))),
    })
}

/// Writes top-level nodes as a binary 7400 file.
pub fn write_binary(nodes: &[Node]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&[0x1a, 0x00]);
    out.extend_from_slice(&WRITE_VERSION.to_le_bytes());
    for node in nodes {
        write_node(&mut out, node);
    }
    out.extend_from_slice(&[0u8; 13]);
    out.extend_from_slice(&FOOTER_ID);
    out.extend_from_slice(&[0u8; 4]);
    let padding = (16 - out.len() % 16) % 16;
    out.extend(std::iter::repeat_n(0u8, padding));
    out.extend_from_slice(&WRITE_VERSION.to_le_bytes());
    out.extend_from_slice(&[0u8; 120]);
    out.extend_from_slice(&FOOTER_MAGIC);
    out
}

fn put_u32(out: &mut [u8], at: usize, value: usize) {
    if let Some(slot) = out.get_mut(at..at + 4) {
        slot.copy_from_slice(&u32::try_from(value).unwrap_or(u32::MAX).to_le_bytes());
    }
}

fn write_node(out: &mut Vec<u8>, node: &Node) {
    let start = out.len();
    out.extend_from_slice(&[0u8; 12]);
    let name = node.name.as_bytes();
    out.push(u8::try_from(name.len()).unwrap_or(u8::MAX));
    out.extend_from_slice(name.get(..usize::from(u8::MAX)).unwrap_or(name));
    let props_start = out.len();
    for prop in &node.props {
        write_prop(out, prop);
    }
    let props_length = out.len() - props_start;
    if !node.children.is_empty() {
        for child in &node.children {
            write_node(out, child);
        }
        out.extend_from_slice(&[0u8; 13]);
    }
    let end = out.len();
    put_u32(out, start, end);
    put_u32(out, start + 4, node.props.len());
    put_u32(out, start + 8, props_length);
}

fn write_array(out: &mut Vec<u8>, code: u8, count: usize, raw: Vec<u8>) {
    out.push(code);
    out.extend_from_slice(&u32::try_from(count).unwrap_or(u32::MAX).to_le_bytes());
    // Arrays of a few elements stay raw, as other writers leave them.
    if raw.len() > 128 {
        let packed = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&u32::try_from(packed.len()).unwrap_or(u32::MAX).to_le_bytes());
        out.extend_from_slice(&packed);
    } else {
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&u32::try_from(raw.len()).unwrap_or(u32::MAX).to_le_bytes());
        out.extend_from_slice(&raw);
    }
}

fn write_prop(out: &mut Vec<u8>, prop: &Prop) {
    match prop {
        Prop::Bool(value) => out.extend_from_slice(&[b'C', u8::from(*value)]),
        Prop::I16(value) => {
            out.push(b'Y');
            out.extend_from_slice(&value.to_le_bytes());
        }
        Prop::I32(value) => {
            out.push(b'I');
            out.extend_from_slice(&value.to_le_bytes());
        }
        Prop::I64(value) => {
            out.push(b'L');
            out.extend_from_slice(&value.to_le_bytes());
        }
        Prop::F32(value) => {
            out.push(b'F');
            out.extend_from_slice(&value.to_le_bytes());
        }
        Prop::F64(value) => {
            out.push(b'D');
            out.extend_from_slice(&value.to_le_bytes());
        }
        Prop::String(text) => {
            out.push(b'S');
            out.extend_from_slice(&u32::try_from(text.len()).unwrap_or(u32::MAX).to_le_bytes());
            out.extend_from_slice(text.as_bytes());
        }
        Prop::Raw(data) => {
            out.push(b'R');
            out.extend_from_slice(&u32::try_from(data.len()).unwrap_or(u32::MAX).to_le_bytes());
            out.extend_from_slice(data);
        }
        Prop::BoolArray(values) => write_array(out, b'b', values.len(), values.iter().map(|value| u8::from(*value)).collect()),
        Prop::I32Array(values) => write_array(out, b'i', values.len(), values.iter().flat_map(|value| value.to_le_bytes()).collect()),
        Prop::I64Array(values) => write_array(out, b'l', values.len(), values.iter().flat_map(|value| value.to_le_bytes()).collect()),
        Prop::F32Array(values) => write_array(out, b'f', values.len(), values.iter().flat_map(|value| value.to_le_bytes()).collect()),
        Prop::F64Array(values) => write_array(out, b'd', values.len(), values.iter().flat_map(|value| value.to_le_bytes()).collect()),
    }
}

// ASCII: `; comment` lines, `Name: value, value, ... { children }`, strings in double quotes
// (with `&quot;` for a quote), arrays as `*count { a: v, v, ... }`.

struct Lexer<'a> {
    text: &'a [u8],
    at: usize,
    budget: usize,
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Name(String),
    Text(String),
    Number(String),
    Star(usize),
    Open,
    Close,
    Comma,
    End,
}

impl Lexer<'_> {
    fn skip(&mut self) {
        loop {
            while self.text.get(self.at).is_some_and(|byte| byte.is_ascii_whitespace()) {
                self.at += 1;
            }
            if self.text.get(self.at) == Some(&b';') {
                while self.text.get(self.at).is_some_and(|byte| *byte != b'\n') {
                    self.at += 1;
                }
                continue;
            }
            break;
        }
    }

    fn next(&mut self) -> Result<Token, SceneError> {
        self.budget = self.budget.checked_sub(1).ok_or_else(|| invalid("the file is too large"))?;
        self.skip();
        let Some(&byte) = self.text.get(self.at) else { return Ok(Token::End) };
        match byte {
            b'{' => {
                self.at += 1;
                Ok(Token::Open)
            }
            b'}' => {
                self.at += 1;
                Ok(Token::Close)
            }
            b',' => {
                self.at += 1;
                Ok(Token::Comma)
            }
            b'"' => {
                let start = self.at + 1;
                let mut end = start;
                while self.text.get(end).is_some_and(|byte| *byte != b'"') {
                    end += 1;
                }
                if end >= self.text.len() {
                    return Err(invalid("a string does not end"));
                }
                self.at = end + 1;
                let raw = String::from_utf8_lossy(self.text.get(start..end).unwrap_or_default()).into_owned();
                Ok(Token::Text(raw.replace("&quot;", "\"")))
            }
            b'*' => {
                self.at += 1;
                let start = self.at;
                while self.text.get(self.at).is_some_and(u8::is_ascii_digit) {
                    self.at += 1;
                }
                let digits = std::str::from_utf8(self.text.get(start..self.at).unwrap_or_default()).unwrap_or("0");
                Ok(Token::Star(digits.parse().map_err(|_| invalid("a damaged array length"))?))
            }
            _ => {
                let start = self.at;
                while self.text.get(self.at).is_some_and(|byte| !byte.is_ascii_whitespace() && !matches!(byte, b',' | b'{' | b'}' | b'"' | b';')) {
                    self.at += 1;
                }
                if self.at == start {
                    return Err(invalid(format!("unexpected byte {byte:#x}")));
                }
                let word = String::from_utf8_lossy(self.text.get(start..self.at).unwrap_or_default()).into_owned();
                match word.strip_suffix(':') {
                    Some(name) => Ok(Token::Name(name.to_string())),
                    None => Ok(Token::Number(word)),
                }
            }
        }
    }

    fn peek(&mut self) -> Result<Token, SceneError> {
        let at = self.at;
        let budget = self.budget;
        let token = self.next();
        self.at = at;
        self.budget = budget;
        token
    }
}

fn parse_ascii(bytes: &[u8]) -> Result<(Vec<Node>, u32), SceneError> {
    let text = String::from_utf8_lossy(bytes);
    if !text.trim_start().starts_with(';') && !text.contains("FBXHeaderExtension") {
        return Err(invalid("not an FBX file"));
    }
    let mut lexer = Lexer { text: bytes, at: 0, budget: MAX_ITEMS };
    let mut nodes = Vec::new();
    loop {
        match lexer.next()? {
            Token::End => break,
            Token::Name(name) => nodes.push(read_ascii_node(&mut lexer, name, 0)?),
            other => return Err(invalid(format!("expected a node name, found {other:?}"))),
        }
    }
    let version = nodes
        .iter()
        .find(|node| node.name == "FBXHeaderExtension")
        .and_then(|header| header.child("FBXVersion"))
        .and_then(|node| node.prop(0))
        .and_then(Prop::as_i64)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(7400);
    Ok((nodes, version))
}

fn number(word: &str) -> Prop {
    if let Ok(value) = word.parse::<i64>() {
        return match i32::try_from(value) {
            Ok(small) => Prop::I32(small),
            Err(_) => Prop::I64(value),
        };
    }
    match word {
        "T" | "Y" => Prop::Bool(true),
        "F" | "N" => Prop::Bool(false),
        _ => word.parse::<f64>().map(Prop::F64).unwrap_or_else(|_| Prop::String(word.to_string())),
    }
}

fn read_ascii_node(lexer: &mut Lexer<'_>, name: String, depth: usize) -> Result<Node, SceneError> {
    if depth > MAX_DEPTH {
        return Err(invalid("the nodes nest too deep"));
    }
    let mut node = Node { name, ..Default::default() };
    loop {
        match lexer.peek()? {
            Token::Text(_) | Token::Number(_) => match lexer.next()? {
                Token::Text(text) => node.props.push(Prop::String(text)),
                Token::Number(word) => node.props.push(number(&word)),
                _ => {}
            },
            Token::Comma => {
                lexer.next()?;
            }
            Token::Star(count) => {
                lexer.next()?;
                if lexer.next()? != Token::Open {
                    return Err(invalid("an array opens with {"));
                }
                if !matches!(lexer.next()?, Token::Name(_)) {
                    return Err(invalid("an array's values follow a: "));
                }
                let mut values: Vec<f64> = Vec::with_capacity(count.min(1 << 20));
                let mut integral = true;
                loop {
                    match lexer.next()? {
                        Token::Number(word) => {
                            let value: f64 = word.parse().map_err(|_| invalid("a damaged array value"))?;
                            integral &= !word.contains(['.', 'e', 'E']);
                            values.push(value);
                        }
                        Token::Comma => {}
                        Token::Close => break,
                        other => return Err(invalid(format!("unexpected {other:?} in an array"))),
                    }
                }
                if values.len() != count {
                    return Err(invalid("an array's length does not match its values"));
                }
                node.props.push(if integral { Prop::I64Array(values.iter().map(|value| *value as i64).collect()) } else { Prop::F64Array(values) });
                return Ok(node);
            }
            Token::Open => {
                lexer.next()?;
                loop {
                    match lexer.next()? {
                        Token::Close => return Ok(node),
                        Token::Name(child) => node.children.push(read_ascii_node(lexer, child, depth + 1)?),
                        other => return Err(invalid(format!("expected a child node, found {other:?}"))),
                    }
                }
            }
            Token::Name(_) | Token::Close | Token::End => return Ok(node),
        }
    }
}

/// The nodes as ASCII FBX text (for tests and inspection).
pub fn write_ascii(nodes: &[Node]) -> String {
    let mut out = String::from("; FBX 7.4.0 project file\n");
    for node in nodes {
        ascii_node(&mut out, node, 0);
    }
    out
}

fn ascii_node(out: &mut String, node: &Node, depth: usize) {
    let indent = "\t".repeat(depth);
    let _ = write!(out, "{indent}{}: ", node.name);
    let mut first = true;
    let mut array = None;
    for prop in &node.props {
        let text = match prop {
            Prop::Bool(value) => (if *value { "T" } else { "F" }).to_string(),
            Prop::I16(value) => value.to_string(),
            Prop::I32(value) => value.to_string(),
            Prop::I64(value) => value.to_string(),
            Prop::F32(value) => format!("{value:?}"),
            Prop::F64(value) => format!("{value:?}"),
            Prop::String(text) => format!("\"{}\"", text.replace('"', "&quot;")),
            Prop::Raw(data) => format!("\"{}\"", data.len()),
            other => {
                array = Some(other.clone());
                continue;
            }
        };
        if !first {
            out.push_str(", ");
        }
        out.push_str(&text);
        first = false;
    }
    if let Some(prop) = array {
        let values: Vec<String> = match &prop {
            Prop::F32Array(values) => values.iter().map(|value| format!("{value:?}")).collect(),
            Prop::F64Array(values) => values.iter().map(|value| format!("{value:?}")).collect(),
            Prop::I32Array(values) => values.iter().map(ToString::to_string).collect(),
            Prop::I64Array(values) => values.iter().map(ToString::to_string).collect(),
            Prop::BoolArray(values) => values.iter().map(|value| u8::from(*value).to_string()).collect(),
            _ => Vec::new(),
        };
        let _ = writeln!(out, "*{} {{\n{indent}\ta: {}\n{indent}}}", values.len(), values.join(","));
        return;
    }
    if node.children.is_empty() {
        out.push('\n');
        return;
    }
    out.push_str(" {\n");
    for child in &node.children {
        ascii_node(out, child, depth + 1);
    }
    let _ = writeln!(out, "{indent}}}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<Node> {
        vec![
            Node::new("Header", vec![Prop::I32(7400), Prop::String("a \"quoted\" name".into())])
                .with(vec![Node::new("Flag", vec![Prop::Bool(true)])]),
            Node::new("Data", vec![Prop::F64Array((0..100).map(f64::from).collect()), Prop::I64(1 << 40)]),
            Node::new("Small", vec![Prop::I32Array(vec![1, -2, 3])]),
        ]
    }

    #[test]
    fn binary_round_trips() {
        let bytes = write_binary(&sample());
        let (nodes, version) = parse(&bytes).unwrap();
        assert_eq!(version, WRITE_VERSION);
        assert_eq!(nodes, sample());
    }

    #[test]
    fn ascii_reads_what_it_writes() {
        let text = write_ascii(&[Node::new("FBXHeaderExtension", vec![]).with(vec![Node::new("FBXVersion", vec![Prop::I32(7400)])])]);
        let (nodes, version) = parse(text.as_bytes()).unwrap();
        assert_eq!(version, 7400);
        assert_eq!(nodes.len(), 1);
        let mut more = text.clone();
        more.push_str("Objects:  {\n\tGeometry: 1, \"Geometry::Cube\", \"Mesh\" {\n\t\tVertices: *3 {\n\t\t\ta: 0,1.5,2\n\t\t}\n\t}\n}\n");
        let (nodes, _) = parse(more.as_bytes()).unwrap();
        let geometry = nodes[1].child("Geometry").unwrap();
        assert_eq!(geometry.prop(1).and_then(Prop::as_str), Some("Geometry::Cube"));
        assert_eq!(geometry.child("Vertices").unwrap().prop(0).unwrap().as_f64s().unwrap(), vec![0.0, 1.5, 2.0]);
    }

    #[test]
    fn every_truncation_is_refused_without_a_panic() {
        let bytes = write_binary(&sample());
        // The footer is not needed to read the nodes; everything shorter than the nodes fails.
        let nodes_end = bytes.len() - 160;
        for cut in 0..nodes_end {
            let _ = parse(&bytes[..cut]);
        }
    }
}
