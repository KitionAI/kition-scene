//! Engine ids per replica (`docs/design.md` section 7.1): every actor mints new object and
//! material ids from its own range, so two people adding at once never mint the same id. The ids
//! a range has issued are kept in the root's `keys` map (one register per range, written only by
//! that range's actor), so an id is never minted twice, even after the object that had it was
//! deleted.

use kition_kit::kdm::{ActorId, Replica};
use kition_scene_model::{Id, IdAllocator, Scene};
use serde_json::Value;

use crate::root_id;

/// Ids below this are the creating replica's (a new file's starter scene); ranges start above.
pub const FIRST_RANGE: Id = 1 << 20;

/// Ids from here up are the repair's: a key a merge found twice moves there. No range reaches it.
pub const REKEY_FLOOR: Id = 0xF000_0000;

/// The ids one range holds. A session that mints more goes on above the scene's highest id, and
/// the repair moves a key that then meets another's.
pub const RANGE: Id = 1 << 14;

/// How many ranges there are: two sessions share one with a chance of 1 in 245,696.
pub const SLOTS: u32 = (REKEY_FLOOR - FIRST_RANGE) / RANGE;

/// The range slot of `actor`: 0 to [`SLOTS`] - 1.
pub fn slot(actor: ActorId) -> u32 {
    u32::try_from(actor.get() % u64::from(SLOTS)).unwrap_or(0)
}

/// The ids `actor` mints: `[start, end)`.
pub fn range(actor: ActorId) -> (Id, Id) {
    let start = FIRST_RANGE.saturating_add(slot(actor).saturating_mul(RANGE));
    (start, start.saturating_add(RANGE).min(REKEY_FLOOR))
}

/// The `keys` map's entry for `actor`'s range.
pub fn key_of(actor: ActorId) -> String {
    slot(actor).to_string()
}

/// The next id `actor` mints in `replica`: past every id its range has issued, and past every id
/// of its range in `scene`.
pub fn next(replica: &Replica, actor: ActorId, scene: &Scene) -> Id {
    let (start, end) = range(actor);
    let issued = replica
        .map_get(&root_id(), "keys", &key_of(actor))
        .ok()
        .and_then(|value| value.as_u64())
        .and_then(|id| u32::try_from(id).ok())
        .unwrap_or(start);
    let used = scene
        .objects
        .iter()
        .map(|object| object.id)
        .chain(scene.materials.iter().map(|material| material.id))
        .chain(crate::nodes::ids(scene))
        .filter(|id| (start..end).contains(id))
        .max();
    issued.max(used.map_or(start, |id| id.saturating_add(1))).clamp(start, end)
}

/// Makes `scene` mint ids as `actor` does in `replica`.
pub fn allocate(replica: &Replica, actor: ActorId, scene: &mut Scene) {
    let (_, end) = range(actor);
    let next = next(replica, actor, scene);
    scene.allocator = IdAllocator(Some((next, end)));
    let highest = scene
        .objects
        .iter()
        .map(|object| object.id)
        .chain(scene.materials.iter().map(|material| material.id))
        .chain(crate::nodes::ids(scene))
        .max()
        .unwrap_or(0);
    scene.next_id = scene.next_id.max(highest.saturating_add(1)).max(1);
}

/// The value the `keys` map holds for `next`.
pub fn issued(next: Id) -> Value {
    Value::from(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_are_disjoint_and_above_the_starter() {
        let a = range(ActorId::from_u64(1).unwrap());
        let b = range(ActorId::from_u64(2).unwrap());
        assert!(a.0 >= FIRST_RANGE && b.0 >= FIRST_RANGE);
        assert!(a.1 <= b.0 || b.1 <= a.0);
        let last = range(ActorId::from_u64(4094).unwrap());
        assert!(last.0 < last.1);
    }

    /// Two sessions (random actors) share a range only by a chance of one in hundreds of
    /// thousands (it was one in 4,095), every range lies between the starter's ids and the repair's, and the last one
    /// is as large as the others.
    #[test]
    fn ranges_are_many_and_stay_below_the_repairs_ids() {
        assert!(SLOTS >= 200_000, "{SLOTS} ranges");
        let top = range(ActorId::from_u64(u64::from(SLOTS) - 1).unwrap());
        assert_eq!(top, (REKEY_FLOOR - RANGE, REKEY_FLOOR));
        let bottom = range(ActorId::from_u64(u64::from(SLOTS)).unwrap());
        assert_eq!(bottom, (FIRST_RANGE, FIRST_RANGE + RANGE));
        // Actor ids spread as random ones are.
        let slots: std::collections::BTreeSet<u32> =
            (1..=64u64).map(|index| slot(ActorId::from_u64(index.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1).unwrap())).collect();
        assert_eq!(slots.len(), 64, "64 sessions take 64 ranges");
    }
}
