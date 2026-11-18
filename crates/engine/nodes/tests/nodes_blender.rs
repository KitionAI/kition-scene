//! Group nodes, zones, and the mesh, instance, sampling, attribute, and rotation nodes against
//! the reference application. `fixtures/nodes-reference.json` holds what Blender 5.2.3 made of
//! each case (`tests/blender/nodes.py` writes it from the cases below) and the case itself (its
//! groups and frames), which must still be this test's. Every case compares the mesh (vertex,
//! face, and edge counts exactly; vertices in order, or as a set where noted; faces as lists, up
//! to rotation, as loops of positions, or for triangulated n-gons by area; edges as vertex
//! pairs), named attributes on every domain (edge ones by their vertex pairs), and the points.
//! Mesh Boolean compares the surface only: the boolean modifier's solver splits faces otherwise
//! than the reference's (its difference of two boxes has 18 vertices and 28 edges where the
//! reference has 16 and 26).
//! With `KITION_BLENDER` naming a Blender executable the script runs again and must give the
//! recorded results; with `KITION_BLENDER_RECORD=1` as well, it writes them.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

mod common;

use std::collections::BTreeMap;

use common::Tree;
use kition_scene_model::glam::Vec3;
use kition_scene_model::scene::euler_matrix;
use kition_scene_model::{GroupSocket, Id, Mesh, NodeGroup, SocketType};
use kition_scene_nodes::field::Scalar;
use kition_scene_nodes::geometry::{Attributes, Domain, Geometry};
use serde_json::{Value, json};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/nodes-reference.json");
const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/blender/nodes.py");

/// How a case's mesh is compared.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Faces {
    /// Vertices in order, the same vertex lists.
    Exact,
    /// Vertices in order, the same lists up to where each starts.
    Rotated,
    /// Vertices as a set; each face as a loop of positions, up to where it starts.
    Positions,
    /// Vertices in order; triangles of n-gons by count, total area, and facing.
    Fill,
    /// The same surface: total area, enclosed volume, and bounds, with every reference vertex
    /// among ours (Mesh Boolean, whose topology differs from the reference's).
    Shape,
}

struct Case {
    name: &'static str,
    tree: Tree,
    others: Vec<NodeGroup>,
    frames: Vec<i32>,
    faces: Faces,
    tolerance: f32,
}

fn case(name: &'static str, tree: Tree) -> Case {
    Case { name, tree, others: Vec::new(), frames: Vec::new(), faces: Faces::Exact, tolerance: 1e-4 }
}

impl Case {
    fn with(mut self, others: Vec<Tree>) -> Self {
        self.others = others.into_iter().map(|tree| tree.group).collect();
        self
    }

    fn at(mut self, frames: &[i32]) -> Self {
        self.frames = frames.to_vec();
        self
    }

    fn compared(mut self, faces: Faces) -> Self {
        self.faces = faces;
        self
    }

    fn json(&self) -> Value {
        let mut out = json!({ "group": self.tree.group });
        if !self.others.is_empty() {
            out["groups"] = json!(self.others);
        }
        if !self.frames.is_empty() {
            out["frames"] = json!(self.frames);
        }
        out
    }
}

/// Adds a node of `kind` taking `from` (node, socket) on its `input`.
fn then(tree: &mut Tree, from: (Id, &str), kind: &str, values: Value, input: &str) -> Id {
    let node = tree.add(kind, values);
    tree.link(from.0, from.1, node, input);
    node
}

/// Stores `value` (node, socket) on `geometry` under `name`.
fn store(tree: &mut Tree, geometry: (Id, &str), value: (Id, &str), name: &str, data_type: &str, domain: &str) -> Id {
    let node = tree.add("storeNamedAttribute", json!({ "Name": name, "dataType": data_type, "domain": domain }));
    tree.link(geometry.0, geometry.1, node, "Geometry");
    tree.link(value.0, value.1, node, "Value");
    node
}

/// Joins `parts`, in order.
fn join(tree: &mut Tree, parts: &[(Id, &str)]) -> Id {
    let node = tree.add("joinGeometry", json!({}));
    for (from, socket) in parts {
        tree.link(*from, socket, node, "Geometry");
    }
    node
}

fn grid(tree: &mut Tree, x: i32, y: i32) -> Id {
    tree.add("meshGrid", json!({ "Vertices X": x, "Vertices Y": y, "Size X": 2.0, "Size Y": 1.5 }))
}

/// A 4 × 3 grid with its vertices lifted by sin(2x) + 0.3y, so faces are not flat in a row.
fn wavy(tree: &mut Tree) -> Id {
    let grid = grid(tree, 4, 3);
    let position = tree.add("position", json!({}));
    let split = then(tree, (position, "Position"), "separateXyz", json!({}), "Vector");
    let twice = then(tree, (split, "X"), "math", json!({ "operation": "multiply", "Value_001": 2.0 }), "Value");
    let sine = then(tree, (twice, "Value"), "math", json!({ "operation": "sine" }), "Value");
    let tilt = then(tree, (split, "Y"), "math", json!({ "operation": "multiply", "Value_001": 0.3 }), "Value");
    let sum = tree.add("math", json!({ "operation": "add" }));
    tree.link(sine, "Value", sum, "Value");
    tree.link(tilt, "Value", sum, "Value_001");
    let lift = then(tree, (sum, "Value"), "combineXyz", json!({}), "Z");
    let set = then(tree, (grid, "Mesh"), "setPosition", json!({}), "Geometry");
    tree.link(lift, "Vector", set, "Offset");
    set
}

/// A selection of elements whose index is `op` (`less_than`, `equal`, ...) `value`.
fn index_is(tree: &mut Tree, op: &str, value: i32) -> Id {
    let at = tree.add("index", json!({}));
    let compare = tree.add("compare", json!({ "dataType": "int", "operation": op, "B_INT": value }));
    tree.link(at, "Index", compare, "A_INT");
    compare
}

/// A selection of elements whose position's `axis` is `op` (`less_than`, ...) `value` (by
/// place, as edge order differs from the reference's).
fn place_is(tree: &mut Tree, axis: &str, op: &str, value: f32) -> Id {
    let position = tree.add("position", json!({}));
    let split = then(tree, (position, "Position"), "separateXyz", json!({}), "Vector");
    then(tree, (split, axis), "compare", json!({ "operation": op, "B": value }), "A")
}

/// One mesh node after `source` (node, socket), to the output.
fn after(source: fn(&mut Tree) -> (Id, &'static str), kind: &str, values: Value, input: &str, output: &str) -> Tree {
    let mut tree = Tree::new();
    let from = source(&mut tree);
    let node = then(&mut tree, from, kind, values, input);
    tree.output(node, output);
    tree
}

fn wavy_mesh(tree: &mut Tree) -> (Id, &'static str) {
    (wavy(tree), "Geometry")
}

fn grid33(tree: &mut Tree) -> (Id, &'static str) {
    (grid(tree, 3, 3), "Mesh")
}

fn cube(tree: &mut Tree) -> (Id, &'static str) {
    (tree.add("meshCube", json!({ "Size": [1.0, 1.5, 2.0] })), "Mesh")
}

fn ico(tree: &mut Tree) -> (Id, &'static str) {
    (tree.add("meshIcoSphere", json!({ "Subdivisions": 1, "Radius": 1.2 })), "Mesh")
}

fn heptagon(tree: &mut Tree) -> (Id, &'static str) {
    (tree.add("meshCircle", json!({ "Vertices": 7, "fillType": "ngon" })), "Mesh")
}

/// A concave n-gon: a nine-sided circle with every other vertex pulled in.
fn star_ngon(tree: &mut Tree) -> (Id, &'static str) {
    let circle = tree.add("meshCircle", json!({ "Vertices": 10, "fillType": "ngon" }));
    let odd = tree.add("index", json!({}));
    let parity = then(tree, (odd, "Index"), "math", json!({ "operation": "modulo", "Value_001": 2.0 }), "Value");
    let pull = then(tree, (parity, "Value"), "math", json!({ "operation": "multiply_add", "Value_001": -0.55, "Value_002": 1.0 }), "Value");
    let position = tree.add("position", json!({}));
    let scaled = tree.add("vectorMath", json!({ "operation": "scale" }));
    tree.link(position, "Position", scaled, "Vector");
    tree.link(pull, "Value", scaled, "Scale");
    let set = then(tree, (circle, "Mesh"), "setPosition", json!({}), "Geometry");
    tree.link(scaled, "Vector", set, "Position");
    (set, "Geometry")
}

/// A group (id 60) lifting its geometry by its Height input and giving the lift as a field.
fn lift_group() -> Tree {
    let mut inner = Tree::other(60, "Lift");
    inner.group.inputs.push(GroupSocket { name: "Height".into(), kind: SocketType::Float, default: json!(0.25) });
    inner.group.outputs.push(GroupSocket { name: "Lift".into(), kind: SocketType::Float, default: Value::Null });
    let (input, output) = inner.ends();
    let position = inner.add("position", json!({}));
    let split = then(&mut inner, (position, "Position"), "separateXyz", json!({}), "Vector");
    let scaled = inner.add("math", json!({ "operation": "multiply" }));
    inner.link(split, "X", scaled, "Value");
    inner.link(input, "Height", scaled, "Value_001");
    let offset = then(&mut inner, (scaled, "Value"), "combineXyz", json!({}), "Z");
    let set = then(&mut inner, (input, "Geometry"), "setPosition", json!({}), "Geometry");
    inner.link(offset, "Vector", set, "Offset");
    inner.link(set, "Geometry", output, "Geometry");
    inner.link(scaled, "Value", output, "Lift");
    inner
}

/// A zone of `kind` (`repeat` or `simulation`) with `items`: its input and output nodes.
fn zone(tree: &mut Tree, kind: &str, items: &[(&str, &str)], values: Value) -> (Id, Id) {
    let items: Vec<Value> = items.iter().map(|(name, kind)| json!({ "name": name, "type": kind })).collect();
    let output = tree.add(&format!("{kind}Output"), json!({ "items": items }));
    let mut values = values;
    values["output"] = json!(output);
    let input = tree.add(&format!("{kind}Input"), values);
    (input, output)
}

/// Rotations stored as Euler angles (`rotation*` names compare as matrices).
fn euler_of(tree: &mut Tree, from: (Id, &str)) -> Id {
    then(tree, from, "rotationToEuler", json!({}), "Rotation")
}

fn cases() -> Vec<Case> {
    let mut out = Vec::new();
    // Nested groups.
    out.push(
        case("groupLift", {
            let mut tree = Tree::new();
            let grid = grid(&mut tree, 4, 3);
            let group = then(&mut tree, (grid, "Mesh"), "group", json!({ "group": 60, "Height": 0.5 }), "Geometry");
            let stored = store(&mut tree, (group, "Geometry"), (group, "Lift"), "lift", "float", "point");
            tree.output(stored, "Geometry");
            tree
        })
        .with(vec![lift_group()]),
    );
    out.push(
        case("groupDefault", {
            let mut tree = Tree::new();
            let grid = grid(&mut tree, 3, 2);
            let group = then(&mut tree, (grid, "Mesh"), "group", json!({ "group": 60 }), "Geometry");
            tree.output(group, "Geometry");
            tree
        })
        .with(vec![lift_group()]),
    );
    out.push(
        case("groupTwice", {
            let mut tree = Tree::new();
            let grid = grid(&mut tree, 4, 3);
            let first = then(&mut tree, (grid, "Mesh"), "group", json!({ "group": 60, "Height": 0.5 }), "Geometry");
            let second = then(&mut tree, (first, "Geometry"), "group", json!({ "group": 60 }), "Geometry");
            tree.link(first, "Lift", second, "Height");
            let stored = store(&mut tree, (second, "Geometry"), (second, "Lift"), "lift", "float", "point");
            tree.output(stored, "Geometry");
            tree
        })
        .with(vec![lift_group()]),
    );
    out.push(
        case("groupNested", {
            let mut tree = Tree::new();
            let line = tree.add("meshLine", json!({ "Count": 5, "Offset": [0.5, 0.2, 0.0] }));
            let group = then(&mut tree, (line, "Mesh"), "group", json!({ "group": 70, "Scale": 2.0 }), "Geometry");
            tree.output(group, "Geometry");
            tree
        })
        .with(vec![
            {
                // Group 70: scales its input, then runs group 60 with Height = Scale.
                let mut middle = Tree::other(70, "Scale and Lift");
                middle.group.inputs.push(GroupSocket { name: "Scale".into(), kind: SocketType::Float, default: json!(1.0) });
                let (input, output) = middle.ends();
                let transform = then(&mut middle, (input, "Geometry"), "transform", json!({}), "Geometry");
                let vector = then(&mut middle, (input, "Scale"), "combineXyz", json!({ "Y": 1.0, "Z": 1.0 }), "X");
                middle.link(vector, "Vector", transform, "Scale");
                let lift = then(&mut middle, (transform, "Geometry"), "group", json!({ "group": 60 }), "Geometry");
                middle.link(input, "Scale", lift, "Height");
                middle.link(lift, "Geometry", output, "Geometry");
                middle
            },
            lift_group(),
        ]),
    );
    out.push(
        case("groupGeometryInput", {
            let mut tree = Tree::new();
            let line = tree.add("meshLine", json!({ "Count": 3, "Offset": [1.5, 0.0, 0.0] }));
            let cube = tree.add("meshCube", json!({ "Size": [0.5, 0.5, 0.5] }));
            let group = then(&mut tree, (line, "Mesh"), "group", json!({ "group": 80 }), "Geometry");
            tree.link(cube, "Mesh", group, "Instance");
            let stored = store(&mut tree, (group, "Geometry"), (group, "Count"), "count", "int", "point");
            tree.output(stored, "Geometry");
            tree
        })
        .with(vec![{
            let mut inner = Tree::other(80, "Copies");
            inner.group.inputs.push(GroupSocket { name: "Instance".into(), kind: SocketType::Geometry, default: Value::Null });
            inner.group.outputs.push(GroupSocket { name: "Count".into(), kind: SocketType::Int, default: Value::Null });
            let (input, output) = inner.ends();
            let on = then(&mut inner, (input, "Geometry"), "instanceOnPoints", json!({}), "Points");
            inner.link(input, "Instance", on, "Instance");
            let real = then(&mut inner, (on, "Instances"), "realizeInstances", json!({}), "Geometry");
            let size = then(&mut inner, (real, "Geometry"), "domainSize", json!({}), "Geometry");
            inner.link(real, "Geometry", output, "Geometry");
            inner.link(size, "Face Count", output, "Count");
            inner
        }]),
    );
    // Repeat zones.
    out.push(case("repeatLift", {
        let mut tree = Tree::new();
        let grid = grid(&mut tree, 3, 3);
        let (input, output) = zone(&mut tree, "repeat", &[("Geometry", "geometry"), ("Count", "int")], json!({ "Iterations": 3 }));
        tree.link(grid, "Mesh", input, "Geometry");
        let set = then(&mut tree, (input, "Geometry"), "setPosition", json!({ "Offset": [0.0, 0.0, 0.25] }), "Geometry");
        let more = then(&mut tree, (input, "Count"), "math", json!({ "operation": "add", "Value_001": 1.0 }), "Value");
        tree.link(set, "Geometry", output, "Geometry");
        tree.link(more, "Value", output, "Count");
        let stored = store(&mut tree, (output, "Geometry"), (output, "Count"), "count", "int", "point");
        tree.output(stored, "Geometry");
        tree
    }));
    out.push(case("repeatIteration", {
        let mut tree = Tree::new();
        let line = tree.add("meshLine", json!({ "Count": 4, "Offset": [0.5, 0.0, 0.0] }));
        let (input, output) = zone(&mut tree, "repeat", &[("Geometry", "geometry")], json!({ "Iterations": 4 }));
        tree.link(line, "Mesh", input, "Geometry");
        let step = then(&mut tree, (input, "Iteration"), "math", json!({ "operation": "multiply", "Value_001": 0.1 }), "Value");
        let offset = then(&mut tree, (step, "Value"), "combineXyz", json!({}), "Z");
        let at = tree.add("index", json!({}));
        let picked = tree.add("compare", json!({ "dataType": "int", "operation": "equal" }));
        tree.link(at, "Index", picked, "A_INT");
        tree.link(input, "Iteration", picked, "B_INT");
        let set = then(&mut tree, (input, "Geometry"), "setPosition", json!({}), "Geometry");
        tree.link(offset, "Vector", set, "Offset");
        tree.link(picked, "Result", set, "Selection");
        tree.link(set, "Geometry", output, "Geometry");
        tree.output(output, "Geometry");
        tree
    }));
    out.push(case("repeatZero", {
        let mut tree = Tree::new();
        let grid = grid(&mut tree, 2, 2);
        let (input, output) = zone(&mut tree, "repeat", &[("Geometry", "geometry")], json!({ "Iterations": 0 }));
        tree.link(grid, "Mesh", input, "Geometry");
        let set = then(&mut tree, (input, "Geometry"), "setPosition", json!({ "Offset": [0.0, 0.0, 1.0] }), "Geometry");
        tree.link(set, "Geometry", output, "Geometry");
        tree.output(output, "Geometry");
        tree
    }));
    out.push(case("repeatGrow", {
        let mut tree = Tree::new();
        let cube = tree.add("meshCube", json!({ "Size": [0.5, 0.5, 0.5] }));
        let (input, output) = zone(&mut tree, "repeat", &[("Geometry", "geometry"), ("Step", "float")], json!({ "Iterations": 3, "Step": 0.75 }));
        tree.link(cube, "Mesh", input, "Geometry");
        let moved = then(&mut tree, (input, "Geometry"), "transform", json!({}), "Geometry");
        let shift = then(&mut tree, (input, "Step"), "combineXyz", json!({ "Z": 0.1 }), "X");
        tree.link(shift, "Vector", moved, "Translation");
        let joined = join(&mut tree, &[(input, "Geometry"), (moved, "Geometry")]);
        let doubled = then(&mut tree, (input, "Step"), "math", json!({ "operation": "multiply", "Value_001": 2.0 }), "Value");
        tree.link(joined, "Geometry", output, "Geometry");
        tree.link(doubled, "Value", output, "Step");
        tree.output(output, "Geometry");
        tree
    }));
    out.push(case("repeatNested", {
        let mut tree = Tree::new();
        let line = tree.add("meshLine", json!({ "Count": 2, "Offset": [1.0, 0.0, 0.0] }));
        let (outer_in, outer_out) = zone(&mut tree, "repeat", &[("Geometry", "geometry")], json!({ "Iterations": 2 }));
        tree.link(line, "Mesh", outer_in, "Geometry");
        let (inner_in, inner_out) = zone(&mut tree, "repeat", &[("Geometry", "geometry")], json!({ "Iterations": 3 }));
        tree.link(outer_in, "Geometry", inner_in, "Geometry");
        let set = then(&mut tree, (inner_in, "Geometry"), "setPosition", json!({ "Offset": [0.0, 0.1, 0.0] }), "Geometry");
        tree.link(set, "Geometry", inner_out, "Geometry");
        let up = then(&mut tree, (inner_out, "Geometry"), "setPosition", json!({ "Offset": [0.0, 0.0, 1.0] }), "Geometry");
        tree.link(up, "Geometry", outer_out, "Geometry");
        tree.output(outer_out, "Geometry");
        tree
    }));
    // Simulation zones, from the start frame (1).
    out.push(
        case("simulationCount", {
            let mut tree = Tree::new();
            let grid = grid(&mut tree, 2, 2);
            let (input, output) = zone(&mut tree, "simulation", &[("Geometry", "geometry"), ("Count", "float")], json!({}));
            tree.link(grid, "Mesh", input, "Geometry");
            let more = then(&mut tree, (input, "Count"), "math", json!({ "operation": "add", "Value_001": 1.0 }), "Value");
            let offset = then(&mut tree, (more, "Value"), "combineXyz", json!({}), "Z");
            let set = then(&mut tree, (input, "Geometry"), "setPosition", json!({}), "Geometry");
            tree.link(offset, "Vector", set, "Offset");
            let delta = store(&mut tree, (set, "Geometry"), (input, "Delta Time"), "delta", "float", "point");
            tree.link(delta, "Geometry", output, "Geometry");
            tree.link(more, "Value", output, "Count");
            let stored = store(&mut tree, (output, "Geometry"), (output, "Count"), "count", "float", "point");
            tree.output(stored, "Geometry");
            tree
        })
        .at(&[-1, 0, 1, 2, 3, 6]),
    );
    out.push(
        case("simulationSkip", {
            let mut tree = Tree::new();
            let line = tree.add("meshLine", json!({ "Count": 3, "Offset": [0.4, 0.0, 0.0] }));
            let (input, output) = zone(&mut tree, "simulation", &[("Geometry", "geometry")], json!({}));
            tree.link(line, "Mesh", input, "Geometry");
            let time = tree.add("sceneTime", json!({}));
            let late = then(&mut tree, (time, "Frame"), "compare", json!({ "operation": "greater_than", "B": 3.5 }), "A");
            tree.link(late, "Result", output, "Skip");
            let set = then(&mut tree, (input, "Geometry"), "setPosition", json!({ "Offset": [0.0, 0.0, 0.5] }), "Geometry");
            tree.link(set, "Geometry", output, "Geometry");
            tree.output(output, "Geometry");
            tree
        })
        .at(&[1, 3, 4, 6]),
    );
    out.push(
        case("simulationTime", {
            let mut tree = Tree::new();
            let line = tree.add("meshLine", json!({ "Count": 2, "Offset": [1.0, 0.0, 0.0] }));
            let (input, output) =
                zone(&mut tree, "simulation", &[("Geometry", "geometry"), ("Speed", "vector")], json!({ "Speed": [0.0, 0.0, 2.0] }));
            tree.link(line, "Mesh", input, "Geometry");
            let time = tree.add("sceneTime", json!({}));
            let step = tree.add("vectorMath", json!({ "operation": "scale" }));
            tree.link(input, "Speed", step, "Vector");
            tree.link(input, "Delta Time", step, "Scale");
            let set = then(&mut tree, (input, "Geometry"), "setPosition", json!({}), "Geometry");
            tree.link(step, "Vector", set, "Offset");
            let slower = then(&mut tree, (input, "Speed"), "vectorMath", json!({ "operation": "scale", "Scale": 0.5 }), "Vector");
            let stamp = store(&mut tree, (set, "Geometry"), (time, "Seconds"), "seconds", "float", "point");
            tree.link(stamp, "Geometry", output, "Geometry");
            tree.link(slower, "Vector", output, "Speed");
            tree.output(output, "Geometry");
            tree
        })
        .at(&[1, 2, 4, 8]),
    );
    out.push(
        case("sceneTime", {
            let mut tree = Tree::new();
            let grid = grid(&mut tree, 2, 2);
            let time = tree.add("sceneTime", json!({}));
            let seconds = store(&mut tree, (grid, "Mesh"), (time, "Seconds"), "seconds", "float", "point");
            let frame = store(&mut tree, (seconds, "Geometry"), (time, "Frame"), "frame", "float", "point");
            tree.output(frame, "Geometry");
            tree
        })
        .at(&[1, 5]),
    );
    // Mesh nodes.
    out.push(case("flipFaces", {
        let mut tree = Tree::new();
        let (grid, socket) = grid33(&mut tree);
        let flip = then(&mut tree, (grid, socket), "flipFaces", json!({}), "Mesh");
        let odd = index_is(&mut tree, "not_equal", 2);
        tree.link(odd, "Result", flip, "Selection");
        let corner = tree.add("index", json!({}));
        let stored = store(&mut tree, (grid, socket), (corner, "Index"), "corner", "int", "corner");
        tree.link(stored, "Geometry", flip, "Mesh");
        tree.group.links.retain(|link| !(link.to == flip && link.from == grid));
        tree.output(flip, "Mesh");
        tree
    }));
    out.push(case("splitEdgesAll", after(grid33, "splitEdges", json!({}), "Mesh", "Mesh")));
    out.push(case("splitEdgesSome", {
        let mut tree = Tree::new();
        let (grid, socket) = wavy_mesh(&mut tree);
        let split = then(&mut tree, (grid, socket), "splitEdges", json!({}), "Mesh");
        let some = place_is(&mut tree, "X", "less_than", 0.0);
        tree.link(some, "Result", split, "Selection");
        let at = tree.add("index", json!({}));
        let stored = store(&mut tree, (split, "Mesh"), (at, "Index"), "face", "int", "face");
        tree.output(stored, "Geometry");
        tree
    }));
    out.push(case("splitEdgesCube", after(cube, "splitEdges", json!({}), "Mesh", "Mesh")));
    for (name, method) in [
        ("triangulateShortest", "shortest_diagonal"),
        ("triangulateLongest", "longest_diagonal"),
        ("triangulateFixed", "fixed"),
        ("triangulateAlternate", "fixed_alternate"),
        ("triangulateBeauty", "beauty"),
    ] {
        out.push(case(name, after(wavy_mesh, "triangulate", json!({ "quadMethod": method }), "Mesh", "Mesh")));
    }
    out.push(case("triangulateSome", {
        let mut tree = Tree::new();
        let (grid, socket) = wavy_mesh(&mut tree);
        let tri = then(&mut tree, (grid, socket), "triangulate", json!({}), "Mesh");
        let some = index_is(&mut tree, "greater_than", 2);
        tree.link(some, "Result", tri, "Selection");
        let at = tree.add("index", json!({}));
        let stored = store(&mut tree, (grid, socket), (at, "Index"), "face", "int", "face");
        tree.link(stored, "Geometry", tri, "Mesh");
        tree.group.links.retain(|link| !(link.to == tri && link.from == grid));
        tree.output(tri, "Mesh");
        tree
    }));
    out.push(case("triangulateClip", after(heptagon, "triangulate", json!({ "ngonMethod": "clip" }), "Mesh", "Mesh")));
    out.push(case("triangulateClipConcave", after(star_ngon, "triangulate", json!({ "ngonMethod": "clip" }), "Mesh", "Mesh")).compared(Faces::Fill));
    out.push(case("triangulateNgonBeauty", after(heptagon, "triangulate", json!({ "ngonMethod": "beauty" }), "Mesh", "Mesh")).compared(Faces::Fill));
    out.push(case("dualGrid", after(wavy_mesh, "dualMesh", json!({}), "Mesh", "Dual Mesh")).compared(Faces::Rotated));
    out.push(case("dualGridKeep", after(wavy_mesh, "dualMesh", json!({ "Keep Boundaries": true }), "Mesh", "Dual Mesh")).compared(Faces::Positions));
    out.push(case("dualCube", after(cube, "dualMesh", json!({}), "Mesh", "Dual Mesh")).compared(Faces::Rotated));
    out.push(case("dualIco", after(ico, "dualMesh", json!({}), "Mesh", "Dual Mesh")).compared(Faces::Rotated));
    out.push(case("scaleFaces", {
        let mut tree = Tree::new();
        let (grid, socket) = wavy_mesh(&mut tree);
        let scale = then(&mut tree, (grid, socket), "scaleElements", json!({ "Scale": 0.5 }), "Geometry");
        let some = tree.add("compare", json!({ "dataType": "int", "operation": "not_equal", "B_INT": 4 }));
        let at = tree.add("index", json!({}));
        tree.link(at, "Index", some, "A_INT");
        tree.link(some, "Result", scale, "Selection");
        tree.output(scale, "Geometry");
        tree
    }));
    out.push(case("scaleFacesApart", {
        let mut tree = Tree::new();
        let (grid, socket) = wavy_mesh(&mut tree);
        let split = then(&mut tree, (grid, socket), "splitEdges", json!({}), "Mesh");
        let scale = then(&mut tree, (split, "Mesh"), "scaleElements", json!({ "Scale": 0.6 }), "Geometry");
        // Center reads each face's centre unless linked.
        let centre = tree.add("vector", json!({ "Vector": [0.0, 0.0, 1.0] }));
        tree.link(centre, "Vector", scale, "Center");
        tree.output(scale, "Geometry");
        tree
    }));
    out.push(case("scaleFacesAxis", {
        let mut tree = Tree::new();
        let (cube, socket) = cube(&mut tree);
        let split = then(&mut tree, (cube, socket), "splitEdges", json!({}), "Mesh");
        let scale = then(
            &mut tree,
            (split, "Mesh"),
            "scaleElements",
            json!({ "Scale": 1.5, "scaleMode": "single_axis", "Axis": [1.0, 1.0, 0.0] }),
            "Geometry",
        );
        tree.output(scale, "Geometry");
        tree
    }));
    out.push(case("scaleEdges", {
        let mut tree = Tree::new();
        let (grid, socket) = wavy_mesh(&mut tree);
        let scale = then(&mut tree, (grid, socket), "scaleElements", json!({ "Scale": 0.5, "domain": "edge" }), "Geometry");
        let some = place_is(&mut tree, "X", "greater_than", 0.5);
        tree.link(some, "Result", scale, "Selection");
        tree.output(scale, "Geometry");
        tree
    }));
    out.push(
        case("meshFields", {
            let mut tree = Tree::new();
            let cube = tree.add("meshCube", json!({}));
            let extrude = then(&mut tree, (cube, "Mesh"), "extrudeMesh", json!({ "Offset Scale": -0.3 }), "Mesh");
            let top = index_is(&mut tree, "equal", 2);
            tree.link(top, "Result", extrude, "Selection");
            let line = tree.add("meshLine", json!({ "Count": 3, "Offset": [0.0, 0.0, 0.5] }));
            let moved = then(&mut tree, (line, "Mesh"), "transform", json!({ "Translation": [3.0, 0.0, 0.0] }), "Geometry");
            let joined = join(&mut tree, &[(moved, "Geometry"), (extrude, "Mesh")]);
            let angle = tree.add("edgeAngle", json!({}));
            let neighbours = tree.add("edgeNeighbors", json!({}));
            let area = tree.add("faceArea", json!({}));
            let island = tree.add("meshIsland", json!({}));
            let mut last = (joined, "Geometry");
            for (name, value, data_type, domain) in [
                ("unsigned", (angle, "Unsigned Angle"), "float", "edge"),
                ("signed", (angle, "Signed Angle"), "float", "edge"),
                ("signedPoint", (angle, "Signed Angle"), "float", "point"),
                ("faces", (neighbours, "Face Count"), "int", "edge"),
                ("area", (area, "Area"), "float", "face"),
                ("areaPoint", (area, "Area"), "float", "point"),
                ("island", (island, "Island Index"), "int", "point"),
                ("islandFace", (island, "Island Index"), "int", "face"),
                ("islands", (island, "Island Count"), "int", "point"),
            ] {
                last = (store(&mut tree, last, value, name, data_type, domain), "Geometry");
            }
            tree.output(last.0, last.1);
            tree
        })
        // Extrusion's side faces start at another corner than the reference's.
        .compared(Faces::Rotated),
    );
    for (name, operation) in [("booleanDifference", "difference"), ("booleanUnion", "union"), ("booleanIntersect", "intersect")] {
        out.push(
            case(name, {
                let mut tree = Tree::new();
                let big = tree.add("meshCube", json!({ "Size": [2.0, 2.0, 2.0] }));
                let small = tree.add("meshCube", json!({}));
                let moved = then(&mut tree, (small, "Mesh"), "transform", json!({ "Translation": [1.0, 0.0, 0.3] }), "Geometry");
                let boolean = tree.add("meshBoolean", json!({ "operation": operation }));
                if operation == "difference" {
                    tree.link(big, "Mesh", boolean, "Mesh 1");
                } else {
                    tree.link(big, "Mesh", boolean, "Mesh 2");
                }
                tree.link(moved, "Geometry", boolean, "Mesh 2");
                tree.output(boolean, "Mesh");
                tree
            })
            .compared(Faces::Shape),
        );
    }
    // Sampling.
    out.push(case("sampleIndex", {
        let mut tree = Tree::new();
        let (grid, socket) = wavy_mesh(&mut tree);
        let other = tree.add("meshLine", json!({ "Count": 5, "Offset": [0.1, 0.3, 0.2] }));
        let sample = tree.add("sampleIndex", json!({ "dataType": "float_vector" }));
        let position = tree.add("position", json!({}));
        tree.link(other, "Mesh", sample, "Geometry");
        tree.link(position, "Position", sample, "Value");
        let at = tree.add("index", json!({}));
        let less = then(&mut tree, (at, "Index"), "math", json!({ "operation": "subtract", "Value_001": 4.0 }), "Value");
        tree.link(less, "Value", sample, "Index");
        let clamped = tree.add("sampleIndex", json!({ "dataType": "float_vector", "clamp": "true" }));
        tree.link(other, "Mesh", clamped, "Geometry");
        tree.link(position, "Position", clamped, "Value");
        tree.link(less, "Value", clamped, "Index");
        let faces = tree.add("sampleIndex", json!({ "domain": "face" }));
        let area = tree.add("faceArea", json!({}));
        tree.link(grid, socket, faces, "Geometry");
        tree.link(area, "Area", faces, "Value");
        tree.link(at, "Index", faces, "Index");
        let first = store(&mut tree, (grid, socket), (sample, "Value"), "sampled", "float_vector", "point");
        let second = store(&mut tree, (first, "Geometry"), (clamped, "Value"), "clamped", "float_vector", "point");
        let third = store(&mut tree, (second, "Geometry"), (faces, "Value"), "area", "float", "point");
        tree.output(third, "Geometry");
        tree
    }));
    for (name, domain) in [("sampleNearestPoint", "point"), ("sampleNearestFace", "face"), ("sampleNearestEdge", "edge")] {
        out.push(case(name, {
            let mut tree = Tree::new();
            let (grid, socket) = wavy_mesh(&mut tree);
            let (sphere, sphere_socket) = ico(&mut tree);
            let nearest = tree.add("sampleNearest", json!({ "domain": domain }));
            tree.link(sphere, sphere_socket, nearest, "Geometry");
            let stored = if domain == "edge" {
                (grid, socket)
            } else {
                (store(&mut tree, (grid, socket), (nearest, "Index"), "nearest", "int", "point"), "Geometry")
            };
            // Where the nearest element is (edge indices differ from the reference's).
            let place = tree.add("sampleIndex", json!({ "dataType": "float_vector", "domain": domain }));
            let position = tree.add("position", json!({}));
            tree.link(sphere, sphere_socket, place, "Geometry");
            tree.link(position, "Position", place, "Value");
            tree.link(nearest, "Index", place, "Index");
            let placed = store(&mut tree, stored, (place, "Value"), "place", "float_vector", "point");
            tree.output(placed, "Geometry");
            tree
        }));
    }
    for (name, interpolation) in [("raycastInterpolated", "interpolated"), ("raycastNearest", "nearest")] {
        out.push(case(name, {
            let mut tree = Tree::new();
            let ground = tree.add("meshGrid", json!({ "Vertices X": 6, "Vertices Y": 5, "Size X": 3.0, "Size Y": 2.5 }));
            let lifted = then(&mut tree, (ground, "Mesh"), "transform", json!({ "Translation": [0.0, 0.0, 2.0] }), "Geometry");
            let (target, target_socket) = ico(&mut tree);
            let tilted = then(
                &mut tree,
                (target, target_socket),
                "transform",
                json!({ "Rotation": [0.3, 0.2, 0.1], "Translation": [0.1, 0.0, 0.0] }),
                "Geometry",
            );
            let ray = tree.add("raycast", json!({ "dataType": "float_vector", "interpolation": interpolation, "Ray Length": 10.0 }));
            let position = tree.add("position", json!({}));
            tree.link(tilted, "Geometry", ray, "Target Geometry");
            tree.link(position, "Position", ray, "Attribute");
            let mut last = (lifted, "Geometry");
            for (name, socket, data_type) in [
                ("hit", "Is Hit", "boolean"),
                ("place", "Hit Position", "float_vector"),
                ("normal", "Hit Normal", "float_vector"),
                ("distance", "Hit Distance", "float"),
                ("attribute", "Attribute", "float_vector"),
            ] {
                last = (store(&mut tree, last, (ray, socket), name, data_type, "point"), "Geometry");
            }
            tree.output(last.0, last.1);
            tree
        }));
    }
    for (name, element) in [("proximityPoints", "points"), ("proximityEdges", "edges"), ("proximityFaces", "faces")] {
        out.push(case(name, {
            let mut tree = Tree::new();
            let (grid, socket) = wavy_mesh(&mut tree);
            let lifted = then(&mut tree, (grid, socket), "transform", json!({ "Translation": [0.3, 0.1, 1.4] }), "Geometry");
            let (target, target_socket) = cube(&mut tree);
            let near = tree.add("geometryProximity", json!({ "targetElement": element }));
            tree.link(target, target_socket, near, "Target");
            let mut last = (lifted, "Geometry");
            for (name, socket, data_type) in
                [("place", "Position", "float_vector"), ("distance", "Distance", "float"), ("valid", "Is Valid", "boolean")]
            {
                last = (store(&mut tree, last, (near, socket), name, data_type, "point"), "Geometry");
            }
            tree.output(last.0, last.1);
            tree
        }));
    }
    out.push(case("proximityGroups", {
        let mut tree = Tree::new();
        let (grid, socket) = wavy_mesh(&mut tree);
        let lifted = then(&mut tree, (grid, socket), "transform", json!({ "Translation": [0.0, 0.0, 1.4] }), "Geometry");
        let (target, target_socket) = cube(&mut tree);
        let near = tree.add("geometryProximity", json!({ "targetElement": "faces" }));
        tree.link(target, target_socket, near, "Target");
        let face = tree.add("index", json!({}));
        let group = then(&mut tree, (face, "Index"), "math", json!({ "operation": "modulo", "Value_001": 3.0 }), "Value");
        tree.link(group, "Value", near, "Group ID");
        let point = tree.add("index", json!({}));
        let wanted = then(&mut tree, (point, "Index"), "math", json!({ "operation": "modulo", "Value_001": 4.0 }), "Value");
        tree.link(wanted, "Value", near, "Sample Group ID");
        let mut last = (lifted, "Geometry");
        for (name, socket, data_type) in [("place", "Position", "float_vector"), ("valid", "Is Valid", "boolean")] {
            last = (store(&mut tree, last, (near, socket), name, data_type, "point"), "Geometry");
        }
        tree.output(last.0, last.1);
        tree
    }));
    // Instances.
    for (name, kind, values) in [
        ("translateLocal", "translateInstances", json!({ "Translation": [0.2, 0.3, 0.4] })),
        ("translateGlobal", "translateInstances", json!({ "Translation": [0.2, 0.3, 0.4], "Local Space": false })),
        ("rotateLocal", "rotateInstances", json!({ "Rotation": [0.3, 0.5, 0.7], "Pivot Point": [0.1, 0.2, 0.0] })),
        ("rotateGlobal", "rotateInstances", json!({ "Rotation": [0.3, 0.5, 0.7], "Pivot Point": [0.1, 0.2, 0.0], "Local Space": false })),
        ("scaleLocal", "scaleInstances", json!({ "Scale": [1.5, 0.5, 2.0], "Center": [0.2, 0.0, 0.1] })),
        ("scaleGlobal", "scaleInstances", json!({ "Scale": [1.5, 0.5, 2.0], "Center": [0.2, 0.0, 0.1], "Local Space": false })),
    ] {
        out.push(case(name, {
            let mut tree = Tree::new();
            let line = tree.add("meshLine", json!({ "Count": 3, "Offset": [2.0, 0.5, 0.0] }));
            let cube = tree.add("meshCube", json!({ "Size": [0.5, 0.8, 0.3] }));
            let on = tree.add("instanceOnPoints", json!({ "Rotation": [0.2, 0.0, 0.4], "Scale": [1.0, 2.0, 1.0] }));
            tree.link(line, "Mesh", on, "Points");
            tree.link(cube, "Mesh", on, "Instance");
            let moved = then(&mut tree, (on, "Instances"), kind, values, "Instances");
            let odd = index_is(&mut tree, "not_equal", 1);
            tree.link(odd, "Result", moved, "Selection");
            let rotation = tree.add("instanceRotation", json!({}));
            let euler = euler_of(&mut tree, (rotation, "Rotation"));
            let scale = tree.add("instanceScale", json!({}));
            let first = store(&mut tree, (moved, "Instances"), (euler, "Euler"), "rotation", "float_vector", "instance");
            let second = store(&mut tree, (first, "Geometry"), (scale, "Scale"), "scale", "float_vector", "instance");
            let real = then(&mut tree, (second, "Geometry"), "realizeInstances", json!({}), "Geometry");
            tree.output(real, "Geometry");
            tree
        }));
    }
    // Rotations, on points.
    out.push(case("rotations", {
        let mut tree = Tree::new();
        let line = tree.add("meshLine", json!({ "Count": 4, "Offset": [0.3, 0.7, -0.4] }));
        let points = then(&mut tree, (line, "Mesh"), "meshToPoints", json!({}), "Mesh");
        let position = tree.add("position", json!({}));
        let euler = then(&mut tree, (position, "Position"), "eulerToRotation", json!({}), "Euler");
        let by = tree.add("eulerToRotation", json!({ "Euler": [0.4, -0.3, 1.1] }));
        let mut last = (points, "Points");
        let mut add = |tree: &mut Tree, name: &str, node: Id, socket: &str, rotation: bool| {
            let value = if rotation { (euler_of(tree, (node, socket)), "Euler") } else { (node, socket) };
            last = (store(tree, last, value, name, "float_vector", "point"), "Geometry");
        };
        for (name, values) in [
            ("rotationEulerObject", json!({ "Rotate By": [0.4, -0.3, 1.1] })),
            ("rotationEulerLocal", json!({ "Rotate By": [0.4, -0.3, 1.1], "space": "local" })),
            ("rotationAxisObject", json!({ "rotationType": "axis_angle", "Axis": [1.0, 2.0, 0.5], "Angle": 0.8 })),
            ("rotationAxisLocal", json!({ "rotationType": "axis_angle", "Axis": [1.0, 2.0, 0.5], "Angle": 0.8, "space": "local" })),
        ] {
            let node = tree.add("rotateEuler", values);
            tree.link(position, "Position", node, "Rotation");
            add(&mut tree, name, node, "Rotation", false);
        }
        for (name, space) in [("rotationGlobal", "global"), ("rotationLocal", "local")] {
            let node = tree.add("rotateRotation", json!({ "rotationSpace": space }));
            tree.link(euler, "Rotation", node, "Rotation");
            tree.link(by, "Rotation", node, "Rotate By");
            add(&mut tree, name, node, "Rotation", true);
        }
        let vector = tree.add("rotateVector", json!({ "Vector": [1.0, 0.5, -0.2] }));
        tree.link(euler, "Rotation", vector, "Rotation");
        add(&mut tree, "turned", vector, "Vector", false);
        let axis = tree.add("axisAngleToRotation", json!({ "Axis": [0.2, 1.0, 0.3] }));
        let split = then(&mut tree, (position, "Position"), "separateXyz", json!({}), "Vector");
        tree.link(split, "Y", axis, "Angle");
        add(&mut tree, "rotationAxisAngle", axis, "Rotation", true);
        let inverse = then(&mut tree, (euler, "Rotation"), "invertRotation", json!({}), "Rotation");
        add(&mut tree, "rotationInverse", inverse, "Rotation", true);
        tree.output(last.0, last.1);
        tree
    }));
    // Attributes and statistics.
    out.push(case("domainSize", {
        let mut tree = Tree::new();
        let (grid, socket) = wavy_mesh(&mut tree);
        let (sphere, sphere_socket) = ico(&mut tree);
        let curve = tree.add("curveCircle", json!({ "Resolution": 6 }));
        let line = tree.add("meshLine", json!({ "Count": 7 }));
        let points = then(&mut tree, (line, "Mesh"), "meshToPoints", json!({}), "Mesh");
        let cube = tree.add("meshCube", json!({}));
        let instances = tree.add("instanceOnPoints", json!({}));
        tree.link(points, "Points", instances, "Points");
        tree.link(cube, "Mesh", instances, "Instance");
        let all = join(&mut tree, &[(sphere, sphere_socket), (curve, "Curve"), (points, "Points"), (instances, "Instances")]);
        let mut last = (grid, socket);
        for (component, sockets) in [
            ("mesh", &["Point Count", "Edge Count", "Face Count", "Face Corner Count"][..]),
            ("pointcloud", &["Point Count"][..]),
            ("curve", &["Point Count", "Spline Count"][..]),
            ("instances", &["Instance Count"][..]),
        ] {
            let size = tree.add("domainSize", json!({ "component": component }));
            tree.link(all, "Geometry", size, "Geometry");
            for socket in sockets {
                let name = format!("{component} {socket}").to_lowercase().replace(' ', "_");
                last = (store(&mut tree, last, (size, socket), Box::leak(name.into_boxed_str()), "int", "point"), "Geometry");
            }
        }
        tree.output(last.0, last.1);
        tree
    }));
    out.push(case("attributeStatistic", {
        let mut tree = Tree::new();
        let (grid, socket) = wavy_mesh(&mut tree);
        let position = tree.add("position", json!({}));
        let split = then(&mut tree, (position, "Position"), "separateXyz", json!({}), "Vector");
        let floats = tree.add("attributeStatistic", json!({}));
        tree.link(grid, socket, floats, "Geometry");
        tree.link(split, "Z", floats, "Attribute");
        let vectors = tree.add("attributeStatistic", json!({ "dataType": "float_vector", "domain": "face" }));
        tree.link(grid, socket, vectors, "Geometry");
        tree.link(position, "Position", vectors, "Attribute");
        let some = index_is(&mut tree, "less_than", 4);
        tree.link(some, "Result", vectors, "Selection");
        let mut last = (grid, socket);
        for (statistic, socket) in [
            ("mean", "Mean"),
            ("median", "Median"),
            ("sum", "Sum"),
            ("min", "Min"),
            ("max", "Max"),
            ("range", "Range"),
            ("deviation", "Standard Deviation"),
            ("variance", "Variance"),
        ] {
            last = (store(&mut tree, last, (floats, socket), statistic, "float", "point"), "Geometry");
            let name = Box::leak(format!("{statistic}Vector").into_boxed_str());
            last = (store(&mut tree, last, (vectors, socket), name, "float_vector", "point"), "Geometry");
        }
        tree.output(last.0, last.1);
        tree
    }));
    out.push(case("accumulateField", {
        let mut tree = Tree::new();
        let (grid, socket) = wavy_mesh(&mut tree);
        let position = tree.add("position", json!({}));
        let split = then(&mut tree, (position, "Position"), "separateXyz", json!({}), "Vector");
        let at = tree.add("index", json!({}));
        let group = then(&mut tree, (at, "Index"), "math", json!({ "operation": "modulo", "Value_001": 3.0 }), "Value");
        let floats = tree.add("accumulateField", json!({}));
        tree.link(split, "Z", floats, "Value");
        tree.link(group, "Value", floats, "Group Index");
        let ints = tree.add("accumulateField", json!({ "dataType": "int" }));
        tree.link(at, "Index", ints, "Value");
        let faces = tree.add("accumulateField", json!({ "dataType": "float_vector", "domain": "face" }));
        tree.link(position, "Position", faces, "Value");
        let mut last = (grid, socket);
        for (name, value, data_type, domain) in [
            ("leading", (floats, "Leading"), "float", "point"),
            ("trailing", (floats, "Trailing"), "float", "point"),
            ("total", (floats, "Total"), "float", "point"),
            ("count", (ints, "Leading"), "int", "point"),
            ("countTotal", (ints, "Total"), "int", "point"),
            ("faceLeading", (faces, "Leading"), "float_vector", "face"),
            ("faceOnPoints", (faces, "Trailing"), "float_vector", "point"),
        ] {
            last = (store(&mut tree, last, value, name, data_type, domain), "Geometry");
        }
        tree.output(last.0, last.1);
        tree
    }));
    out.push(case("namedAttribute", {
        let mut tree = Tree::new();
        let (grid, socket) = wavy_mesh(&mut tree);
        let at = tree.add("index", json!({}));
        let half = then(&mut tree, (at, "Index"), "math", json!({ "operation": "multiply", "Value_001": 0.5 }), "Value");
        let stored = store(&mut tree, (grid, socket), (half, "Value"), "height", "float", "face");
        let read = tree.add("namedAttribute", json!({ "Name": "height" }));
        let missing = tree.add("namedAttribute", json!({ "Name": "nothing", "dataType": "int" }));
        let place = tree.add("namedAttribute", json!({ "Name": "position", "dataType": "float_vector" }));
        let mut last = (stored, "Geometry");
        for (name, value, data_type) in [
            ("read", (read, "Attribute"), "float"),
            ("exists", (read, "Exists"), "boolean"),
            ("missing", (missing, "Attribute"), "int"),
            ("missingExists", (missing, "Exists"), "boolean"),
            ("place", (place, "Attribute"), "float_vector"),
        ] {
            last = (store(&mut tree, last, value, name, data_type, "point"), "Geometry");
        }
        tree.output(last.0, last.1);
        tree
    }));
    out
}

fn vec3(value: &Value) -> Vec3 {
    let item = |index: usize| value[index].as_f64().unwrap() as f32;
    Vec3::new(item(0), item(1), item(2))
}

fn vectors(value: &Value) -> Vec<Vec3> {
    value.as_array().unwrap().iter().map(vec3).collect()
}

fn worst_apart(what: &str, ours: &[Vec3], theirs: &[Vec3]) -> f32 {
    assert_eq!(ours.len(), theirs.len(), "{what}: count");
    ours.iter().zip(theirs).fold(0.0f32, |worst, (a, b)| worst.max(a.distance(*b)))
}

/// Pairs each of our vertices with the nearest unused one of theirs: our index to theirs, and
/// the worst distance.
fn match_vertices(what: &str, ours: &[Vec3], theirs: &[Vec3]) -> (Vec<usize>, f32) {
    assert_eq!(ours.len(), theirs.len(), "{what}: vertex count");
    let mut used = vec![false; theirs.len()];
    let mut map = Vec::new();
    let mut worst = 0.0f32;
    for point in ours {
        let (best, distance) = theirs
            .iter()
            .enumerate()
            .filter(|(index, _)| !used[*index])
            .map(|(index, other)| (index, point.distance(*other)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        used[best] = true;
        map.push(best);
        worst = worst.max(distance);
    }
    (map, worst)
}

fn edge_key(a: u64, b: u64) -> (u64, u64) {
    (a.min(b), a.max(b))
}

fn scalar_difference(what: &str, name: &str, mine: &Scalar, theirs: &Value) -> f32 {
    match (mine, theirs) {
        (Scalar::Vector(mine), Value::Array(_)) if name.starts_with("rotation") => {
            let (a, b) = (euler_matrix(*mine), euler_matrix(vec3(theirs)));
            (a - b).to_cols_array().iter().fold(0.0f32, |most, value| most.max(value.abs()))
        }
        (Scalar::Vector(mine), Value::Array(_)) => mine.distance(vec3(theirs)),
        (Scalar::Float(mine), Value::Number(theirs)) => (mine - theirs.as_f64().unwrap() as f32).abs(),
        (Scalar::Int(mine), Value::Number(theirs)) => {
            assert_eq!(i64::from(*mine), theirs.as_i64().unwrap(), "{what}");
            0.0
        }
        (Scalar::Bool(mine), Value::Bool(theirs)) => {
            assert_eq!(mine, theirs, "{what}");
            0.0
        }
        (mine, theirs) => panic!("{what}: {mine:?} against {theirs}"),
    }
}

fn parse_domain(text: &str) -> Domain {
    match text {
        "edge" => Domain::Edge,
        "face" => Domain::Face,
        "corner" => Domain::Corner,
        "instance" => Domain::Instance,
        "curve" => Domain::Curve,
        _ => Domain::Point,
    }
}

/// Compares named attributes. `points` maps our vertices to theirs when vertices compare as a
/// set; edge attributes compare by vertex pairs (`our_edges`, `their_edges`).
fn compare_attributes(
    what: &str,
    ours: &Attributes,
    theirs: &Value,
    points: Option<&[usize]>,
    our_edges: &[[u32; 2]],
    their_edges: &[(u64, u64)],
) -> f32 {
    let mut worst = 0.0f32;
    for (name, recorded) in theirs.as_object().unwrap() {
        let what = format!("{what} attribute {name}");
        let attribute = ours.get(name).unwrap_or_else(|| panic!("{what}: missing"));
        let domain = parse_domain(recorded["domain"].as_str().unwrap());
        assert!(attribute.domain == domain, "{what}: domain {:?} against {domain:?}", attribute.domain);
        let values = recorded["values"].as_array().unwrap();
        assert_eq!(attribute.values.len(), values.len(), "{what}: count");
        match (domain, points) {
            (Domain::Edge, _) => {
                let map = |key: (u64, u64)| match points {
                    Some(points) => edge_key(points[key.0 as usize] as u64, points[key.1 as usize] as u64),
                    None => key,
                };
                let theirs_by: BTreeMap<(u64, u64), &Value> = their_edges.iter().copied().zip(values).collect();
                for (edge, mine) in our_edges.iter().zip(&attribute.values) {
                    let key = map(edge_key(u64::from(edge[0]), u64::from(edge[1])));
                    let theirs = theirs_by.get(&key).unwrap_or_else(|| panic!("{what}: no edge {key:?}"));
                    worst = worst.max(scalar_difference(&what, name, mine, theirs));
                }
            }
            (Domain::Point, Some(points)) => {
                for (index, mine) in attribute.values.iter().enumerate() {
                    worst = worst.max(scalar_difference(&what, name, mine, &values[points[index]]));
                }
            }
            (Domain::Corner, Some(_)) => {}
            _ => {
                for (mine, theirs) in attribute.values.iter().zip(values) {
                    worst = worst.max(scalar_difference(&what, name, mine, theirs));
                }
            }
        }
    }
    worst
}

fn area(mesh: &Mesh) -> f32 {
    mesh.faces.iter().map(|face| kition_scene_nodes::mesh_ops::face_area(mesh, face)).sum()
}

/// The volume a closed mesh encloses.
fn volume(mesh: &Mesh) -> f32 {
    mesh.triangles()
        .iter()
        .map(|([a, b, c], _)| {
            let at = |index: u32| mesh.positions[index as usize];
            at(*a).dot(at(*b).cross(at(*c))) / 6.0
        })
        .sum()
}

fn compare_shape(what: &str, ours: &Mesh, theirs: &Mesh) -> f32 {
    let (area_ours, area_theirs) = (area(ours), area(theirs));
    let (volume_ours, volume_theirs) = (volume(ours), volume(theirs));
    let (bounds_ours, bounds_theirs) =
        (ours.bounds(kition_scene_model::glam::Mat4::IDENTITY), theirs.bounds(kition_scene_model::glam::Mat4::IDENTITY));
    eprintln!(
        "{what}: {} vertices, {} faces, {} edges against {} vertices, {} faces, {} edges",
        ours.positions.len(),
        ours.faces.len(),
        ours.edges().len(),
        theirs.positions.len(),
        theirs.faces.len(),
        theirs.edges().len()
    );
    let mut worst = (area_ours - area_theirs).abs().max((volume_ours - volume_theirs).abs());
    worst = worst.max(bounds_ours.min.distance(bounds_theirs.min)).max(bounds_ours.max.distance(bounds_theirs.max));
    for point in &theirs.positions {
        let nearest = ours.positions.iter().map(|other| other.distance(*point)).fold(f32::INFINITY, f32::min);
        worst = worst.max(nearest);
    }
    worst
}

fn compare_mesh(what: &str, ours: &kition_scene_nodes::geometry::MeshComponent, theirs: &Value, faces: Faces) -> f32 {
    let mesh = &ours.mesh;
    let positions = vectors(&theirs["positions"]);
    let recorded: Vec<Vec<u32>> = theirs["faces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|face| face.as_array().unwrap().iter().map(|vertex| vertex.as_u64().unwrap() as u32).collect())
        .collect();
    let their_edges: Vec<(u64, u64)> =
        theirs["edges"].as_array().unwrap().iter().map(|edge| edge_key(edge[0].as_u64().unwrap(), edge[1].as_u64().unwrap())).collect();
    let our_edges = mesh.edges();
    if faces == Faces::Shape {
        return compare_shape(what, mesh, &Mesh { positions, faces: recorded, ..Mesh::default() });
    }
    assert_eq!(mesh.faces.len(), recorded.len(), "{what}: faces");
    assert_eq!(our_edges.len(), their_edges.len(), "{what}: edges");
    let (worst, points) = if faces == Faces::Positions {
        let (map, worst) = match_vertices(what, &mesh.positions, &positions);
        (worst, Some(map))
    } else {
        (worst_apart(&format!("{what} vertices"), &mesh.positions, &positions), None)
    };
    let mut worst = worst;
    match faces {
        Faces::Exact => assert_eq!(mesh.faces, recorded, "{what}: faces"),
        Faces::Rotated => {
            for (mine, theirs) in mesh.faces.iter().zip(&recorded) {
                let start = mine.iter().position(|vertex| *vertex == theirs[0]).unwrap_or_else(|| panic!("{what}: face {mine:?} against {theirs:?}"));
                let turned: Vec<u32> = mine.iter().cycle().skip(start).take(mine.len()).copied().collect();
                assert_eq!(&turned, theirs, "{what}: face");
            }
        }
        Faces::Positions => {
            let map = points.as_ref().unwrap();
            for (mine, theirs) in mesh.faces.iter().zip(&recorded) {
                let mapped: Vec<u32> = mine.iter().map(|vertex| map[*vertex as usize] as u32).collect();
                let start =
                    mapped.iter().position(|vertex| *vertex == theirs[0]).unwrap_or_else(|| panic!("{what}: face {mapped:?} against {theirs:?}"));
                let turned: Vec<u32> = mapped.iter().cycle().skip(start).take(mapped.len()).copied().collect();
                assert_eq!(&turned, theirs, "{what}: face");
            }
        }
        Faces::Shape => {}
        Faces::Fill => {
            let reference = Mesh { positions: positions.clone(), faces: recorded.clone(), ..Mesh::default() };
            let (mine, theirs) = (area(mesh), area(&reference));
            assert!((mine - theirs).abs() < 1e-4 * theirs.abs().max(1.0), "{what}: area {mine} against {theirs}");
            let facing =
                |mesh: &Mesh| -> Vec3 { mesh.faces.iter().map(|face| kition_scene_model::mesh::newell_vector(&mesh.face_positions(face))).sum() };
            worst = worst.max(facing(mesh).distance(facing(&reference)));
        }
    }
    if faces != Faces::Positions && faces != Faces::Fill {
        let mut mine: Vec<(u64, u64)> = our_edges.iter().map(|[a, b]| edge_key(u64::from(*a), u64::from(*b))).collect();
        let mut recorded_edges = their_edges.clone();
        mine.sort_unstable();
        recorded_edges.sort_unstable();
        assert_eq!(mine, recorded_edges, "{what}: edges");
    }
    let named: Attributes =
        ours.attributes.iter().filter(|(name, _)| !name.starts_with('.')).map(|(name, value)| (name.clone(), value.clone())).collect();
    worst.max(compare_attributes(what, &named, &theirs["attributes"], points.as_deref(), &our_edges, &their_edges))
}

fn compare(case: &Case, ours: &Geometry, theirs: &Value) -> f32 {
    let what = case.name;
    let mut worst = 0.0f32;
    assert!(ours.curves.as_ref().is_none_or(|curves| curves.curves_len() == 0), "{what}: curves came out");
    assert!(ours.instances.as_ref().is_none_or(|instances| instances.items.is_empty()), "{what}: instances came out");
    let our_mesh = ours.mesh.as_ref().filter(|mesh| !mesh.mesh.positions.is_empty());
    assert_eq!(our_mesh.is_some(), !theirs["mesh"].is_null(), "{what}: mesh");
    if let Some(mesh) = our_mesh {
        worst = worst.max(compare_mesh(what, mesh, &theirs["mesh"], case.faces));
    }
    let our_points = ours.points.as_ref().filter(|points| !points.positions.is_empty());
    assert_eq!(our_points.is_some(), !theirs["points"].is_null(), "{what}: points");
    if let Some(points) = our_points {
        let theirs = &theirs["points"];
        worst = worst.max(worst_apart(&format!("{what} points"), &points.positions, &vectors(&theirs["positions"])));
        let named: Attributes =
            points.attributes.iter().filter(|(name, _)| !name.starts_with('.')).map(|(name, value)| (name.clone(), value.clone())).collect();
        worst = worst.max(compare_attributes(what, &named, &theirs["attributes"], None, &[], &[]));
    }
    worst
}

fn base() -> Mesh {
    kition_scene_model::Primitive::Cube { size: 2.0 }.build().unwrap()
}

fn fixture() -> Value {
    serde_json::from_str(&std::fs::read_to_string(FIXTURE).expect("the recorded reference is missing")).unwrap()
}

#[test]
fn nodes_match_the_reference() {
    let recorded = fixture();
    let cases = cases();
    let mut names: Vec<&str> = cases.iter().map(|case| case.name).collect();
    let mut recorded_names: Vec<&str> = recorded["cases"].as_object().unwrap().keys().map(String::as_str).collect();
    names.sort_unstable();
    recorded_names.sort_unstable();
    assert_eq!(names, recorded_names, "the recorded cases are not this test's: record them again");
    for case in &cases {
        let entry = &recorded["cases"][case.name];
        assert_eq!(entry["case"], case.json(), "{}: the recorded case is not this one", case.name);
        let frames: Vec<Option<i32>> = if case.frames.is_empty() { vec![None] } else { case.frames.iter().copied().map(Some).collect() };
        let mut worst = 0.0f32;
        for frame in frames {
            let theirs = match frame {
                Some(frame) => &entry["result"][frame.to_string()],
                None => &entry["result"],
            };
            let ours = case.tree.run_in(base(), &case.others, frame).unwrap_or_else(|error| panic!("{}: {error}", case.name));
            worst = worst.max(compare(case, &ours, theirs));
        }
        eprintln!("{}: worst difference {worst:.2e}", case.name);
        assert!(worst <= case.tolerance, "{}: {worst} against the reference", case.name);
    }
}

#[test]
fn the_reference_gives_the_recorded_results() {
    let Some(executable) = std::env::var_os("KITION_BLENDER") else {
        eprintln!("not run again: set KITION_BLENDER to a Blender executable (the recorded results are checked either way)");
        return;
    };
    let dir = std::env::temp_dir().join(format!("kition-scene-nodes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (input, out) = (dir.join("cases.json"), dir.join("out.json"));
    let cases = cases();
    let made: serde_json::Map<String, Value> = cases.iter().map(|case| (case.name.to_owned(), case.json())).collect();
    std::fs::write(&input, Value::Object(made.clone()).to_string()).unwrap();
    let status = std::process::Command::new(executable)
        .args(["-b", "--factory-startup", "--python", SCRIPT, "--"])
        .arg(&input)
        .arg(&out)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "Blender failed");
    let fresh: Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let entries: serde_json::Map<String, Value> =
        made.into_iter().map(|(name, case)| (name.clone(), json!({ "case": case, "result": fresh["cases"][&name] }))).collect();
    let all = json!({ "blender": fresh["blender"], "cases": entries });
    if std::env::var_os("KITION_BLENDER_RECORD").is_some() {
        std::fs::write(FIXTURE, serde_json::to_string_pretty(&all).unwrap() + "\n").unwrap();
        return;
    }
    assert_eq!(all, fixture(), "the reference no longer gives the recorded results");
}
