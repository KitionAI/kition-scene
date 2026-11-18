//! The USDA text format: a tokenizer and a parser into layers, prims, and properties.
//!
//! Composition arcs (references, payloads, variants, inherits) are kept as metadata and not
//! followed: a file is read as the prims it defines.

use kition_scene_model::SceneError;

/// Nesting deeper than this (prims or values) is refused.
const MAX_DEPTH: usize = 128;

pub(crate) fn invalid(message: impl Into<String>) -> SceneError {
    SceneError::InvalidFile(format!("USD: {}", message.into()))
}

/// A value: a number, text, a token, an asset path, a scene path, or a tuple, list, or dictionary.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Number(f64),
    Text(String),
    Ident(String),
    Asset(String),
    Path(String),
    Tuple(Vec<Value>),
    List(Vec<Value>),
    Dict(Vec<(String, Value)>),
}

impl Value {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Bool(value) => Some(u8::from(*value).into()),
            Self::Number(number) => Some(*number),
            Self::Ident(word) => match word.as_str() {
                "true" => Some(1.0),
                "false" => Some(0.0),
                "inf" => Some(f64::INFINITY),
                "-inf" => Some(f64::NEG_INFINITY),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(text) | Self::Ident(text) | Self::Asset(text) | Self::Path(text) => Some(text),
            _ => None,
        }
    }

    /// The items of a tuple or a list.
    pub fn items(&self) -> &[Value] {
        match self {
            Self::Tuple(items) | Self::List(items) => items,
            _ => &[],
        }
    }

    /// Every number in the value, flattened (a tuple's, a list of tuples', or a matrix's).
    pub fn numbers(&self) -> Vec<f64> {
        let mut out = Vec::new();
        self.collect_numbers(&mut out);
        out
    }

    fn collect_numbers(&self, out: &mut Vec<f64>) {
        match self {
            Self::Tuple(items) | Self::List(items) => items.iter().for_each(|item| item.collect_numbers(out)),
            other => {
                if let Some(number) = other.as_f64() {
                    out.push(number);
                }
            }
        }
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Self::Dict(entries) => entries.iter().find(|(name, _)| name == key).map(|(_, value)| value),
            _ => None,
        }
    }
}

/// An attribute or relationship.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Property {
    pub name: String,
    /// The value type (`float3`, `token`, ...), with `[]` for arrays; empty for relationships.
    pub type_name: String,
    pub relationship: bool,
    pub uniform: bool,
    pub custom: bool,
    pub value: Option<Value>,
    /// Time samples: (time code, value).
    pub samples: Vec<(f64, Value)>,
    /// The targets an attribute is connected to.
    pub connect: Option<Value>,
    pub metadata: Vec<(String, Value)>,
}

impl Property {
    pub fn meta(&self, key: &str) -> Option<&Value> {
        self.metadata.iter().find(|(name, _)| name == key).map(|(_, value)| value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Specifier {
    Def,
    Over,
    Class,
}

/// A prim and its subtree.
#[derive(Debug, Clone, PartialEq)]
pub struct Prim {
    pub specifier: Specifier,
    pub type_name: String,
    pub name: String,
    pub metadata: Vec<(String, Value)>,
    pub properties: Vec<Property>,
    pub children: Vec<Prim>,
}

impl Prim {
    pub fn property(&self, name: &str) -> Option<&Property> {
        self.properties.iter().find(|property| property.name == name)
    }

    /// A property's default value, else its first time sample.
    pub fn value(&self, name: &str) -> Option<&Value> {
        let property = self.property(name)?;
        property.value.as_ref().or_else(|| property.samples.first().map(|(_, value)| value))
    }

    pub fn number(&self, name: &str) -> Option<f64> {
        self.value(name).and_then(Value::as_f64)
    }

    pub fn meta(&self, key: &str) -> Option<&Value> {
        self.metadata.iter().find(|(name, _)| name == key).map(|(_, value)| value)
    }
}

/// A file: its metadata and root prims.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layer {
    pub metadata: Vec<(String, Value)>,
    pub prims: Vec<Prim>,
}

impl Layer {
    pub fn meta(&self, key: &str) -> Option<&Value> {
        self.metadata.iter().find(|(name, _)| name == key).map(|(_, value)| value)
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Punct(u8),
    Ident(String),
    Number(f64),
    Text(String),
    Asset(String),
    Path(String),
}

fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':' | b'.')
}

fn tokenize(text: &str) -> Result<Vec<Token>, SceneError> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut at = 0usize;
    let byte = |index: usize| bytes.get(index).copied();
    let slice = |from: usize, to: usize| text.get(from..to).ok_or_else(|| invalid("text that is not UTF-8"));
    while let Some(current) = byte(at) {
        match current {
            b' ' | b'\t' | b'\r' | b'\n' => at += 1,
            b'#' => {
                while byte(at).is_some_and(|next| next != b'\n') {
                    at += 1;
                }
            }
            b'(' | b')' | b'[' | b']' | b'{' | b'}' | b'=' | b',' | b':' | b';' => {
                tokens.push(Token::Punct(current));
                at += 1;
            }
            b'"' | b'\'' => {
                let triple = byte(at + 1) == Some(current) && byte(at + 2) == Some(current);
                let quote_length = if triple { 3 } else { 1 };
                at += quote_length;
                let mut value = String::new();
                loop {
                    let Some(next) = byte(at) else { return Err(invalid("a string does not end")) };
                    if next == current && (!triple || (byte(at + 1) == Some(current) && byte(at + 2) == Some(current))) {
                        at += quote_length;
                        break;
                    }
                    if next == b'\n' && !triple {
                        return Err(invalid("a string runs over a line"));
                    }
                    if next == b'\\' {
                        let escaped = byte(at + 1).ok_or_else(|| invalid("a string does not end"))?;
                        value.push(match escaped {
                            b'n' => '\n',
                            b't' => '\t',
                            b'r' => '\r',
                            b'0' => '\0',
                            other => char::from(other),
                        });
                        at += 2;
                        continue;
                    }
                    // Copy one UTF-8 character.
                    let width = text.get(at..).and_then(|rest| rest.chars().next()).map_or(1, char::len_utf8);
                    value.push_str(slice(at, at + width)?);
                    at += width;
                }
                tokens.push(Token::Text(value));
            }
            b'@' => {
                let triple = byte(at + 1) == Some(b'@') && byte(at + 2) == Some(b'@');
                let mark = if triple { "@@@" } else { "@" };
                let start = at + mark.len();
                let rest = text.get(start..).ok_or_else(|| invalid("an asset path does not end"))?;
                let length = rest.find(mark).ok_or_else(|| invalid("an asset path does not end"))?;
                tokens.push(Token::Asset(slice(start, start + length)?.to_string()));
                at = start + length + mark.len();
            }
            b'<' => {
                let rest = text.get(at + 1..).ok_or_else(|| invalid("a path does not end"))?;
                let length = rest.find('>').ok_or_else(|| invalid("a path does not end"))?;
                tokens.push(Token::Path(slice(at + 1, at + 1 + length)?.to_string()));
                at += length + 2;
            }
            b'-' | b'+' | b'.' | b'0'..=b'9' => {
                let start = at;
                if matches!(current, b'-' | b'+') && byte(at + 1).is_some_and(is_ident_start) {
                    at += 1;
                    while byte(at).is_some_and(is_ident) {
                        at += 1;
                    }
                    tokens.push(Token::Ident(slice(start, at)?.to_string()));
                    continue;
                }
                at += 1;
                while let Some(next) = byte(at) {
                    let exponent_sign = matches!(next, b'-' | b'+') && matches!(byte(at - 1), Some(b'e' | b'E'));
                    if next.is_ascii_digit() || matches!(next, b'.' | b'e' | b'E') || exponent_sign {
                        at += 1;
                    } else {
                        break;
                    }
                }
                let word = slice(start, at)?;
                let number: f64 = word.parse().map_err(|_| invalid(format!("a bad number {word:?}")))?;
                tokens.push(Token::Number(number));
            }
            other if is_ident_start(other) => {
                let start = at;
                while byte(at).is_some_and(is_ident) {
                    at += 1;
                }
                tokens.push(Token::Ident(slice(start, at)?.to_string()));
            }
            other => return Err(invalid(format!("an unexpected character {:?}", char::from(other)))),
        }
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.at + offset)
    }

    fn next(&mut self) -> Result<Token, SceneError> {
        let token = self.tokens.get(self.at).cloned().ok_or_else(|| invalid("the file ends early"))?;
        self.at += 1;
        Ok(token)
    }

    fn is(&self, punct: u8) -> bool {
        self.peek() == Some(&Token::Punct(punct))
    }

    fn eat(&mut self, punct: u8) -> bool {
        let found = self.is(punct);
        if found {
            self.at += 1;
        }
        found
    }

    fn expect(&mut self, punct: u8) -> Result<(), SceneError> {
        if self.eat(punct) { Ok(()) } else { Err(invalid(format!("expected {:?} near token {}", char::from(punct), self.at))) }
    }

    fn ident(&mut self) -> Result<String, SceneError> {
        match self.next()? {
            Token::Ident(word) => Ok(word),
            other => Err(invalid(format!("expected a name, found {other:?}"))),
        }
    }

    fn peek_ident(&self) -> Option<&str> {
        match self.peek() {
            Some(Token::Ident(word)) => Some(word),
            _ => None,
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, SceneError> {
        if depth > MAX_DEPTH {
            return Err(invalid("values nest too deep"));
        }
        Ok(match self.next()? {
            Token::Number(number) => Value::Number(number),
            Token::Text(text) => Value::Text(text),
            Token::Ident(word) => Value::Ident(word),
            Token::Path(path) => Value::Path(path),
            Token::Asset(asset) => {
                // A reference's target prim follows its asset.
                if matches!(self.peek(), Some(Token::Path(_))) {
                    self.at += 1;
                }
                Value::Asset(asset)
            }
            Token::Punct(b'(') => Value::Tuple(self.sequence(b')', depth)?),
            Token::Punct(b'[') => Value::List(self.sequence(b']', depth)?),
            Token::Punct(b'{') => Value::Dict(self.dict(depth)?),
            other => return Err(invalid(format!("expected a value, found {other:?}"))),
        })
    }

    fn sequence(&mut self, close: u8, depth: usize) -> Result<Vec<Value>, SceneError> {
        let mut items = Vec::new();
        loop {
            if self.eat(close) {
                return Ok(items);
            }
            items.push(self.value(depth + 1)?);
            if !self.eat(b',') {
                self.expect(close)?;
                return Ok(items);
            }
        }
    }

    /// A dictionary: `key: value` entries (time samples) or `type name = value` ones.
    fn dict(&mut self, depth: usize) -> Result<Vec<(String, Value)>, SceneError> {
        let mut entries = Vec::new();
        loop {
            if self.eat(b'}') {
                return Ok(entries);
            }
            if self.eat(b',') || self.eat(b';') {
                continue;
            }
            let keyed = matches!(self.peek(), Some(Token::Number(_) | Token::Text(_))) && self.peek_at(1) == Some(&Token::Punct(b':'));
            if keyed {
                let key = match self.next()? {
                    Token::Number(number) => number.to_string(),
                    Token::Text(text) => text,
                    _ => String::new(),
                };
                self.expect(b':')?;
                entries.push((key, self.value(depth + 1)?));
                continue;
            }
            // `type[] name = value`
            let type_name = self.ident()?;
            let array = self.eat(b'[');
            if array {
                self.expect(b']')?;
            }
            let key = match self.next()? {
                Token::Ident(word) | Token::Text(word) => word,
                other => return Err(invalid(format!("expected a dictionary key, found {other:?}"))),
            };
            self.expect(b'=')?;
            let value = self.value(depth + 1)?;
            let value = if type_name == "bool" && !array {
                match value.as_f64() {
                    Some(0.0) => Value::Bool(false),
                    Some(1.0) => Value::Bool(true),
                    _ => return Err(invalid("a dictionary bool is neither true nor false")),
                }
            } else {
                value
            };
            entries.push((key, value));
        }
    }

    /// `( ... )` metadata, the opening parenthesis already read.
    fn metadata(&mut self, depth: usize) -> Result<Vec<(String, Value)>, SceneError> {
        let mut entries = Vec::new();
        loop {
            if self.eat(b')') {
                return Ok(entries);
            }
            if self.eat(b',') || self.eat(b';') {
                continue;
            }
            if let Some(Token::Text(_)) = self.peek() {
                let value = self.value(depth + 1)?;
                entries.push(("doc".to_string(), value));
                continue;
            }
            let mut key = self.ident()?;
            if matches!(key.as_str(), "prepend" | "append" | "delete" | "add" | "reorder") {
                key = self.ident()?;
            }
            if self.eat(b'=') {
                entries.push((key, self.value(depth + 1)?));
            } else {
                entries.push((key, Value::Ident(String::new())));
            }
        }
    }

    fn prim(&mut self, specifier: Specifier, depth: usize) -> Result<Prim, SceneError> {
        if depth > MAX_DEPTH {
            return Err(invalid("prims nest too deep"));
        }
        let type_name = if let Some(Token::Ident(_)) = self.peek() { self.ident()? } else { String::new() };
        let name = match self.next()? {
            Token::Text(name) => name,
            other => return Err(invalid(format!("expected a prim name, found {other:?}"))),
        };
        let metadata = if self.eat(b'(') { self.metadata(depth)? } else { Vec::new() };
        self.expect(b'{')?;
        let mut prim = Prim { specifier, type_name, name, metadata, properties: Vec::new(), children: Vec::new() };
        loop {
            if self.eat(b'}') {
                return Ok(prim);
            }
            if self.eat(b';') {
                continue;
            }
            match self.peek_ident() {
                Some("def") | Some("over") | Some("class") => {
                    let specifier = match self.ident()?.as_str() {
                        "def" => Specifier::Def,
                        "over" => Specifier::Over,
                        _ => Specifier::Class,
                    };
                    prim.children.push(self.prim(specifier, depth + 1)?);
                }
                Some("variantSet") => {
                    // Variants are not composed: skip the set.
                    self.at += 1;
                    self.next()?;
                    self.expect(b'=')?;
                    self.expect(b'{')?;
                    self.skip_block(b'}')?;
                }
                Some("reorder") => {
                    self.at += 1;
                    self.ident()?;
                    self.expect(b'=')?;
                    self.value(depth + 1)?;
                }
                _ => self.property(&mut prim, depth)?,
            }
        }
    }

    /// Skips to the bracket closing an open one (already read).
    fn skip_block(&mut self, close: u8) -> Result<(), SceneError> {
        let open = match close {
            b'}' => b'{',
            b')' => b'(',
            _ => b'[',
        };
        let mut level = 1usize;
        while level > 0 {
            match self.next()? {
                Token::Punct(byte) if byte == open => level += 1,
                Token::Punct(byte) if byte == close => level -= 1,
                _ => {}
            }
        }
        Ok(())
    }

    fn property(&mut self, prim: &mut Prim, depth: usize) -> Result<(), SceneError> {
        let mut relationship = false;
        let mut uniform = false;
        let mut custom = false;
        let mut type_name = String::new();
        loop {
            let word = self.ident()?;
            match word.as_str() {
                "custom" => custom = true,
                "uniform" => uniform = true,
                "varying" | "config" | "prepend" | "append" | "delete" | "add" => continue,
                "rel" => {
                    relationship = true;
                    break;
                }
                _ => {
                    type_name = word;
                    if self.eat(b'[') {
                        self.expect(b']')?;
                        type_name.push_str("[]");
                    }
                    break;
                }
            }
        }
        let full = self.ident()?;
        let (name, suffix) = match full.rsplit_once('.') {
            Some((base, suffix @ ("timeSamples" | "connect" | "spline" | "default"))) => (base.to_string(), suffix),
            _ => (full, ""),
        };
        let value = if self.eat(b'=') { Some(self.value(depth + 1)?) } else { None };
        let metadata = if self.eat(b'(') { self.metadata(depth)? } else { Vec::new() };
        let index = match prim.properties.iter().position(|property| property.name == name) {
            Some(index) => index,
            None => {
                prim.properties.push(Property { name, ..Property::default() });
                prim.properties.len() - 1
            }
        };
        let Some(property) = prim.properties.get_mut(index) else { return Ok(()) };
        property.relationship |= relationship;
        property.uniform |= uniform;
        property.custom |= custom;
        if !type_name.is_empty() {
            property.type_name = type_name;
        }
        property.metadata.extend(metadata);
        match suffix {
            "timeSamples" => {
                if let Some(Value::Dict(entries)) = value {
                    let mut samples: Vec<(f64, Value)> = entries
                        .into_iter()
                        .filter_map(|(time, value)| time.parse::<f64>().ok().filter(|time| time.is_finite()).map(|time| (time, value)))
                        .collect();
                    samples.sort_by(|a, b| a.0.total_cmp(&b.0));
                    property.samples = samples;
                }
            }
            "connect" => property.connect = value,
            "spline" => {}
            _ => {
                if value.is_some() {
                    property.value = value;
                }
            }
        }
        Ok(())
    }
}

/// Reads a USDA file.
pub fn parse(text: &str) -> Result<Layer, SceneError> {
    let body = text.strip_prefix('\u{feff}').unwrap_or(text);
    if !body.starts_with("#usda") {
        return Err(invalid("not a USDA file (no #usda header)"));
    }
    let mut parser = Parser { tokens: tokenize(body)?, at: 0 };
    let mut layer = Layer::default();
    if parser.eat(b'(') {
        layer.metadata = parser.metadata(0)?;
    }
    while parser.peek().is_some() {
        let specifier = match parser.ident()?.as_str() {
            "def" => Specifier::Def,
            "over" => Specifier::Over,
            "class" => Specifier::Class,
            other => return Err(invalid(format!("expected a prim, found {other:?}"))),
        };
        layer.prims.push(parser.prim(specifier, 0)?);
    }
    Ok(layer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_prims_properties_samples_and_metadata() {
        let layer = parse(
            r#"#usda 1.0
(
    defaultPrim = "root"
    metersPerUnit = 0.01
    upAxis = "Y"
    customData = { dictionary Tool = { bool generated = 1 } }
    "A comment"
)

def Xform "root" (
    prepend apiSchemas = ["MaterialBindingAPI"]
    kind = "component"
)
{
    # a comment
    double3 xformOp:translate = (1, 2.5, -3e-1)
    float3 xformOp:rotateXYZ.timeSamples = {
        1: (0, 0, 0),
        24: (0, 90, 0),
    }
    uniform token[] xformOpOrder = ["xformOp:translate", "xformOp:rotateXYZ"]
    rel material:binding = </root/Looks/Red>
    texCoord2f[] primvars:st = [(0, 0), (1, 0)] (
        interpolation = "faceVarying"
    )
    token outputs:surface.connect = </root/Shader.outputs:surface>
    asset inputs:file = @textures/a b.png@
    matrix4d xformOp:transform = ( (1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (5, 6, 7, 1) )
    variantSet "look" = {
        "red" { def Sphere "S" {} }
    }
    def "Untyped" {}
    over "Other" {}
}
"#,
        )
        .unwrap();
        assert_eq!(layer.meta("upAxis").and_then(Value::as_str), Some("Y"));
        let root = &layer.prims[0];
        assert_eq!(root.type_name, "Xform");
        assert_eq!(root.value("xformOp:translate").unwrap().numbers(), vec![1.0, 2.5, -0.3]);
        let rotation = root.property("xformOp:rotateXYZ").unwrap();
        assert_eq!(rotation.samples.len(), 2);
        assert_eq!(rotation.samples[1].0, 24.0);
        assert!(root.property("material:binding").unwrap().relationship);
        let st = root.property("primvars:st").unwrap();
        assert_eq!(st.type_name, "texCoord2f[]");
        assert_eq!(st.meta("interpolation").and_then(Value::as_str), Some("faceVarying"));
        assert_eq!(root.property("outputs:surface").unwrap().connect.as_ref().and_then(Value::as_str), Some("/root/Shader.outputs:surface"));
        assert_eq!(root.value("inputs:file").and_then(Value::as_str), Some("textures/a b.png"));
        assert_eq!(root.value("xformOp:transform").unwrap().numbers().len(), 16);
        assert_eq!(root.children.len(), 2);
        assert_eq!(root.children[1].specifier, Specifier::Over);
    }

    #[test]
    fn damaged_text_is_refused_without_a_panic() {
        let text = "#usda 1.0\ndef Mesh \"m\" { int[] faceVertexCounts = [3] point3f[] points = [(0,0,0), (1,0,0), (0,1,0)] }\n";
        assert!(parse(text).is_ok());
        for end in 0..text.len() {
            let _ = parse(&text[..end]);
        }
        let deep = format!("#usda 1.0\ndef X \"a\" {{ float a = {}1{} }}", "(".repeat(5000), ")".repeat(5000));
        assert!(parse(&deep).is_err());
        assert!(parse("#usda 1.0\ndef X \"a\" { float a = \"open").is_err());
    }
}
