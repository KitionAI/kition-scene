//! Typed values carried by the crate writer. The type name, not the shape of a tuple,
//! selects the on-disk representation (notably half scales and quaternion ordering).

use super::*;

fn value_kind(name: &str) -> Result<u8, SceneError> {
    Ok(match name {
        "bool" => kind::BOOL,
        "uchar" => kind::UCHAR,
        "int" => kind::INT,
        "uint" => kind::UINT,
        "float" => kind::FLOAT,
        "double" => kind::DOUBLE,
        "half" => kind::HALF,
        "token" => kind::TOKEN,
        "string" => kind::STRING,
        "asset" => kind::ASSET,
        "float2" | "texCoord2f" => kind::VEC2F,
        "float3" | "point3f" | "normal3f" | "vector3f" | "color3f" => kind::VEC3F,
        "float4" | "color4f" => kind::VEC4F,
        "double2" | "texCoord2d" => kind::VEC2D,
        "double3" | "point3d" | "normal3d" | "vector3d" | "color3d" => kind::VEC3D,
        "double4" | "color4d" => kind::VEC4D,
        "half2" => kind::VEC2H,
        "half3" => kind::VEC3H,
        "half4" => kind::VEC4H,
        "matrix2d" => kind::MATRIX2D,
        "matrix3d" => kind::MATRIX3D,
        "matrix4d" => kind::MATRIX4D,
        "quatf" => kind::QUATF,
        "quatd" => kind::QUATD,
        "quath" => kind::QUATH,
        _ => return Err(invalid(format!("unsupported binary layer type {name}"))),
    })
}

impl Writer {
    pub(super) fn value(&mut self, value: &Value, name: &str) -> Result<u64, SceneError> {
        let array = name.ends_with("[]");
        let kind = value_kind(name.trim_end_matches("[]"))?;
        if array {
            let Value::List(items) = value else {
                return Err(invalid("binary array value is not a list"));
            };
            if items.is_empty() {
                return Ok(rep(kind, 0, false, true));
            }
            let offset = self.bytes.len() as u64;
            self.u64(items.len() as u64)?;
            for item in items {
                self.element(item, kind)?;
            }
            return Ok(rep(kind, offset, false, true));
        }
        let small = match kind {
            kind::BOOL => Some(u64::from(boolean(value)?)),
            kind::UCHAR => Some(integer(value, 0.0, 255.0)? as u64),
            kind::INT => Some(u64::from(integer(value, i32::MIN.into(), i32::MAX.into())? as i32 as u32)),
            kind::UINT => Some(integer(value, 0.0, u32::MAX.into())? as u64),
            kind::FLOAT => Some(u64::from(float(value)?.to_bits())),
            kind::HALF => Some(u64::from(half(float(value)?)?)),
            kind::TOKEN | kind::ASSET | kind::STRING => Some(u64::from(self.text_index(value, kind)?)),
            _ => None,
        };
        if let Some(payload) = small {
            return Ok(rep(kind, payload, true, false));
        }
        let offset = self.bytes.len() as u64;
        self.element(value, kind)?;
        Ok(rep(kind, offset, false, false))
    }
    fn text_index(&mut self, value: &Value, kind: u8) -> Result<u32, SceneError> {
        let text = value.as_str().ok_or_else(|| invalid("binary text value is not text"))?;
        if kind == kind::STRING { self.string(text) } else { self.token(text) }
    }
    fn element(&mut self, value: &Value, kind: u8) -> Result<(), SceneError> {
        if let Some((scalar, count)) = shape(kind) {
            let mut values = Vec::with_capacity(count);
            components(value, count, &mut values)?;
            if values.len() != count {
                return Err(invalid("binary tuple has the wrong number of components"));
            }
            if matches!(kind, kind::QUATF | kind::QUATD | kind::QUATH) {
                values.rotate_left(1);
            }
            for value in values {
                if !value.is_finite() {
                    return Err(invalid("binary tuple components must be finite"));
                }
                match scalar {
                    Scalar::Double => self.append(&value.to_le_bytes())?,
                    Scalar::Float => self.append(&float(&Value::Number(value))?.to_le_bytes())?,
                    Scalar::Half => self.append(&half(float(&Value::Number(value))?)?.to_le_bytes())?,
                    Scalar::Int => self.append(&(integer(&Value::Number(value), i32::MIN.into(), i32::MAX.into())? as i32).to_le_bytes())?,
                }
            }
            return Ok(());
        }
        match kind {
            kind::BOOL => self.append(&[u8::from(boolean(value)?)])?,
            kind::UCHAR => self.append(&[integer(value, 0.0, 255.0)? as u8])?,
            kind::INT => self.append(&(integer(value, i32::MIN.into(), i32::MAX.into())? as i32).to_le_bytes())?,
            kind::UINT => self.u32(integer(value, 0.0, u32::MAX.into())? as u32)?,
            kind::HALF => self.append(&half(float(value)?)?.to_le_bytes())?,
            kind::FLOAT => self.append(&float(value)?.to_le_bytes())?,
            kind::DOUBLE => self.append(&number(value)?.to_le_bytes())?,
            kind::STRING | kind::TOKEN | kind::ASSET => {
                let id = self.text_index(value, kind)?;
                self.u32(id)?;
            }
            _ => return Err(invalid("unsupported binary array element")),
        }
        Ok(())
    }
    pub(super) fn list_op(&mut self, value: &Value, kind: u8) -> Result<u64, SceneError> {
        let items = match value {
            Value::List(items) => items.as_slice(),
            _ => std::slice::from_ref(value),
        };
        let offset = self.bytes.len() as u64;
        // IsExplicit | HasExplicitItems, avoiding any dependency on another layer's lists.
        self.append(&[3])?;
        self.u64(items.len() as u64)?;
        for item in items {
            let text = item.as_str().ok_or_else(|| invalid("binary list operation requires names or paths"))?;
            let id = if kind == kind::PATH_LIST_OP {
                *self.path_ids.get(text).ok_or_else(|| invalid("binary list operation has an unknown path"))?
            } else {
                self.token(text)?
            };
            self.u32(id)?;
        }
        Ok(rep(kind, offset, false, false))
    }
    pub(super) fn dictionary(&mut self, value: &Value, depth: usize) -> Result<u64, SceneError> {
        if depth > MAX_DEPTH {
            return Err(invalid("binary dictionary nests too deep"));
        }
        let Value::Dict(entries) = value else {
            return Err(invalid("binary custom data must be a dictionary"));
        };
        let mut encoded = Vec::new();
        for (key, value) in entries {
            let key = self.string(key)?;
            let value = match value {
                Value::Dict(_) => self.dictionary(value, depth + 1)?,
                Value::Bool(_) => self.value(value, "bool")?,
                Value::Number(_) => self.value(value, "double")?,
                Value::Text(_) | Value::Ident(_) => self.value(value, "string")?,
                _ => return Err(invalid("unsupported binary custom data value")),
            };
            encoded.push((key, value));
        }
        let offset = self.bytes.len() as u64;
        self.u64(encoded.len() as u64)?;
        for (key, value) in encoded {
            self.u32(key)?;
            self.u64(8)?;
            self.u64(value)?;
        }
        Ok(rep(kind::DICTIONARY, offset, false, false))
    }
    pub(super) fn samples(&mut self, samples: &[(f64, Value)], name: &str) -> Result<u64, SceneError> {
        let mut values = Vec::new();
        let mut previous = f64::NEG_INFINITY;
        for (time, value) in samples {
            if *time <= previous || !time.is_finite() {
                return Err(invalid("binary sample times must increase"));
            }
            previous = *time;
            values.push(self.value(value, name)?);
        }
        let times = self.bytes.len() as u64;
        self.u64(samples.len() as u64)?;
        for (time, _) in samples {
            self.append(&time.to_le_bytes())?;
        }
        let offset = self.bytes.len() as u64;
        self.u64(8)?;
        self.u64(rep(kind::DOUBLE_VECTOR, times, false, false))?;
        self.u64(8)?;
        self.u64(values.len() as u64)?;
        for value in values {
            self.u64(value)?;
        }
        Ok(rep(kind::TIME_SAMPLES, offset, false, false))
    }
}

/// Stop at the expected width instead of flattening an arbitrarily large hostile tuple.
fn components(value: &Value, count: usize, out: &mut Vec<f64>) -> Result<(), SceneError> {
    match value {
        Value::Tuple(items) => {
            for value in items {
                components(value, count, out)?;
            }
        }
        _ => {
            if out.len() >= count {
                return Err(invalid("binary tuple has too many components"));
            }
            out.push(number(value)?);
        }
    }
    Ok(())
}

fn number(value: &Value) -> Result<f64, SceneError> {
    value.as_f64().filter(|v| v.is_finite()).ok_or_else(|| invalid("binary numeric value must be finite"))
}
fn float(value: &Value) -> Result<f32, SceneError> {
    let value = number(value)? as f32;
    if value.is_finite() { Ok(value) } else { Err(invalid("binary float is out of range")) }
}
fn integer(value: &Value, min: f64, max: f64) -> Result<f64, SceneError> {
    let value = number(value)?;
    if value >= min && value <= max && value.fract() == 0.0 { Ok(value) } else { Err(invalid("binary integer is out of range")) }
}
fn boolean(value: &Value) -> Result<bool, SceneError> {
    match number(value)? {
        0.0 => Ok(false),
        1.0 => Ok(true),
        _ => Err(invalid("binary bool must be zero or one")),
    }
}

/// IEEE 754 binary16, rounded to nearest with ties to even. Values that would overflow the
/// format are refused instead of generating infinite joint scales.
fn half(value: f32) -> Result<u16, SceneError> {
    if !value.is_finite() || value.abs() >= 65520.0 {
        return Err(invalid("binary half is out of range"));
    }
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 255) as i32 - 127;
    if exponent < -25 {
        return Ok(sign);
    }
    let mantissa = (bits & 0x7f_ffff) | 0x80_0000;
    let shift = if exponent < -14 { (-exponent - 1) as u32 } else { 13 };
    let rounded = (mantissa + (1u32 << (shift - 1)) - 1 + ((mantissa >> shift) & 1)) >> shift;
    let magnitude = if exponent < -14 { rounded } else { (((exponent + 14) as u32) << 10) + rounded };
    Ok(sign | magnitude as u16)
}
