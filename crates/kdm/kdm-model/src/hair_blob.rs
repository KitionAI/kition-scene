//! Groomed hair as a blob: each groomed system's parent strands by system name, in the object's
//! own space (the particles crate's `KCHR` format). Hair as generated is not stored: it comes
//! from the settings and the seed again.

use kition_scene_model::Object;
use kition_scene_model::particles::blob;

/// The blob's MIME type.
pub const MIME: &str = "application/vnd.kition.scene.hair";

pub use blob::encode;

/// Puts the groomed strands in `bytes` on `object`'s systems by name. A groom that no longer
/// fits its system (a peer changed the count or the segments at the same time) is dropped, and
/// the hair shows as generated again.
pub fn apply(object: &mut Object, bytes: &[u8]) -> Result<(), String> {
    for (name, strands) in blob::decode(bytes)? {
        let Some(system) = object.particles.iter_mut().find(|system| system.name == name) else { continue };
        system.groomed = strands;
        if system.check().is_err() {
            system.groomed.clear();
        }
    }
    Ok(())
}
