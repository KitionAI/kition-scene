//! Other people's selections in a co-edited scene: an outline around each object a peer has
//! selected, in the peer's colour, outside the silhouette, over
//! the background (never over a locally selected object).

use std::collections::BTreeSet;

use kition_scene_model::Id;

use crate::draw::{Canvas, NONE};

/// One peer's selected objects, and the colour to outline them in (linear RGB).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PeerOutline {
    pub color: [f32; 3],
    pub objects: BTreeSet<Id>,
}

/// Draws each peer's outline, two pixels wide, around the objects it selected; pixels of the
/// objects themselves and of locally selected objects are left alone.
pub fn outline_peers(canvas: &mut Canvas, peers: &[PeerOutline], local: &BTreeSet<Id>) {
    let (width, height) = (canvas.width as usize, canvas.height as usize);
    if width == 0 || height == 0 {
        return;
    }
    for peer in peers.iter().filter(|peer| !peer.objects.is_empty()) {
        let at = |x: usize, y: usize| canvas.object.get(y * width + x).copied().unwrap_or(NONE);
        let (mut left, mut top, mut right, mut bottom) = (usize::MAX, usize::MAX, 0usize, 0usize);
        for (slot, object) in canvas.object.iter().enumerate() {
            if *object != NONE && peer.objects.contains(object) {
                let (x, y) = (slot % width, slot / width);
                left = left.min(x);
                right = right.max(x);
                top = top.min(y);
                bottom = bottom.max(y);
            }
        }
        if left > right {
            continue;
        }
        let mut marks = Vec::new();
        for y in top.saturating_sub(2)..(bottom + 3).min(height) {
            for x in left.saturating_sub(2)..(right + 3).min(width) {
                let here = at(x, y);
                if peer.objects.contains(&here) || local.contains(&here) {
                    continue;
                }
                let near = (-2i64..=2).any(|dy| {
                    (-2i64..=2).any(|dx| {
                        let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                        dx * dx + dy * dy <= 5
                            && nx >= 0
                            && ny >= 0
                            && (nx as usize) < width
                            && (ny as usize) < height
                            && peer.objects.contains(&at(nx as usize, ny as usize))
                    })
                });
                if near {
                    marks.push((x, y));
                }
            }
        }
        for (x, y) in marks {
            canvas.blend(x as i64, y as i64, peer.color, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_peer_outline_rings_its_object_and_spares_the_local_selection() {
        let mut canvas = Canvas::new(10, 10, [1.0, 1.0, 1.0, 1.0]);
        for y in 3..7 {
            for x in 3..7 {
                canvas.object[y * 10 + x] = 4;
            }
        }
        let peers = [PeerOutline { color: [1.0, 0.0, 0.0], objects: BTreeSet::from([4]) }];
        outline_peers(&mut canvas, &peers, &BTreeSet::new());
        assert_eq!(canvas.pixel(2, 4).unwrap()[1], 0.0, "ringed in red");
        assert_eq!(canvas.pixel(4, 4).unwrap()[1], 1.0, "the object itself is untouched");
        assert_eq!(canvas.pixel(0, 0).unwrap()[1], 1.0, "far away is untouched");
        let mut spared = Canvas::new(10, 10, [1.0, 1.0, 1.0, 1.0]);
        for y in 3..7 {
            for x in 3..7 {
                spared.object[y * 10 + x] = 4;
            }
        }
        spared.object[2 * 10 + 4] = 9;
        outline_peers(&mut spared, &peers, &BTreeSet::from([9]));
        assert_eq!(spared.pixel(4, 2).unwrap()[1], 1.0, "a locally selected object's pixel is left alone");
    }
}
