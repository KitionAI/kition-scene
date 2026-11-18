//! Face sets and hiding in sculpt mode, and the box and lasso gestures that mask, hide, or make
//! a face set of what they enclose, as the reference application's sculpt mode has them.
//!
//! A face set is an id per face (0 for none, [`Mesh::face_sets`]); hidden faces
//! ([`Mesh::hidden`]) are not drawn in sculpt mode and brushes leave their vertices alone. Both
//! lists are empty when unused, and are ignored (then dropped) when they no longer have one entry
//! per face.

use std::collections::BTreeSet;

use glam::Vec3;

use crate::error::{Result, SceneError};
use crate::mesh::Mesh;

/// Drops face sets or hidden flags that no longer have one entry per face, and empty ones.
pub fn tidy(mesh: &mut Mesh) {
    let faces = mesh.faces.len();
    if mesh.face_sets.len() != faces || mesh.face_sets.iter().all(|set| *set == 0) {
        mesh.face_sets.clear();
    }
    if mesh.hidden.len() != faces || mesh.hidden.iter().all(|hidden| !hidden) {
        mesh.hidden.clear();
    }
}

/// The face set of each face (0 for none), one per face.
pub fn face_sets(mesh: &Mesh) -> Vec<u32> {
    if mesh.face_sets.len() == mesh.faces.len() { mesh.face_sets.clone() } else { vec![0; mesh.faces.len()] }
}

/// Whether each face is hidden, one per face.
pub fn hidden(mesh: &Mesh) -> Vec<bool> {
    if mesh.hidden.len() == mesh.faces.len() { mesh.hidden.clone() } else { vec![false; mesh.faces.len()] }
}

/// Whether each vertex is hidden: it has faces and every one of them is hidden.
pub fn hidden_vertices(mesh: &Mesh) -> Vec<bool> {
    let count = mesh.positions.len();
    if mesh.hidden.len() != mesh.faces.len() || mesh.hidden.is_empty() {
        return vec![false; count];
    }
    let mut shown = vec![false; count];
    let mut any = vec![false; count];
    for (face, hidden) in mesh.faces.iter().zip(&mesh.hidden) {
        for vertex in face {
            if let Some(slot) = any.get_mut(*vertex as usize) {
                *slot = true;
            }
            if !hidden && let Some(slot) = shown.get_mut(*vertex as usize) {
                *slot = true;
            }
        }
    }
    any.iter().zip(&shown).map(|(any, shown)| *any && !shown).collect()
}

/// The id a new face set takes: one more than the highest.
pub fn next_id(mesh: &Mesh) -> u32 {
    face_sets(mesh).into_iter().max().unwrap_or(0).saturating_add(1).max(1)
}

/// Puts the faces `chosen` picks into a new face set; answers its id and how many faces.
fn new_set(mesh: &mut Mesh, mut chosen: impl FnMut(usize, &[u32]) -> bool) -> Result<(u32, usize)> {
    let id = next_id(mesh);
    let mut sets = face_sets(mesh);
    let mut count = 0;
    for (index, (face, set)) in mesh.faces.iter().zip(sets.iter_mut()).enumerate() {
        if chosen(index, face) {
            *set = id;
            count += 1;
        }
    }
    if count == 0 {
        return Err(SceneError::params("no face would be in the face set"));
    }
    mesh.face_sets = sets;
    tidy(mesh);
    Ok((id, count))
}

/// Face Set from Masked: the faces whose every vertex is at least half masked.
pub fn from_mask(mesh: &mut Mesh) -> Result<(u32, usize)> {
    let mask = mesh.mask.clone();
    let hidden = hidden(mesh);
    new_set(mesh, |index, face| {
        !hidden.get(index).copied().unwrap_or(false) && face.iter().all(|vertex| mask.get(*vertex as usize).copied().unwrap_or(0.0) >= 0.5)
    })
}

/// Face Set from Visible: every face not hidden.
pub fn from_visible(mesh: &mut Mesh) -> Result<(u32, usize)> {
    let hidden = hidden(mesh);
    new_set(mesh, |index, _| !hidden.get(index).copied().unwrap_or(false))
}

/// Face sets by loose parts: each connected part of the mesh its own set, from 1.
pub fn from_loose_parts(mesh: &mut Mesh) -> usize {
    let mut parent: Vec<usize> = (0..mesh.positions.len()).collect();
    fn root(parent: &mut [usize], mut at: usize) -> usize {
        while let Some(&up) = parent.get(at) {
            if up == at {
                break;
            }
            let grand = parent.get(up).copied().unwrap_or(up);
            if let Some(slot) = parent.get_mut(at) {
                *slot = grand;
            }
            at = up;
        }
        at
    }
    for face in &mesh.faces {
        let Some(first) = face.first() else { continue };
        for vertex in face.iter().skip(1) {
            let (a, b) = (root(&mut parent, *first as usize), root(&mut parent, *vertex as usize));
            if a != b
                && let Some(slot) = parent.get_mut(a.max(b))
            {
                *slot = a.min(b);
            }
        }
    }
    let mut ids = std::collections::HashMap::new();
    let mut sets = Vec::with_capacity(mesh.faces.len());
    for face in &mesh.faces {
        let part = face.first().map_or(0, |vertex| root(&mut parent, *vertex as usize));
        let next = ids.len() as u32 + 1;
        sets.push(*ids.entry(part).or_insert(next));
    }
    mesh.face_sets = sets;
    let count = ids.len();
    tidy(mesh);
    count
}

/// Clears every face set.
pub fn clear(mesh: &mut Mesh) {
    mesh.face_sets.clear();
}

/// What a face set visibility command does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    /// Hides the faces of the set.
    Hide,
    /// Hides every face but the set's.
    Isolate,
    /// Shows every face.
    ShowAll,
    /// Shows every face when some are hidden, else isolates the set.
    Toggle,
}

/// Hides or shows faces by face set `id` (0 is the faces in no set); answers how many faces are
/// hidden after.
pub fn visibility(mesh: &mut Mesh, id: u32, action: Visibility) -> usize {
    let sets = face_sets(mesh);
    let mut hidden = hidden(mesh);
    let action = match action {
        Visibility::Toggle if hidden.iter().any(|flag| *flag) => Visibility::ShowAll,
        Visibility::Toggle => Visibility::Isolate,
        other => other,
    };
    for (flag, set) in hidden.iter_mut().zip(&sets) {
        match action {
            Visibility::Hide if *set == id => *flag = true,
            Visibility::Isolate => *flag = *set != id,
            Visibility::ShowAll => *flag = false,
            _ => {}
        }
    }
    mesh.hidden = hidden;
    tidy(mesh);
    mesh.hidden.iter().filter(|flag| **flag).count()
}

/// What a box or lasso gesture does to what it encloses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Gesture {
    /// Sets the mask of the enclosed vertices to a value (1 masks, 0 unmasks).
    Mask(f32),
    /// Hides the faces with an enclosed vertex.
    Hide,
    /// Shows the faces with an enclosed vertex.
    Show,
    /// Puts the faces whose centre is enclosed into a new face set.
    FaceSet,
}

/// Runs `gesture` on the vertices `inside` says are enclosed (given each vertex's object-space
/// position; `outside` flips it); answers how many vertices or faces it changed.
pub fn gesture(mesh: &mut Mesh, gesture: Gesture, outside: bool, inside: &dyn Fn(Vec3) -> bool) -> Result<usize> {
    tidy(mesh);
    let enclosed: Vec<bool> = mesh.positions.iter().map(|point| inside(*point) != outside).collect();
    let hidden_vertex = hidden_vertices(mesh);
    let count = match gesture {
        Gesture::Mask(value) => {
            if !(value.is_finite() && (0.0..=1.0).contains(&value)) {
                return Err(SceneError::params("a mask value goes from 0 to 1"));
            }
            if mesh.mask.len() != mesh.positions.len() {
                mesh.mask = vec![0.0; mesh.positions.len()];
            }
            let mut changed = 0;
            for ((mask, inside), hidden) in mesh.mask.iter_mut().zip(&enclosed).zip(&hidden_vertex) {
                if *inside && !hidden {
                    *mask = value;
                    changed += 1;
                }
            }
            if mesh.mask.iter().all(|value| *value == 0.0) {
                mesh.mask.clear();
            }
            changed
        }
        Gesture::Hide | Gesture::Show => {
            let hide = gesture == Gesture::Hide;
            let mut flags = hidden(mesh);
            let mut changed = 0;
            for (face, flag) in mesh.faces.iter().zip(flags.iter_mut()) {
                if face.iter().any(|vertex| enclosed.get(*vertex as usize).copied().unwrap_or(false)) && *flag != hide {
                    *flag = hide;
                    changed += 1;
                }
            }
            mesh.hidden = flags;
            tidy(mesh);
            changed
        }
        Gesture::FaceSet => {
            let hidden = hidden(mesh);
            let centres: Vec<bool> = mesh.faces.iter().map(|face| inside(mesh.face_center(face)) != outside).collect();
            new_set(mesh, |index, _| centres.get(index).copied().unwrap_or(false) && !hidden.get(index).copied().unwrap_or(false))?.1
        }
    };
    Ok(count)
}

/// Whether `point` lies inside the polygon `outline` (even-odd rule).
pub fn in_polygon(outline: &[(f32, f32)], point: (f32, f32)) -> bool {
    let mut inside = false;
    let count = outline.len();
    for index in 0..count {
        let (Some(&(xa, ya)), Some(&(xb, yb))) = (outline.get(index), outline.get((index + count - 1) % count)) else { continue };
        if (ya > point.1) != (yb > point.1) && point.0 < (xb - xa) * (point.1 - ya) / (yb - ya) + xa {
            inside = !inside;
        }
    }
    inside
}

/// A face set's colour in the viewport: a stable bright hue from its id (none for 0).
pub fn color(id: u32) -> Option<[f32; 3]> {
    if id == 0 {
        return None;
    }
    // The golden angle spreads neighbouring ids round the hue circle.
    let hue = (id as f32 * 0.618_034).fract();
    let (saturation, value) = (0.45, 0.95);
    let sector = hue * 6.0;
    let fraction = sector.fract();
    let (p, q, t) = (value * (1.0 - saturation), value * (1.0 - saturation * fraction), value * (1.0 - saturation * (1.0 - fraction)));
    Some(match sector as u32 {
        0 => [value, t, p],
        1 => [q, value, p],
        2 => [p, value, t],
        3 => [p, q, value],
        4 => [t, p, value],
        _ => [value, p, q],
    })
}

/// The face sets in use, ascending.
pub fn ids(mesh: &Mesh) -> BTreeSet<u32> {
    face_sets(mesh).into_iter().filter(|id| *id != 0).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::Primitive;

    #[test]
    fn sets_from_the_mask_hide_and_isolate() {
        let mut mesh = Primitive::Grid { x_subdivisions: 4, y_subdivisions: 4, size: 2.0 }.build().unwrap();
        mesh.mask = mesh.positions.iter().map(|point| if point.x > 0.1 { 1.0 } else { 0.0 }).collect();
        let (id, count) = from_mask(&mut mesh).unwrap();
        assert_eq!((id, count), (1, 4));
        assert_eq!(visibility(&mut mesh, 1, Visibility::Hide), 4);
        assert_eq!(hidden_vertices(&mesh).iter().filter(|flag| **flag).count(), 5);
        assert_eq!(visibility(&mut mesh, 1, Visibility::Isolate), 12);
        assert_eq!(visibility(&mut mesh, 1, Visibility::Toggle), 0);
        assert!(mesh.hidden.is_empty());
        let (visible, _) = from_visible(&mut mesh).unwrap();
        assert_eq!(visible, 2);
        assert_eq!(ids(&mesh).len(), 1);
    }

    #[test]
    fn gestures_mask_hide_and_make_sets() {
        let mut mesh = Primitive::Grid { x_subdivisions: 4, y_subdivisions: 4, size: 2.0 }.build().unwrap();
        let left = |point: Vec3| point.x < -0.6;
        assert_eq!(gesture(&mut mesh, Gesture::Mask(1.0), false, &left).unwrap(), 5);
        assert_eq!(gesture(&mut mesh, Gesture::Mask(1.0), true, &left).unwrap(), 20);
        assert!(mesh.mask.iter().all(|value| *value == 1.0));
        assert_eq!(gesture(&mut mesh, Gesture::Hide, false, &left).unwrap(), 4);
        // A hidden vertex keeps its mask.
        gesture(&mut mesh, Gesture::Mask(0.0), false, &|_| true).unwrap();
        assert_eq!(mesh.mask.iter().filter(|value| **value == 1.0).count(), 5);
        assert_eq!(gesture(&mut mesh, Gesture::Show, false, &|_| true).unwrap(), 4);
        assert_eq!(gesture(&mut mesh, Gesture::FaceSet, false, &|point: Vec3| point.y > 0.0).unwrap(), 8);
        assert_eq!(from_loose_parts(&mut mesh), 1);
    }

    #[test]
    fn a_lasso_encloses_by_the_even_odd_rule() {
        let square = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        assert!(in_polygon(&square, (5.0, 5.0)));
        assert!(!in_polygon(&square, (15.0, 5.0)));
        assert!(color(0).is_none());
        assert_ne!(color(1), color(2));
    }
}
