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

/// The range slot of `actor`: 1 to 4095.
pub fn slot(actor: ActorId) -> u32 {
    u32::try_from(actor.get() % 4095).unwrap_or(0) + 1
}

/// The ids `actor` mints: `[start, end)`.
pub fn range(actor: ActorId) -> (Id, Id) {
    let start = u64::from(slot(actor)) << 20;
    let end = (start + (1 << 20)).min(u64::from(u32::MAX));
    (u32::try_from(start).unwrap_or(u32::MAX), u32::try_from(end).unwrap_or(u32::MAX))
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
}
