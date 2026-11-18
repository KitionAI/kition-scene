//! Deterministic crate 0.8 layers. This is the inverse of our clean-room reader: typed
//! values precede six tables, whose offsets are carried by the bootstrap. Table compression
//! uses LZ4 blocks and compact USD delta codes; scene arrays keep their declared precision.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use kition_scene_model::SceneError;

use super::{MAGIC, MAX_DEPTH, Scalar, kind, shape, spec};
use crate::usda::{Layer, Prim, Property, Specifier, Value, invalid};

mod tables;
mod values;

const MAX_ENTRIES: usize = 1_000_000;
const MAX_BYTES: usize = 512 * 1024 * 1024;

fn index(value: usize) -> Result<u32, SceneError> {
    if value > MAX_ENTRIES {
        return Err(invalid("too many entries to write a binary layer"));
    }
    u32::try_from(value).map_err(|_| invalid("a binary layer index is too large"))
}

fn next_index(length: usize) -> Result<u32, SceneError> {
    index(length.saturating_add(1))?;
    index(length)
}

fn rep(kind: u8, payload: u64, inline: bool, array: bool) -> u64 {
    (u64::from(kind) << 48) | payload | (u64::from(inline) << 62) | (u64::from(array) << 63)
}

#[derive(Default)]
struct Writer {
    bytes: Vec<u8>,
    tokens: Vec<String>,
    token_ids: HashMap<String, u32>,
    strings: Vec<u32>,
    string_ids: HashMap<u32, u32>,
    token_bytes: usize,
    paths: BTreeSet<String>,
    path_ids: HashMap<String, u32>,
    path_rows: Vec<(i32, i32, i32)>,
    fields: Vec<(u32, u64)>,
    sets: Vec<u32>,
    specs: Vec<(u32, u32, u32)>,
    spec_paths: BTreeSet<String>,
}

impl Writer {
    fn append(&mut self, bytes: &[u8]) -> Result<(), SceneError> {
        if self.bytes.len().saturating_add(bytes.len()) > MAX_BYTES {
            return Err(invalid("binary layer exceeds the 512 MiB output limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
    fn u32(&mut self, value: u32) -> Result<(), SceneError> {
        self.append(&value.to_le_bytes())
    }
    fn u64(&mut self, value: u64) -> Result<(), SceneError> {
        self.append(&value.to_le_bytes())
    }
    fn token(&mut self, text: &str) -> Result<u32, SceneError> {
        if let Some(id) = self.token_ids.get(text) {
            return Ok(*id);
        }
        if text.contains('\0') || self.token_bytes.saturating_add(text.len() + 1) > 64 * 1024 * 1024 {
            return Err(invalid("a binary layer token is invalid or its token table is too large"));
        }
        let id = next_index(self.tokens.len())?;
        self.token_bytes += text.len() + 1;
        self.tokens.push(text.to_string());
        self.token_ids.insert(text.to_string(), id);
        Ok(id)
    }
    fn string(&mut self, text: &str) -> Result<u32, SceneError> {
        let token = self.token(text)?;
        if let Some(id) = self.string_ids.get(&token) {
            return Ok(*id);
        }
        let id = next_index(self.strings.len())?;
        self.strings.push(token);
        self.string_ids.insert(token, id);
        Ok(id)
    }
    fn path(&mut self, path: &str) -> Result<(), SceneError> {
        if !path.starts_with('/')
            || path.len() > 16 * 1024
            || path.contains(['[', ']', '{', '}'])
            || path.bytes().filter(|byte| *byte == b'/').count() > MAX_DEPTH
            || path.contains("//")
            || path.matches('.').count() > 1
        {
            return Err(invalid("unsupported binary layer path"));
        }
        if path != "/" {
            let (prim_path, property) = path.split_once('.').map_or((path, None), |(prim, property)| (prim, Some(property)));
            if !prim_path.trim_start_matches('/').split('/').all(valid_name) || property.is_some_and(|property| !property.split(':').all(valid_name))
            {
                return Err(invalid("invalid binary layer path name"));
            }
        }
        let mut current = path;
        loop {
            if !self.paths.contains(current) {
                next_index(self.paths.len())?;
                self.paths.insert(current.to_string());
            }
            if current == "/" {
                break;
            }
            current = parent(current);
        }
        Ok(())
    }
    fn spec_path(&mut self, path: &str) -> Result<(), SceneError> {
        self.path(path)?;
        next_index(self.spec_paths.len())?;
        if !self.spec_paths.insert(path.to_string()) {
            return Err(invalid("duplicate binary layer spec path"));
        }
        Ok(())
    }
    fn collect_value(&mut self, value: &Value, depth: usize) -> Result<(), SceneError> {
        if depth > MAX_DEPTH {
            return Err(invalid("binary layer values nest too deep"));
        }
        match value {
            Value::Path(path) => self.path(path)?,
            Value::Number(number) if !number.is_finite() => return Err(invalid("binary layer numbers must be finite")),
            Value::List(items) | Value::Tuple(items) => {
                if items.len() > MAX_BYTES / 8 {
                    return Err(invalid("a binary layer array is too large"));
                }
                for value in items {
                    self.collect_value(value, depth + 1)?;
                }
            }
            Value::Dict(entries) => {
                index(entries.len())?;
                for (_, value) in entries {
                    self.collect_value(value, depth + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn collect(&mut self, prim: &Prim, owner: &str, depth: usize) -> Result<(), SceneError> {
        if depth > MAX_DEPTH {
            return Err(invalid("binary layer prims nest too deep"));
        }
        if prim.name.is_empty() || prim.name.contains(['/', '.', '\0']) {
            return Err(invalid("invalid binary layer prim name"));
        }
        let path = format!("{owner}/{}", prim.name);
        self.spec_path(&path)?;
        for (_, value) in &prim.metadata {
            self.collect_value(value, 0)?;
        }
        for property in &prim.properties {
            index(property.samples.len())?;
            if property.name.is_empty() || property.name.contains(['/', '.', '\0']) {
                return Err(invalid("invalid binary layer property name"));
            }
            self.spec_path(&format!("{path}.{}", property.name))?;
            for (_, value) in &property.metadata {
                self.collect_value(value, 0)?;
            }
            for value in property.value.iter().chain(property.connect.iter()) {
                self.collect_value(value, 0)?;
            }
            for (time, value) in &property.samples {
                if !time.is_finite() {
                    return Err(invalid("binary layer sample times must be finite"));
                }
                self.collect_value(value, 0)?;
            }
        }
        for child in &prim.children {
            self.collect(child, &path, depth + 1)?;
        }
        Ok(())
    }
    fn build_paths(&mut self, path: &str, children: &BTreeMap<String, Vec<String>>, sibling: bool) -> Result<(), SceneError> {
        let slot = next_index(self.path_rows.len())?;
        let name = if path == "/" { "" } else { path.rsplit(['/', '.']).next().unwrap_or_default() };
        let element = self.token(name)? as i32 * if path.contains('.') { -1 } else { 1 };
        self.path_ids.insert(path.to_string(), slot);
        self.path_rows.push((slot as i32, element, -2));
        let kids = children.get(path).map(Vec::as_slice).unwrap_or(&[]);
        for (i, child) in kids.iter().enumerate() {
            self.build_paths(child, children, i + 1 < kids.len())?;
        }
        let span = self.path_rows.len() - slot as usize;
        let jump = match (kids.is_empty(), sibling) {
            (true, true) => 0,
            (true, false) => -2,
            (false, false) => -1,
            (false, true) => index(span)? as i32,
        };
        if let Some(row) = self.path_rows.get_mut(slot as usize) {
            row.2 = jump;
        }
        Ok(())
    }
    fn field(&mut self, name: &str, value: u64, fields: &mut Vec<u32>) -> Result<(), SceneError> {
        let name = self.token(name)?;
        fields.push(next_index(self.fields.len())?);
        self.fields.push((name, value));
        Ok(())
    }
    fn token_field(&mut self, name: &str, value: &str, fields: &mut Vec<u32>) -> Result<(), SceneError> {
        let value = rep(kind::TOKEN, u64::from(self.token(value)?), true, false);
        self.field(name, value, fields)
    }
    fn names(&mut self, name: &str, names: Vec<String>, fields: &mut Vec<u32>) -> Result<(), SceneError> {
        if names.is_empty() {
            return Ok(());
        }
        let offset = self.bytes.len() as u64;
        self.u64(names.len() as u64)?;
        for name in names {
            let token = self.token(&name)?;
            self.u32(token)?;
        }
        self.field(name, rep(kind::TOKEN_VECTOR, offset, false, false), fields)
    }
    fn metadata(&mut self, entries: &[(String, Value)], fields: &mut Vec<u32>) -> Result<(), SceneError> {
        for (name, value) in entries {
            let value = match name.as_str() {
                "apiSchemas" => self.list_op(value, kind::TOKEN_LIST_OP)?,
                "customData" => self.dictionary(value, 0)?,
                "doc" | "comment" => self.value(value, "string")?,
                "elementSize" => self.value(value, "int")?,
                "metersPerUnit" | "framesPerSecond" | "timeCodesPerSecond" | "startTimeCode" | "endTimeCode" => self.value(value, "double")?,
                "defaultPrim" | "upAxis" | "interpolation" | "kind" => self.value(value, "token")?,
                _ => return Err(invalid(format!("unsupported binary layer metadata {name}"))),
            };
            self.field(name, value, fields)?;
        }
        Ok(())
    }
    fn spec(&mut self, path: &str, kind: u32, fields: &[u32]) -> Result<(), SceneError> {
        if !self.spec_paths.remove(path) {
            return Err(invalid("missing binary layer spec path"));
        }
        index(self.sets.len().saturating_add(fields.len()).saturating_add(1))?;
        let set = index(self.sets.len())?;
        let path = *self.path_ids.get(path).ok_or_else(|| invalid("missing binary layer path"))?;
        self.sets.extend_from_slice(fields);
        self.sets.push(u32::MAX);
        next_index(self.specs.len())?;
        self.specs.push((path, set, kind));
        Ok(())
    }
    fn prim(&mut self, prim: &Prim, owner: &str) -> Result<(), SceneError> {
        let path = format!("{owner}/{}", prim.name);
        let mut fields = Vec::new();
        let specifier = match prim.specifier {
            Specifier::Def => 0,
            Specifier::Over => 1,
            Specifier::Class => 2,
        };
        self.field("specifier", rep(kind::SPECIFIER, specifier, true, false), &mut fields)?;
        self.token_field("typeName", &prim.type_name, &mut fields)?;
        self.names("primChildren", prim.children.iter().map(|p| p.name.clone()).collect(), &mut fields)?;
        self.names("properties", prim.properties.iter().map(|p| p.name.clone()).collect(), &mut fields)?;
        self.metadata(&prim.metadata, &mut fields)?;
        self.spec(&path, spec::PRIM, &fields)?;
        for property in &prim.properties {
            self.property(property, &path)?;
        }
        for child in &prim.children {
            self.prim(child, &path)?;
        }
        Ok(())
    }
    fn property(&mut self, property: &Property, owner: &str) -> Result<(), SceneError> {
        let path = format!("{owner}.{}", property.name);
        let mut fields = Vec::new();
        self.field("custom", rep(kind::BOOL, u64::from(property.custom), true, false), &mut fields)?;
        self.field("variability", rep(kind::VARIABILITY, u64::from(property.uniform || property.relationship), true, false), &mut fields)?;
        if !property.relationship {
            self.token_field("typeName", &property.type_name, &mut fields)?;
        }
        if let Some(value) = &property.value {
            let value = if property.relationship { self.list_op(value, kind::PATH_LIST_OP)? } else { self.value(value, &property.type_name)? };
            self.field(if property.relationship { "targetPaths" } else { "default" }, value, &mut fields)?;
        }
        if let Some(value) = &property.connect {
            let value = self.list_op(value, kind::PATH_LIST_OP)?;
            self.field("connectionPaths", value, &mut fields)?;
        }
        if !property.samples.is_empty() {
            let value = self.samples(&property.samples, &property.type_name)?;
            self.field("timeSamples", value, &mut fields)?;
        }
        self.metadata(&property.metadata, &mut fields)?;
        self.spec(&path, if property.relationship { spec::RELATIONSHIP } else { spec::ATTRIBUTE }, &fields)
    }
    fn finish(mut self) -> Result<Vec<u8>, SceneError> {
        let mut sections = Vec::new();
        let start = self.bytes.len() as u64;
        let token_text: Vec<u8> = self.tokens.iter().flat_map(|t| t.bytes().chain([0])).collect();
        self.u64(self.tokens.len() as u64)?;
        self.u64(token_text.len() as u64)?;
        self.compressed(&token_text)?;
        sections.push(("TOKENS", start, self.bytes.len() as u64 - start));
        let start = self.bytes.len() as u64;
        self.u64(self.strings.len() as u64)?;
        for token in std::mem::take(&mut self.strings) {
            self.u32(token)?;
        }
        sections.push(("STRINGS", start, self.bytes.len() as u64 - start));
        let start = self.bytes.len() as u64;
        let fields = std::mem::take(&mut self.fields);
        self.u64(fields.len() as u64)?;
        self.ints(fields.iter().map(|(name, _)| *name as i32))?;
        let reps: Vec<u8> = fields.iter().flat_map(|(_, rep)| rep.to_le_bytes()).collect();
        self.compressed(&reps)?;
        sections.push(("FIELDS", start, self.bytes.len() as u64 - start));
        let start = self.bytes.len() as u64;
        let sets = std::mem::take(&mut self.sets);
        self.u64(sets.len() as u64)?;
        self.ints(sets.iter().map(|v| *v as i32))?;
        sections.push(("FIELDSETS", start, self.bytes.len() as u64 - start));
        let start = self.bytes.len() as u64;
        let paths = std::mem::take(&mut self.path_rows);
        self.u64(paths.len() as u64)?;
        self.u64(paths.len() as u64)?;
        self.ints(paths.iter().map(|r| r.0))?;
        self.ints(paths.iter().map(|r| r.1))?;
        self.ints(paths.iter().map(|r| r.2))?;
        sections.push(("PATHS", start, self.bytes.len() as u64 - start));
        let start = self.bytes.len() as u64;
        let specs = std::mem::take(&mut self.specs);
        self.u64(specs.len() as u64)?;
        self.ints(specs.iter().map(|r| r.0 as i32))?;
        self.ints(specs.iter().map(|r| r.1 as i32))?;
        self.ints(specs.iter().map(|r| r.2 as i32))?;
        sections.push(("SPECS", start, self.bytes.len() as u64 - start));
        let toc = self.bytes.len() as u64;
        self.u64(sections.len() as u64)?;
        for (name, start, size) in sections {
            let mut label = [0; 16];
            for (out, byte) in label.iter_mut().zip(name.bytes()) {
                *out = byte;
            }
            self.append(&label)?;
            self.u64(start)?;
            self.u64(size)?;
        }
        if let Some(slot) = self.bytes.get_mut(16..24) {
            slot.copy_from_slice(&toc.to_le_bytes());
        }
        Ok(self.bytes)
    }
    fn compressed(&mut self, bytes: &[u8]) -> Result<(), SceneError> {
        if bytes.len() > MAX_BYTES {
            return Err(invalid("a binary table exceeds the 512 MiB output limit"));
        }
        // lz4_flex 0.11 computes its bound as input_len * 110 / 100. Keep that
        // multiplication within usize on WASM32; larger tables remain one valid literal
        // block, retaining the layer limits without relying on a compressor allocation.
        if bytes.len() > 32 * 1024 * 1024 {
            let extra = bytes.len() - 15;
            self.u64((2 + extra / 255 + 1 + bytes.len()) as u64)?;
            self.append(&[0, 0xf0])?;
            for _ in 0..extra / 255 {
                self.append(&[255])?;
            }
            self.append(&[(extra % 255) as u8])?;
            return self.append(bytes);
        }
        let block = lz4_flex::block::compress(bytes);
        self.u64(block.len() as u64 + 1)?;
        // USD's zero chunk-count byte denotes one raw LZ4 block, without a size prefix.
        self.append(&[0])?;
        self.append(&block)
    }
    fn ints(&mut self, values: impl ExactSizeIterator<Item = i32> + Clone) -> Result<(), SceneError> {
        self.compressed(&tables::integers(values)?)
    }
}

fn parent(path: &str) -> &str {
    if let Some((owner, _)) = path.rsplit_once('.') {
        return owner;
    }
    path.rsplit_once('/').map(|(owner, _)| if owner.is_empty() { "/" } else { owner }).unwrap_or("/")
}

fn valid_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|first| first.is_ascii_alphabetic() || first == b'_') && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// Writes the layer types authored by Kition Scene as a binary USD crate (version 0.8.0). Unknown
/// metadata and types are refused, rather than silently dropping scene information. The
/// writer limits nesting, table counts, and output size before emitting an invalid file.
pub fn write(layer: &Layer) -> Result<Vec<u8>, SceneError> {
    let mut writer = Writer::default();
    writer.append(MAGIC)?;
    writer.append(&[0, 8, 0, 0, 0, 0, 0, 0])?;
    writer.append(&[0; 72])?;
    writer.token("")?;
    writer.spec_path("/")?;
    for (_, value) in &layer.metadata {
        writer.collect_value(value, 0)?;
    }
    for prim in &layer.prims {
        writer.collect(prim, "", 0)?;
    }
    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for path in &writer.paths {
        if path != "/" {
            children.entry(parent(path).into()).or_default().push(path.clone());
        }
    }
    writer.build_paths("/", &children, false)?;
    let mut fields = Vec::new();
    writer.names("primChildren", layer.prims.iter().map(|p| p.name.clone()).collect(), &mut fields)?;
    writer.metadata(&layer.metadata, &mut fields)?;
    writer.spec("/", spec::PSEUDO_ROOT, &fields)?;
    for prim in &layer.prims {
        writer.prim(prim, "")?;
    }
    writer.finish()
}
