//! Mesh nodes: the mesh's own fields (islands, edge angles, face areas, faces per edge) and the
//! nodes that change a mesh's faces (Flip Faces, Split Edges, Triangulate, Dual Mesh, Scale
//! Elements, Mesh Boolean). Point attributes follow the vertices they were on; face attributes
//! follow the faces they came from.

use std::collections::{BTreeMap, HashMap};

use glam::Vec3;
use kition_scene_model::Mesh;
use kition_scene_model::csg::{self, Operation};

use crate::field::{Column, MeshInput, Scalar};
use crate::geometry::{Attribute, Attributes, Domain, MAX_ELEMENTS, MeshComponent};

fn index(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn key(a: u32, b: u32) -> (u32, u32) {
    (a.min(b), a.max(b))
}

/// The faces using each edge (by its vertex pair), in face order.
fn edge_faces(mesh: &Mesh) -> HashMap<(u32, u32), Vec<usize>> {
    let mut out: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (number, face) in mesh.faces.iter().enumerate() {
        for (a, b) in face.iter().zip(face.iter().cycle().skip(1)) {
            out.entry(key(*a, *b)).or_default().push(number);
        }
    }
    out
}

/// Which connected part each vertex is in (numbered from the part with the lowest vertex), and
/// how many parts there are.
pub fn islands(mesh: &Mesh) -> (Vec<i32>, i32) {
    let count = mesh.positions.len();
    let mut parent: Vec<usize> = (0..count).collect();
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
    for [a, b] in mesh.edges() {
        let (ra, rb) = (root(&mut parent, a as usize), root(&mut parent, b as usize));
        if ra != rb {
            let (low, high) = (ra.min(rb), ra.max(rb));
            if let Some(slot) = parent.get_mut(high) {
                *slot = low;
            }
        }
    }
    let mut numbers: HashMap<usize, i32> = HashMap::new();
    let mut out = Vec::with_capacity(count);
    for vertex in 0..count {
        let top = root(&mut parent, vertex);
        let next = i32::try_from(numbers.len()).unwrap_or(i32::MAX);
        out.push(*numbers.entry(top).or_insert(next));
    }
    (out, i32::try_from(numbers.len()).unwrap_or(i32::MAX))
}

/// Per edge (in [`Mesh::edges`] order): the angle between its two faces' normals, signed
/// negative where the edge is concave; zero for edges without exactly two faces.
pub fn edge_angles(mesh: &Mesh) -> (Vec<f32>, Vec<f32>) {
    let faces = edge_faces(mesh);
    let normals: Vec<Vec3> = mesh.faces.iter().map(|face| mesh.face_normal(face)).collect();
    let centres: Vec<Vec3> = mesh.faces.iter().map(|face| mesh.face_center(face)).collect();
    let mut unsigned = Vec::new();
    let mut signed = Vec::new();
    for [a, b] in mesh.edges() {
        let pair = faces.get(&key(a, b)).filter(|faces| faces.len() == 2);
        let Some([first, second]) = pair.map(|faces| [faces.first().copied().unwrap_or(0), faces.get(1).copied().unwrap_or(0)]) else {
            unsigned.push(0.0);
            signed.push(0.0);
            continue;
        };
        let (n1, n2) = (normals.get(first).copied().unwrap_or(Vec3::Z), normals.get(second).copied().unwrap_or(Vec3::Z));
        let angle = n1.dot(n2).clamp(-1.0, 1.0).acos();
        let towards = centres.get(second).copied().unwrap_or(Vec3::ZERO) - centres.get(first).copied().unwrap_or(Vec3::ZERO);
        let concave = n1.dot(towards) > 1e-6 || (angle > std::f32::consts::PI - 1e-4 && n1.dot(towards).abs() <= 1e-6);
        unsigned.push(angle);
        signed.push(if concave { -angle } else { angle });
    }
    (unsigned, signed)
}

/// A mesh input's values on `domain`.
pub fn mesh_input(mesh: &MeshComponent, input: MeshInput, domain: Domain) -> Column {
    let m = &mesh.mesh;
    let (values, from): (Column, Domain) = match input {
        MeshInput::IslandIndex | MeshInput::IslandCount => {
            let (numbers, count) = islands(m);
            if input == MeshInput::IslandCount {
                return vec![Scalar::Int(count); mesh.domain_len(domain)];
            }
            (numbers.into_iter().map(Scalar::Int).collect(), Domain::Point)
        }
        MeshInput::UnsignedAngle | MeshInput::SignedAngle => {
            let (unsigned, signed) = edge_angles(m);
            let pick = if input == MeshInput::SignedAngle { signed } else { unsigned };
            (pick.into_iter().map(Scalar::Float).collect(), Domain::Edge)
        }
        MeshInput::FaceArea => (m.faces.iter().map(|face| Scalar::Float(face_area(m, face))).collect(), Domain::Face),
        MeshInput::EdgeFaces => {
            let faces = edge_faces(m);
            let counts = m
                .edges()
                .into_iter()
                .map(|[a, b]| Scalar::Int(faces.get(&key(a, b)).map_or(0, |faces| i32::try_from(faces.len()).unwrap_or(i32::MAX))));
            (counts.collect(), Domain::Edge)
        }
    };
    mesh.adapt(&values, from, domain)
}

/// A face's area.
pub fn face_area(mesh: &Mesh, face: &[u32]) -> f32 {
    let points = mesh.face_positions(face);
    let mut sum = Vec3::ZERO;
    for (a, b) in points.iter().zip(points.iter().cycle().skip(1)) {
        sum += a.cross(*b);
    }
    sum.length() / 2.0
}

/// Corner attributes of faces made from the corners of old faces (`corners[face]` lists the old
/// corner each new corner comes from), and face attributes from the old faces.
fn carry_faces(attributes: &Attributes, old: &Mesh, faces_from: &[usize], corners_from: &[Vec<usize>]) -> Attributes {
    let mut starts = Vec::with_capacity(old.faces.len());
    let mut at = 0;
    for face in &old.faces {
        starts.push(at);
        at += face.len();
    }
    let mut out = Attributes::new();
    for (name, attribute) in attributes {
        let zero = attribute.values.first().map_or(Scalar::Float(0.0), |value| Scalar::zero(value.kind()));
        let values: Column = match attribute.domain {
            Domain::Point => attribute.values.clone(),
            Domain::Face => faces_from.iter().map(|face| attribute.values.get(*face).copied().unwrap_or(zero)).collect(),
            Domain::Corner => faces_from
                .iter()
                .zip(corners_from)
                .flat_map(|(face, corners)| {
                    let start = starts.get(*face).copied().unwrap_or(0);
                    corners.iter().map(move |corner| attribute.values.get(start + corner).copied().unwrap_or(zero)).collect::<Vec<_>>()
                })
                .collect(),
            _ => continue,
        };
        out.insert(name.clone(), Attribute { domain: attribute.domain, values });
    }
    out
}

/// Face materials and texture coordinates of faces made from old ones.
fn carry_face_data(component: &mut MeshComponent, old: &Mesh, faces_from: &[usize], corners_from: &[Vec<usize>]) {
    if !component.materials.is_empty() {
        let materials = std::mem::take(&mut component.materials);
        component.materials = faces_from.iter().map(|face| materials.get(*face).copied().flatten()).collect();
    }
    if old.uvs.len() == old.faces.len() && !old.uvs.is_empty() {
        component.mesh.uvs = faces_from
            .iter()
            .zip(corners_from)
            .map(|(face, corners)| {
                corners.iter().map(|corner| old.uvs.get(*face).and_then(|uvs| uvs.get(*corner)).copied().unwrap_or_default()).collect()
            })
            .collect();
    } else {
        component.mesh.uvs.clear();
    }
}

/// Flip Faces: each selected face's corners in the other order, from the same first corner.
pub fn flip_faces(component: &mut MeshComponent, selected: &[bool]) {
    let old = component.mesh.clone();
    let mut corners_from = Vec::with_capacity(old.faces.len());
    for (number, face) in component.mesh.faces.iter_mut().enumerate() {
        let order: Vec<usize> = if selected.get(number).copied().unwrap_or(false) {
            std::iter::once(0).chain((1..face.len()).rev()).collect()
        } else {
            (0..face.len()).collect()
        };
        *face = order.iter().filter_map(|corner| old.faces.get(number).and_then(|face| face.get(*corner)).copied()).collect();
        corners_from.push(order);
    }
    let faces_from: Vec<usize> = (0..old.faces.len()).collect();
    component.attributes = carry_faces(&component.attributes, &old, &faces_from, &corners_from);
    carry_face_data(component, &old, &faces_from, &corners_from);
}

/// Split Edges: the selected edges' faces stop sharing vertices along them. Around each vertex,
/// the faces that still meet across unsplit edges keep one vertex: the last such group keeps the
/// old vertex, the others get new ones (after the old vertices, vertex by vertex).
pub fn split_edges(component: &mut MeshComponent, selected: &[bool]) -> Result<(), String> {
    let mesh = &component.mesh;
    let edges = mesh.edges();
    let split: std::collections::HashSet<(u32, u32)> =
        edges.iter().zip(selected).filter(|(_, selected)| **selected).map(|([a, b], _)| key(*a, *b)).collect();
    if split.is_empty() {
        return Ok(());
    }
    let shared = edge_faces(mesh);
    // Each corner: (face, corner in face); the corners of each vertex in corner order.
    let mut corners_of: Vec<Vec<(usize, usize)>> = vec![Vec::new(); mesh.positions.len()];
    for (number, face) in mesh.faces.iter().enumerate() {
        for (slot, vertex) in face.iter().enumerate() {
            if let Some(list) = corners_of.get_mut(*vertex as usize) {
                list.push((number, slot));
            }
        }
    }
    let mut new_vertex: HashMap<(usize, usize), u32> = HashMap::new();
    let mut positions = mesh.positions.clone();
    let mut sources: Vec<usize> = (0..mesh.positions.len()).collect();
    for (vertex, corners) in corners_of.iter().enumerate() {
        if corners.len() < 2 {
            continue;
        }
        // Group the corners: two corners of faces meeting across an unsplit edge at this vertex
        // are in one group.
        let mut group: Vec<usize> = (0..corners.len()).collect();
        fn find(group: &mut [usize], mut at: usize) -> usize {
            while let Some(&up) = group.get(at) {
                if up == at {
                    break;
                }
                at = up;
            }
            at
        }
        let vertex_id = index(vertex);
        for (first, (face, _)) in corners.iter().enumerate() {
            let Some(face_vertices) = mesh.faces.get(*face) else { continue };
            for other in face_vertices.iter().filter(|other| **other != vertex_id) {
                let edge = key(vertex_id, *other);
                if !face_vertices.iter().zip(face_vertices.iter().cycle().skip(1)).any(|(a, b)| key(*a, *b) == edge) || split.contains(&edge) {
                    continue;
                }
                for neighbour in shared.get(&edge).into_iter().flatten().filter(|neighbour| *neighbour != face) {
                    if let Some(second) = corners.iter().position(|(face, _)| face == neighbour) {
                        let (a, b) = (find(&mut group, first), find(&mut group, second));
                        if a != b
                            && let Some(slot) = group.get_mut(a.max(b))
                        {
                            *slot = a.min(b);
                        }
                    }
                }
            }
        }
        let mut order: Vec<usize> = Vec::new();
        let roots: Vec<usize> = (0..corners.len()).map(|at| find(&mut group, at)).collect();
        for root in &roots {
            if !order.contains(root) {
                order.push(*root);
            }
        }
        // The last group keeps the vertex; the others get new ones.
        for root in order.iter().take(order.len().saturating_sub(1)) {
            let made = index(positions.len());
            positions.push(mesh.positions.get(vertex).copied().unwrap_or(Vec3::ZERO));
            sources.push(vertex);
            for (at, corner) in corners.iter().enumerate() {
                if roots.get(at) == Some(root) {
                    new_vertex.insert(*corner, made);
                }
            }
        }
        if positions.len() > MAX_ELEMENTS {
            return Err(format!("splitting would make more than {MAX_ELEMENTS} vertices"));
        }
    }
    let faces: Vec<Vec<u32>> = mesh
        .faces
        .iter()
        .enumerate()
        .map(|(number, face)| face.iter().enumerate().map(|(slot, vertex)| new_vertex.get(&(number, slot)).copied().unwrap_or(*vertex)).collect())
        .collect();
    let mut attributes = Attributes::new();
    for (name, attribute) in &component.attributes {
        let values = match attribute.domain {
            Domain::Point => sources.iter().map(|source| attribute.values.get(*source).copied().unwrap_or(Scalar::Float(0.0))).collect(),
            Domain::Face | Domain::Corner => attribute.values.clone(),
            _ => continue,
        };
        attributes.insert(name.clone(), Attribute { domain: attribute.domain, values });
    }
    component.attributes = attributes;
    let mesh = &mut component.mesh;
    mesh.positions = positions;
    mesh.faces = faces;
    mesh.seams.clear();
    mesh.groups.clear();
    mesh.pinned.clear();
    mesh.shape_keys.clear();
    mesh.mask.clear();
    Ok(())
}

/// How Triangulate splits quads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuadMethod {
    Beauty,
    Fixed,
    FixedAlternate,
    ShortestDiagonal,
    LongestDiagonal,
}

impl QuadMethod {
    pub fn parse(text: &str) -> Self {
        match text {
            "beauty" => Self::Beauty,
            "fixed" => Self::Fixed,
            "fixed_alternate" => Self::FixedAlternate,
            "longest_diagonal" => Self::LongestDiagonal,
            _ => Self::ShortestDiagonal,
        }
    }
}

/// The smallest corner angle of a triangle.
fn smallest_angle(a: Vec3, b: Vec3, c: Vec3) -> f32 {
    let angle = |at: Vec3, p: Vec3, q: Vec3| (p - at).normalize_or_zero().dot((q - at).normalize_or_zero()).clamp(-1.0, 1.0).acos();
    angle(a, b, c).min(angle(b, c, a)).min(angle(c, a, b))
}

/// A quad's two triangles as corners: split from corner 0 to 2, or from 1 to 3.
fn quad_triangles(points: [Vec3; 4], method: QuadMethod) -> [[usize; 3]; 2] {
    let first = [[2, 0, 1], [0, 2, 3]];
    let second = [[1, 3, 0], [3, 1, 2]];
    let [a, b, c, d] = points;
    let use_first = match method {
        QuadMethod::Fixed => true,
        QuadMethod::FixedAlternate => false,
        QuadMethod::ShortestDiagonal => a.distance_squared(c) < b.distance_squared(d),
        QuadMethod::LongestDiagonal => a.distance_squared(c) > b.distance_squared(d),
        QuadMethod::Beauty => smallest_angle(a, b, c).min(smallest_angle(a, c, d)) > smallest_angle(a, b, d).min(smallest_angle(b, c, d)),
    };
    if use_first { first } else { second }
}

/// A polygon's triangles by clipping ears in turn around it (each next ear tried two corners on
/// from the last), as corners of the polygon.
pub fn clip_ears(points: &[Vec3]) -> Vec<[usize; 3]> {
    let normal = kition_scene_model::mesh::newell_vector(points).normalize_or_zero();
    let mut left: Vec<usize> = (0..points.len()).collect();
    let mut out = Vec::new();
    let at = |corner: usize| points.get(corner).copied().unwrap_or(Vec3::ZERO);
    let mut position = 0usize;
    let mut misses = 0usize;
    while left.len() > 3 {
        let count = left.len();
        let slot = position % count;
        let corner = |at: usize| left.get(at % count).copied().unwrap_or(0);
        let (previous, here, next) = (corner(slot + count - 1), corner(slot), corner(slot + 1));
        let (a, b, c) = (at(previous), at(here), at(next));
        let convex = (b - a).cross(c - b).dot(normal) > 0.0;
        let empty =
            left.iter().filter(|corner| ![previous, here, next].contains(corner)).all(|corner| !inside_triangle(at(*corner), a, b, c, normal));
        if (convex && empty) || misses >= count {
            out.push([previous, here, next]);
            left.remove(slot);
            position = slot + 1;
            misses = 0;
        } else {
            position = slot + 1;
            misses += 1;
        }
    }
    if let [a, b, c] = *left.as_slice() {
        out.push([a, b, c]);
    }
    out
}

fn inside_triangle(point: Vec3, a: Vec3, b: Vec3, c: Vec3, normal: Vec3) -> bool {
    let side = |p: Vec3, q: Vec3| (q - p).cross(point - p).dot(normal) >= 0.0;
    side(a, b) && side(b, c) && side(c, a)
}

/// Flips the shared edges of triangle pairs where the other diagonal makes better shaped
/// triangles (the ear clipping's triangles, made more even).
fn beautify(points: &[Vec3], triangles: &mut [[usize; 3]]) {
    let at = |corner: usize| points.get(corner).copied().unwrap_or(Vec3::ZERO);
    let quality = |[a, b, c]: [usize; 3]| smallest_angle(at(a), at(b), at(c));
    for _ in 0..triangles.len() * triangles.len() + 1 {
        let mut flipped = false;
        for first in 0..triangles.len() {
            for second in first + 1..triangles.len() {
                let (Some(t1), Some(t2)) = (triangles.get(first).copied(), triangles.get(second).copied()) else { continue };
                // The edge t1 runs c -> a, t2 runs a -> c.
                let corner = |triangle: [usize; 3], at: usize| triangle.get(at % 3).copied().unwrap_or(0);
                let Some(turn) = (0..3).find(|turn| {
                    let (a, c) = (corner(t1, *turn), corner(t1, *turn + 2));
                    (0..3).any(|other| corner(t2, other) == a && corner(t2, other + 1) == c)
                }) else {
                    continue;
                };
                let (a, b, c) = (corner(t1, turn), corner(t1, turn + 1), corner(t1, turn + 2));
                let Some(d) = t2.iter().copied().find(|corner| *corner != a && *corner != c) else { continue };
                let (new1, new2) = ([a, b, d], [b, c, d]);
                let convex = {
                    let normal = (at(b) - at(a)).cross(at(c) - at(a));
                    (at(b) - at(a)).cross(at(d) - at(a)).dot(normal) > 0.0 && (at(c) - at(b)).cross(at(d) - at(b)).dot(normal) > 0.0
                };
                if convex && quality(new1).min(quality(new2)) > quality(t1).min(quality(t2)) + 1e-5 {
                    if let Some(slot) = triangles.get_mut(first) {
                        *slot = new1;
                    }
                    if let Some(slot) = triangles.get_mut(second) {
                        *slot = new2;
                    }
                    flipped = true;
                }
            }
        }
        if !flipped {
            break;
        }
    }
}

/// Triangulate: each selected face of four or more corners becomes triangles; they come first,
/// in face order, then the faces left as they were.
pub fn triangulate(component: &mut MeshComponent, selected: &[bool], quads: QuadMethod, beauty: bool) -> Result<(), String> {
    let old = component.mesh.clone();
    let mut faces = Vec::new();
    let mut faces_from = Vec::new();
    let mut corners_from = Vec::new();
    let split = |number: usize, face: &Vec<u32>| face.len() >= 4 && selected.get(number).copied().unwrap_or(false);
    for (number, face) in old.faces.iter().enumerate().filter(|(number, face)| split(*number, face)) {
        let points = old.face_positions(face);
        let triangles: Vec<[usize; 3]> = if let [a, b, c, d] = *points.as_slice() {
            quad_triangles([a, b, c, d], quads).to_vec()
        } else {
            let mut triangles = clip_ears(&points);
            if beauty {
                beautify(&points, &mut triangles);
            }
            triangles
        };
        for triangle in triangles {
            faces.push(triangle.iter().filter_map(|corner| face.get(*corner).copied()).collect());
            faces_from.push(number);
            corners_from.push(triangle.to_vec());
        }
        if faces.len() > MAX_ELEMENTS {
            return Err(format!("triangulating would make more than {MAX_ELEMENTS} faces"));
        }
    }
    for (number, face) in old.faces.iter().enumerate().filter(|(number, face)| !split(*number, face)) {
        faces.push(face.clone());
        faces_from.push(number);
        corners_from.push((0..face.len()).collect());
    }
    component.mesh.faces = faces;
    component.attributes = carry_faces(&component.attributes, &old, &faces_from, &corners_from);
    carry_face_data(component, &old, &faces_from, &corners_from);
    component.mesh.face_sets.clear();
    component.mesh.hidden.clear();
    Ok(())
}

/// Dual Mesh: a vertex at each face's centre and a face around each vertex the faces surround.
/// With `keep_boundaries`, vertices on open edges get faces too, through the open edges'
/// midpoints and the vertex itself; without, they get none.
pub fn dual_mesh(component: &MeshComponent, keep_boundaries: bool) -> Result<Mesh, String> {
    let mesh = &component.mesh;
    let shared = edge_faces(mesh);
    let mut positions: Vec<Vec3> = mesh.faces.iter().map(|face| mesh.face_center(face)).collect();
    // The face whose edge `a -> b` is: by directed edge.
    let mut directed: HashMap<(u32, u32), usize> = HashMap::new();
    for (number, face) in mesh.faces.iter().enumerate() {
        for (a, b) in face.iter().zip(face.iter().cycle().skip(1)) {
            directed.insert((*a, *b), number);
        }
    }
    let open = |a: u32, b: u32| shared.get(&key(a, b)).is_none_or(|faces| faces.len() < 2);
    let mut midpoints: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    let mut corners: BTreeMap<u32, u32> = BTreeMap::new();
    let mut rings: Vec<Vec<Ring>> = Vec::new();
    #[derive(Clone, Copy)]
    enum Ring {
        Face(usize),
        Middle(u32, u32),
        Vertex(u32),
    }
    let around = |face: usize, vertex: u32| -> Option<(u32, u32)> {
        let face = mesh.faces.get(face)?;
        let slot = face.iter().position(|other| *other == vertex)?;
        let count = face.len();
        Some((*face.get((slot + count - 1) % count)?, *face.get((slot + 1) % count)?))
    };
    let mut first_face: Vec<Option<usize>> = vec![None; mesh.positions.len()];
    let mut faces_of: Vec<usize> = vec![0; mesh.positions.len()];
    for (number, face) in mesh.faces.iter().enumerate() {
        for vertex in face {
            if let Some(slot) = first_face.get_mut(*vertex as usize) {
                slot.get_or_insert(number);
            }
            if let Some(count) = faces_of.get_mut(*vertex as usize) {
                *count += 1;
            }
        }
    }
    for vertex in 0..mesh.positions.len() {
        let v = index(vertex);
        let Some(start) = first_face.get(vertex).copied().flatten() else {
            rings.push(Vec::new());
            continue;
        };
        // On a boundary, start at the face whose next edge from the vertex is open.
        let mut begin = start;
        let mut boundary = false;
        let mut at = start;
        for _ in 0..=faces_of.get(vertex).copied().unwrap_or(0) {
            let Some((_, next)) = around(at, v) else { break };
            if open(v, next) {
                begin = at;
                boundary = true;
                break;
            }
            // Step backwards around the vertex: the face across the edge to `next`.
            match directed.get(&(next, v)) {
                Some(face) if *face != start => at = *face,
                _ => break,
            }
        }
        if boundary && !keep_boundaries {
            rings.push(Vec::new());
            continue;
        }
        let mut ring = Vec::new();
        let mut at = begin;
        let first_next = around(begin, v).map(|(_, next)| next);
        let mut last_previous = None;
        for _ in 0..=faces_of.get(vertex).copied().unwrap_or(0) {
            ring.push(Ring::Face(at));
            let Some((previous, _)) = around(at, v) else { break };
            last_previous = Some(previous);
            if open(previous, v) {
                break;
            }
            match directed.get(&(v, previous)) {
                Some(face) if *face != begin => at = *face,
                _ => break,
            }
        }
        if boundary && let (Some(previous), Some(next)) = (last_previous, first_next) {
            ring.push(Ring::Middle(v, previous));
            ring.push(Ring::Vertex(v));
            ring.push(Ring::Middle(v, next));
        }
        if ring.len() < 3 {
            ring.clear();
        }
        rings.push(ring);
    }
    // Open edges' midpoints in edge order, then the boundary vertices in vertex order.
    if keep_boundaries {
        for [a, b] in mesh.edges() {
            if open(a, b) && rings.iter().any(|ring| ring.iter().any(|item| matches!(item, Ring::Middle(x, y) if key(*x, *y) == key(a, b)))) {
                midpoints.insert(key(a, b), index(positions.len()));
                let point = (mesh.positions.get(a as usize).copied().unwrap_or(Vec3::ZERO)
                    + mesh.positions.get(b as usize).copied().unwrap_or(Vec3::ZERO))
                    / 2.0;
                positions.push(point);
            }
        }
        for (vertex, ring) in rings.iter().enumerate() {
            if ring.iter().any(|item| matches!(item, Ring::Vertex(_))) {
                corners.insert(index(vertex), index(positions.len()));
                positions.push(mesh.positions.get(vertex).copied().unwrap_or(Vec3::ZERO));
            }
        }
    }
    if positions.len() > MAX_ELEMENTS {
        return Err(format!("the dual mesh would have more than {MAX_ELEMENTS} vertices"));
    }
    let faces: Vec<Vec<u32>> = rings
        .iter()
        .filter(|ring| !ring.is_empty())
        .map(|ring| {
            ring.iter()
                .filter_map(|item| match item {
                    Ring::Face(face) => Some(index(*face)),
                    Ring::Middle(a, b) => midpoints.get(&key(*a, *b)).copied(),
                    Ring::Vertex(v) => corners.get(v).copied(),
                })
                .collect()
        })
        .collect();
    Ok(Mesh { positions, faces, ..Mesh::default() })
}

/// Scale Elements: the selected faces (or edges), in groups of those that share vertices,
/// scaled about each group's centre (the mean of its elements' centres), uniformly or along an
/// axis.
pub fn scale_elements(mesh: &mut Mesh, elements: &[Vec<u32>], scales: &[f32], centres: &[Vec3], axes: Option<&[Vec3]>) {
    let count = mesh.positions.len();
    let mut parent: Vec<usize> = (0..elements.len()).collect();
    fn find(parent: &mut [usize], mut at: usize) -> usize {
        while let Some(&up) = parent.get(at) {
            if up == at {
                break;
            }
            at = up;
        }
        at
    }
    let mut owner: Vec<Option<usize>> = vec![None; count];
    for (number, element) in elements.iter().enumerate() {
        for vertex in element {
            match owner.get(*vertex as usize).copied().flatten() {
                Some(other) => {
                    let (a, b) = (find(&mut parent, other), find(&mut parent, number));
                    if a != b
                        && let Some(slot) = parent.get_mut(a.max(b))
                    {
                        *slot = a.min(b);
                    }
                }
                None => {
                    if let Some(slot) = owner.get_mut(*vertex as usize) {
                        *slot = Some(number);
                    }
                }
            }
        }
    }
    let mut groups: BTreeMap<usize, (Vec3, f32, Vec3, usize)> = BTreeMap::new();
    for number in 0..elements.len() {
        let root = find(&mut parent, number);
        let entry = groups.entry(root).or_insert((Vec3::ZERO, 0.0, Vec3::ZERO, 0));
        entry.0 += centres.get(number).copied().unwrap_or(Vec3::ZERO);
        entry.1 += scales.get(number).copied().unwrap_or(1.0);
        entry.2 += axes.and_then(|axes| axes.get(number)).copied().unwrap_or(Vec3::X);
        entry.3 += 1;
    }
    let original = mesh.positions.clone();
    for (vertex, owner) in owner.iter().enumerate() {
        let Some(owner) = owner else { continue };
        let root = find(&mut parent, *owner);
        let Some((centre, scale, axis, count)) = groups.get(&root).copied() else { continue };
        let n = count.max(1) as f32;
        let (centre, scale) = (centre / n, scale / n);
        let Some(point) = original.get(vertex).copied() else { continue };
        let moved = if axes.is_some() {
            let axis = (axis / n).normalize_or_zero();
            point + axis * ((point - centre).dot(axis) * (scale - 1.0))
        } else {
            centre + (point - centre) * scale
        };
        if let Some(slot) = mesh.positions.get_mut(vertex)
            && moved.is_finite()
        {
            *slot = moved;
        }
    }
}

/// Mesh Boolean: the first mesh without the others (difference), or all of them as one (union)
/// or only where all are (intersect).
pub fn boolean(first: Option<&Mesh>, others: &[Mesh], operation: Operation) -> Result<Mesh, String> {
    let fail = || "the boolean failed (the meshes may not be closed, or are too large)".to_owned();
    match operation {
        Operation::Difference => {
            let Some(mut result) = first.cloned() else { return Ok(Mesh::default()) };
            for other in others.iter().filter(|other| !other.faces.is_empty()) {
                result = csg::boolean(&result, other, Operation::Difference).ok_or_else(fail)?;
            }
            Ok(result)
        }
        _ => {
            let mut parts = others.iter().filter(|other| !other.faces.is_empty());
            let Some(mut result) = parts.next().cloned() else { return Ok(Mesh::default()) };
            for other in parts {
                result = csg::boolean(&result, other, operation).ok_or_else(fail)?;
            }
            Ok(result)
        }
    }
}
