//! A uniform grid over vertex positions, to find the vertices near a point without looking at
//! all of them. Built once per stroke (and again when vertices have moved far): vertices are
//! sorted by cell, and each cell keeps the range of the sorted list it holds.

use std::collections::HashMap;

use glam::Vec3;

/// Vertices bucketed by cell.
#[derive(Debug, Clone, Default)]
pub struct Grid {
    cell: f32,
    order: Vec<u32>,
    cells: HashMap<(i32, i32, i32), (u32, u32)>,
}

fn key(point: Vec3, cell: f32) -> (i32, i32, i32) {
    let index = |value: f32| (value / cell).floor().clamp(i32::MIN as f32, i32::MAX as f32) as i32;
    (index(point.x), index(point.y), index(point.z))
}

impl Grid {
    /// A grid with cells `cell` wide (clamped to a sane size).
    pub fn build(positions: &[Vec3], cell: f32) -> Self {
        let cell = if cell.is_finite() && cell > 1e-6 { cell } else { 1.0 };
        let mut keyed: Vec<((i32, i32, i32), u32)> =
            positions.iter().enumerate().filter(|(_, point)| point.is_finite()).map(|(index, point)| (key(*point, cell), index as u32)).collect();
        keyed.sort_unstable();
        let mut cells = HashMap::new();
        let mut start = 0usize;
        while start < keyed.len() {
            let Some((cell_key, _)) = keyed.get(start).copied() else { break };
            let mut end = start + 1;
            while keyed.get(end).is_some_and(|(other, _)| *other == cell_key) {
                end += 1;
            }
            cells.insert(cell_key, (start as u32, end as u32));
            start = end;
        }
        Self { cell, order: keyed.into_iter().map(|(_, index)| index).collect(), cells }
    }

    /// The cell width.
    pub fn cell(&self) -> f32 {
        self.cell
    }

    /// Calls `visit` with every vertex in the cells within `radius` of `center` (a superset of
    /// those within `radius`).
    pub fn query(&self, center: Vec3, radius: f32, mut visit: impl FnMut(u32)) {
        if !(center.is_finite() && radius.is_finite()) {
            return;
        }
        let low = key(center - Vec3::splat(radius), self.cell);
        let high = key(center + Vec3::splat(radius), self.cell);
        // A huge radius against a fine grid: walk the cells that exist instead.
        let span = |a: i32, b: i32| i64::from(b) - i64::from(a) + 1;
        if span(low.0, high.0) * span(low.1, high.1) * span(low.2, high.2) > self.cells.len() as i64 {
            for (cell_key, (start, end)) in &self.cells {
                if (low.0..=high.0).contains(&cell_key.0) && (low.1..=high.1).contains(&cell_key.1) && (low.2..=high.2).contains(&cell_key.2) {
                    for index in self.order.get(*start as usize..*end as usize).unwrap_or(&[]) {
                        visit(*index);
                    }
                }
            }
            return;
        }
        for x in low.0..=high.0 {
            for y in low.1..=high.1 {
                for z in low.2..=high.2 {
                    if let Some((start, end)) = self.cells.get(&(x, y, z)) {
                        for index in self.order.get(*start as usize..*end as usize).unwrap_or(&[]) {
                            visit(*index);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_same_vertices_as_looking_at_all() {
        let positions: Vec<Vec3> =
            (0..2000).map(|index| Vec3::new((index % 13) as f32 * 0.1, (index % 7) as f32 * 0.17, (index % 11) as f32 * 0.09)).collect();
        let grid = Grid::build(&positions, 0.2);
        for (center, radius) in [(Vec3::splat(0.5), 0.2), (Vec3::ZERO, 0.35), (Vec3::splat(10.0), 30.0)] {
            let mut found = Vec::new();
            grid.query(center, radius, |index| found.push(index));
            let near: Vec<u32> =
                positions.iter().enumerate().filter(|(_, point)| point.distance(center) <= radius).map(|(index, _)| index as u32).collect();
            assert!(near.iter().all(|index| found.contains(index)), "{center} {radius}");
        }
    }
}
