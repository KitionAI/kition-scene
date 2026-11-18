//! Groomed hair as bytes (an object's hair blob in the replica): for each system with groomed
//! strands, its name, then its strands' points, little-endian.

use glam::Vec3;

use crate::hair::Strand;

const MAGIC: &[u8; 4] = b"KCHR";
const VERSION: u32 = 1;
/// The most points a blob may hold.
const MAX_POINTS: usize = 50_000_000;

fn put_u32(out: &mut Vec<u8>, value: usize) -> Result<(), String> {
    let value = u32::try_from(value).map_err(|_| "too much hair to save".to_string())?;
    out.extend(value.to_le_bytes());
    Ok(())
}

/// Encodes `(system name, strands)` pairs. Every strand of a system has the same point count.
pub fn encode(systems: &[(&str, &[Strand])]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    out.extend(MAGIC);
    put_u32(&mut out, VERSION as usize)?;
    put_u32(&mut out, systems.len())?;
    for (name, strands) in systems {
        put_u32(&mut out, name.len())?;
        out.extend(name.as_bytes());
        put_u32(&mut out, strands.len())?;
        let points = strands.first().map_or(0, Vec::len);
        if strands.iter().any(|strand| strand.len() != points) {
            return Err(format!("the strands of {name} differ in length"));
        }
        put_u32(&mut out, points)?;
        for point in strands.iter().flatten() {
            for value in point.to_array() {
                out.extend(value.to_le_bytes());
            }
        }
    }
    Ok(out)
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, count: usize) -> Result<&[u8], String> {
        let end = self.at.checked_add(count).ok_or("damaged hair")?;
        let slice = self.bytes.get(self.at..end).ok_or("the hair is cut short")?;
        self.at = end;
        Ok(slice)
    }

    fn u32(&mut self) -> Result<u32, String> {
        let bytes = self.take(4)?;
        let array: [u8; 4] = bytes.try_into().map_err(|_| "damaged hair".to_string())?;
        Ok(u32::from_le_bytes(array))
    }
}

/// Decodes what [`encode`] wrote.
pub fn decode(bytes: &[u8]) -> Result<Vec<(String, Vec<Strand>)>, String> {
    let mut reader = Reader { bytes, at: 0 };
    if reader.take(4)? != MAGIC {
        return Err("not hair".into());
    }
    if reader.u32()? != VERSION {
        return Err("hair of an unknown version".into());
    }
    let systems = reader.u32()? as usize;
    let mut out = Vec::new();
    let mut total = 0usize;
    for _ in 0..systems {
        let name_length = reader.u32()? as usize;
        let name = String::from_utf8(reader.take(name_length)?.to_vec()).map_err(|_| "damaged hair name")?;
        let strands = reader.u32()? as usize;
        let points = reader.u32()? as usize;
        total = total.saturating_add(strands.saturating_mul(points));
        if total > MAX_POINTS || strands.saturating_mul(points).saturating_mul(12) > bytes.len() {
            return Err("the hair claims more points than it holds".into());
        }
        let mut list = Vec::with_capacity(strands);
        for _ in 0..strands {
            let mut strand = Vec::with_capacity(points);
            for _ in 0..points {
                let raw = reader.take(12)?;
                let value = |slot: usize| raw.get(slot * 4..slot * 4 + 4).and_then(|four| four.try_into().ok()).map_or(0.0, f32::from_le_bytes);
                let point = Vec3::new(value(0), value(1), value(2));
                if !point.is_finite() {
                    return Err("a hair point is not a number".into());
                }
                strand.push(point);
            }
            list.push(strand);
        }
        out.push((name, list));
    }
    if reader.at != bytes.len() {
        return Err("hair has trailing bytes".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hair_round_trips_and_damage_is_refused() {
        let strands = vec![vec![Vec3::ZERO, Vec3::Z], vec![Vec3::X, Vec3::new(1.0, 0.0, 2.0)]];
        let bytes = encode(&[("Fur", &strands)]).unwrap();
        assert_eq!(decode(&bytes).unwrap(), vec![("Fur".to_string(), strands)]);
        for cut in 0..bytes.len() {
            assert!(decode(&bytes[..cut]).is_err(), "cut at {cut}");
        }
        let mut huge = bytes.clone();
        huge[19..23].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(&huge).is_err());
    }
}
