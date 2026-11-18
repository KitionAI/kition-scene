//! Curve nodes against the reference application. `fixtures/curves-reference.json` holds what
//! Blender 5.2.3 made of each case's node group (`tests/blender/curves.py` writes it from the
//! groups below) and the group itself, which must still be the case's. Every case compares the
//! curves (point counts and offsets exactly, positions, handles, radius, tilt, and named
//! attributes within its tolerance), the mesh (vertex, face, and edge counts exactly, vertices in
//! order, faces as lists, up to rotation, or for triangle fills by area and facing), and the
//! points. With `KITION_BLENDER` naming a Blender executable the script runs again and must give
//! the recorded results; with `KITION_BLENDER_RECORD=1` as well, it writes them.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

mod common;

use common::Tree;
use kition_scene_model::glam::{Mat3, Vec3};
use kition_scene_model::scene::euler_matrix;
use kition_scene_model::{HandleType, Id, Mesh};
use kition_scene_nodes::curves::{CurveType, CurvesComponent};
use kition_scene_nodes::field::Scalar;
use kition_scene_nodes::geometry::{Attributes, Domain, Geometry};
use serde_json::{Value, json};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/curves-reference.json");
const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/blender/curves.py");

/// How a case's mesh faces are compared.
#[derive(Clone, Copy, PartialEq)]
enum Faces {
    /// The same vertex lists.
    Exact,
    /// The same lists up to where each starts.
    Rotated,
    /// Triangulated fills: the same count, area, and facing (+Z).
    Fill,
}

struct Case {
    name: &'static str,
    tree: Tree,
    faces: Faces,
    tolerance: f32,
}

fn case(name: &'static str, tree: Tree) -> Case {
    Case { name, tree, faces: Faces::Exact, tolerance: 1e-4 }
}

/// A node of `kind` alone, its `socket` to the output.
fn alone(kind: &str, values: Value, socket: &str) -> Tree {
    let mut tree = Tree::new();
    let node = tree.add(kind, values);
    tree.output(node, socket);
    tree
}

/// Adds a node taking `from` (node, socket) on its `input`.
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

fn bezier(tree: &mut Tree, resolution: i32) -> Id {
    tree.add(
        "curveBezierSegment",
        json!({ "Resolution": resolution, "Start": [-1.0, 0.2, 0.0], "Start Handle": [-0.3, 1.0, 0.5], "End Handle": [0.4, -0.6, 0.0], "End": [1.5, 0.0, 0.3] }),
    )
}

fn spiral(tree: &mut Tree) -> Id {
    tree.add("curveSpiral", json!({ "Resolution": 8, "Rotations": 1.3, "Start Radius": 0.5, "End Radius": 1.5, "Height": 1.0 }))
}

fn quad3d(tree: &mut Tree) -> Id {
    tree.add(
        "curveQuadrilateral",
        json!({ "mode": "points", "Point 1": [-1.0, -1.0, 0.0], "Point 2": [1.0, -1.0, 1.0], "Point 3": [1.0, 1.0, 0.3], "Point 4": [-1.0, 1.5, 2.0] }),
    )
}

/// The geometry output of node `id`.
fn output_of(tree: &Tree, id: Id) -> &'static str {
    match tree.group.node(id).map(|node| node.kind.as_str()) {
        Some("joinGeometry" | "transform" | "setSplineCyclic" | "setSplineResolution" | "setPosition") => "Geometry",
        _ => "Curve",
    }
}

/// One curve node after `source`, to the output.
fn after(source: fn(&mut Tree) -> Id, kind: &str, values: Value) -> Tree {
    let mut tree = Tree::new();
    let from = source(&mut tree);
    let socket = output_of(&tree, from);
    let node = then(&mut tree, (from, socket), kind, values, "Curve");
    tree.output(node, "Curve");
    tree
}

/// Curve to Mesh of `main`, with `profile` when given.
fn sweep(main: fn(&mut Tree) -> Id, profile: Option<fn(&mut Tree) -> Id>, values: Value) -> Tree {
    let mut tree = Tree::new();
    let curve = main(&mut tree);
    let socket = output_of(&tree, curve);
    let mesh = then(&mut tree, (curve, socket), "curveToMesh", values, "Curve");
    if let Some(profile) = profile {
        let profile = profile(&mut tree);
        let socket = output_of(&tree, profile);
        tree.link(profile, socket, mesh, "Profile Curve");
    }
    tree.output(mesh, "Mesh");
    tree
}

/// Curve to Points of `source`, its tangent, normal, and rotation stored on the points.
fn to_points(source: fn(&mut Tree) -> Id, values: Value) -> Tree {
    let mut tree = Tree::new();
    let curve = source(&mut tree);
    let socket = output_of(&tree, curve);
    let points = then(&mut tree, (curve, socket), "curveToPoints", values, "Curve");
    let mut last = (points, "Points");
    for (name, socket) in [("tangent", "Tangent"), ("normal", "Normal"), ("rotation", "Rotation")] {
        last = (store(&mut tree, last, (points, socket), name, "float_vector", "point"), "Geometry");
    }
    tree.output(last.0, last.1);
    tree
}

fn fill(source: fn(&mut Tree) -> Id, values: Value) -> Case {
    let mut tree = Tree::new();
    let curve = source(&mut tree);
    let socket = output_of(&tree, curve);
    let mesh = then(&mut tree, (curve, socket), "fillCurve", values, "Curve");
    tree.output(mesh, "Mesh");
    Case { faces: Faces::Fill, ..case("", tree) }
}

fn ring(tree: &mut Tree) -> Id {
    let outer = tree.add("curveCircle", json!({ "Resolution": 6 }));
    let inner = tree.add("curveCircle", json!({ "Resolution": 4, "Radius": 0.4 }));
    join(tree, &[(outer, "Curve"), (inner, "Curve")])
}

fn star(tree: &mut Tree) -> Id {
    tree.add("curveStar", json!({ "Points": 5, "Inner Radius": 0.6, "Outer Radius": 1.4, "Twist": 0.2 }))
}

fn line(tree: &mut Tree) -> Id {
    tree.add("curveLine", json!({}))
}

fn circle5(tree: &mut Tree) -> Id {
    tree.add("curveCircle", json!({ "Resolution": 5 }))
}

/// A selection of the curve with index `index`.
fn index_is(tree: &mut Tree, index: i32) -> Id {
    let at = tree.add("index", json!({}));
    let compare = tree.add("compare", json!({ "dataType": "int", "operation": "equal", "B_INT": index }));
    tree.link(at, "Index", compare, "A_INT");
    compare
}

fn cases() -> Vec<Case> {
    let out = vec![
        case("lineEnds", alone("curveLine", json!({ "Start": [0.5, -1.0, 0.25], "End": [1.0, 2.0, 3.0] }), "Curve")),
        case(
            "lineDirection",
            alone("curveLine", json!({ "mode": "direction", "Start": [0.2, 0.0, 0.0], "Direction": [1.0, 2.0, 2.0], "Length": 1.5 }), "Curve"),
        ),
        case("circleRadius", alone("curveCircle", json!({ "Resolution": 7, "Radius": 1.3 }), "Curve")),
        case("bezierPosition", {
            let mut tree = Tree::new();
            let node = bezier(&mut tree, 5);
            tree.output(node, "Curve");
            tree
        }),
        case(
            "bezierOffset",
            alone(
                "curveBezierSegment",
                json!({ "mode": "offset", "Resolution": 4, "Start Handle": [0.5, 0.8, 0.0], "End Handle": [-0.4, 0.5, 0.2] }),
                "Curve",
            ),
        ),
        case("quadRectangle", alone("curveQuadrilateral", json!({ "Width": 3.0, "Height": 1.5 }), "Curve")),
        case(
            "quadParallelogram",
            alone("curveQuadrilateral", json!({ "mode": "parallelogram", "Width": 2.0, "Height": 1.0, "Offset": 0.6 }), "Curve"),
        ),
        case(
            "quadTrapezoid",
            alone("curveQuadrilateral", json!({ "mode": "trapezoid", "Bottom Width": 3.0, "Top Width": 1.0, "Offset": 0.3, "Height": 1.2 }), "Curve"),
        ),
        case("quadKite", alone("curveQuadrilateral", json!({ "mode": "kite", "Width": 1.5, "Bottom Height": 2.0, "Top Height": 0.7 }), "Curve")),
        case("quadPoints", {
            let mut tree = Tree::new();
            let node = quad3d(&mut tree);
            tree.output(node, "Curve");
            tree
        }),
        case("spiral", {
            let mut tree = Tree::new();
            let node = spiral(&mut tree);
            tree.output(node, "Curve");
            tree
        }),
        case("spiralReverse", alone("curveSpiral", json!({ "Resolution": 6, "Rotations": 2.0, "Reverse": true }), "Curve")),
        case("circlePoints", {
            let mut tree = Tree::new();
            let circle = tree.add(
                "curveCircle",
                json!({ "mode": "points", "Resolution": 6, "Point 1": [0.0, 0.0, 0.0], "Point 2": [2.0, 0.5, 1.0], "Point 3": [1.0, 2.0, 0.0] }),
            );
            let stored = store(&mut tree, (circle, "Curve"), (circle, "Center"), "center", "float_vector", "point");
            tree.output(stored, "Geometry");
            tree
        }),
        case("star", {
            let mut tree = Tree::new();
            let node = star(&mut tree);
            let stored = store(&mut tree, (node, "Curve"), (node, "Outer Points"), "outer", "boolean", "point");
            tree.output(stored, "Geometry");
            tree
        }),
        // Curve to Mesh.
        case("wireBezier", sweep(|tree| bezier(tree, 6), None, json!({}))),
        case("wireCircle", sweep(circle5, None, json!({}))),
        case(
            "tubeSpiral",
            sweep(
                |tree| tree.add("curveSpiral", json!({ "Resolution": 6, "Rotations": 1.5 })),
                Some(|tree| tree.add("curveCircle", json!({ "Resolution": 5, "Radius": 0.2 }))),
                json!({}),
            ),
        ),
        case(
            "tubeBezierCaps",
            sweep(
                |tree| bezier(tree, 8),
                Some(|tree| tree.add("curveCircle", json!({ "Resolution": 6, "Radius": 0.3 }))),
                json!({ "Fill Caps": true }),
            ),
        ),
        case("ribbonStar", sweep(star, Some(|tree| tree.add("curveLine", json!({ "Start": [-0.1, 0.0, 0.0], "End": [0.1, 0.2, 0.05] }))), json!({}))),
        case(
            "frameClosed",
            sweep(quad3d, Some(|tree| tree.add("curveQuadrilateral", json!({ "Width": 0.2, "Height": 0.1 }))), json!({ "Fill Caps": true })),
        ),
        case("tubeRadiusTilt", {
            let mut tree = Tree::new();
            let curve = spiral(&mut tree);
            let radius = then(&mut tree, (curve, "Curve"), "setCurveRadius", json!({ "Radius": 0.5 }), "Curve");
            let tilt = then(&mut tree, (radius, "Curve"), "setCurveTilt", json!({ "Tilt": 0.7 }), "Curve");
            let mesh = then(&mut tree, (tilt, "Curve"), "curveToMesh", json!({ "Scale": 0.3, "Fill Caps": true }), "Curve");
            let profile = tree.add("curveCircle", json!({ "Resolution": 4 }));
            tree.link(profile, "Curve", mesh, "Profile Curve");
            tree.output(mesh, "Mesh");
            tree
        }),
        case("tubesTwoCurves", {
            let mut tree = Tree::new();
            let (a, b) = (line(&mut tree), bezier(&mut tree, 4));
            let both = join(&mut tree, &[(a, "Curve"), (b, "Curve")]);
            let mesh = then(&mut tree, (both, "Geometry"), "curveToMesh", json!({ "Fill Caps": true }), "Curve");
            let profile = tree.add("curveCircle", json!({ "Resolution": 4, "Radius": 0.2 }));
            tree.link(profile, "Curve", mesh, "Profile Curve");
            tree.output(mesh, "Mesh");
            tree
        }),
        case("sweepTwoProfiles", {
            let mut tree = Tree::new();
            let curve = tree.add("curveLine", json!({ "End": [0.0, 1.0, 1.0] }));
            let mesh = then(&mut tree, (curve, "Curve"), "curveToMesh", json!({}), "Curve");
            let (a, b) = (
                tree.add("curveCircle", json!({ "Resolution": 3, "Radius": 0.2 })),
                tree.add("curveLine", json!({ "Start": [0.5, 0.0, 0.0], "End": [0.8, 0.1, 0.0] })),
            );
            let both = join(&mut tree, &[(a, "Curve"), (b, "Curve")]);
            tree.link(both, "Geometry", mesh, "Profile Curve");
            tree.output(mesh, "Mesh");
            tree
        }),
        // Mesh to Curve.
        case("edgesGrid", {
            let mut tree = Tree::new();
            let grid = tree.add("meshGrid", json!({ "Vertices X": 4, "Vertices Y": 3, "Size X": 2.0, "Size Y": 1.5 }));
            let curve = then(&mut tree, (grid, "Mesh"), "meshToCurve", json!({}), "Mesh");
            tree.output(curve, "Curve");
            tree
        }),
        case("edgesCircle", {
            let mut tree = Tree::new();
            let mesh = tree.add("meshCircle", json!({ "Vertices": 6 }));
            let curve = then(&mut tree, (mesh, "Mesh"), "meshToCurve", json!({}), "Mesh");
            tree.output(curve, "Curve");
            tree
        }),
        case("edgesLine", {
            let mut tree = Tree::new();
            let mesh = tree.add("meshLine", json!({ "Count": 4 }));
            let curve = then(&mut tree, (mesh, "Mesh"), "meshToCurve", json!({}), "Mesh");
            tree.output(curve, "Curve");
            tree
        }),
        case("edgesSelected", {
            let mut tree = Tree::new();
            let grid = tree.add("meshGrid", json!({}));
            let position = tree.add("position", json!({}));
            let x = then(&mut tree, (position, "Position"), "separateXyz", json!({}), "Vector");
            let compare = then(&mut tree, (x, "X"), "compare", json!({ "B": -0.1 }), "A");
            let curve = then(&mut tree, (grid, "Mesh"), "meshToCurve", json!({}), "Mesh");
            tree.link(compare, "Result", curve, "Selection");
            tree.output(curve, "Curve");
            tree
        }),
        case("facesGrid", {
            let mut tree = Tree::new();
            let grid = tree.add("meshGrid", json!({}));
            let curve = then(&mut tree, (grid, "Mesh"), "meshToCurve", json!({ "mode": "faces" }), "Mesh");
            tree.output(curve, "Curve");
            tree
        }),
        // Curve to Points.
        case("pointsCount", to_points(spiral, json!({ "Count": 7 }))),
        case("pointsLength", to_points(|tree| bezier(tree, 6), json!({ "mode": "length", "Length": 0.35 }))),
        case("pointsEvaluated", to_points(quad3d, json!({ "mode": "evaluated" }))),
        case("pointsCyclic", to_points(circle5, json!({ "Count": 4 }))),
        case("pointsBezier", to_points(|tree| bezier(tree, 4), json!({ "Count": 9 }))),
        // Fill Curve.
        Case { name: "fillStar", ..fill(star, json!({})) },
        Case { name: "fillRing", ..fill(ring, json!({})) },
        Case { name: "fillRingNonZero", ..fill(ring, json!({ "fillRule": "non_zero" })) },
        // An open curve fills as if closed (this one does not cross itself where it closes: the
        // reference adds a vertex at such a crossing, and this fill does not).
        Case { name: "fillBezier", ..fill(|tree| tree.add("curveBezierSegment", json!({ "Resolution": 6 })), json!({})) },
        Case { name: "fillStarNgon", faces: Faces::Rotated, ..fill(star, json!({ "mode": "ngons" })) },
        Case { name: "fillQuadNgon", faces: Faces::Rotated, ..fill(quad3d, json!({ "mode": "ngons" })) },
        Case {
            faces: Faces::Fill,
            ..case("fillGroups", {
                let mut tree = Tree::new();
                let a = circle5(&mut tree);
                let b = circle5(&mut tree);
                let moved = then(&mut tree, (b, "Curve"), "transform", json!({ "Translation": [0.5, 0.0, 0.0] }), "Geometry");
                let both = join(&mut tree, &[(a, "Curve"), (moved, "Geometry")]);
                let fill = then(&mut tree, (both, "Geometry"), "fillCurve", json!({}), "Curve");
                let index = tree.add("index", json!({}));
                tree.link(index, "Index", fill, "Group ID");
                tree.output(fill, "Mesh");
                tree
            })
        },
        // Operations.
        case("resampleBezier", after(|tree| bezier(tree, 5), "resampleCurve", json!({ "Count": 7 }))),
        case("resampleCircle", after(circle5, "resampleCurve", json!({ "Count": 4 }))),
        case("resampleLength", after(spiral, "resampleCurve", json!({ "mode": "length", "Length": 0.4 }))),
        case("resampleShort", after(line, "resampleCurve", json!({ "mode": "length", "Length": 2.0 }))),
        case("resampleShortDropped", after(line, "resampleCurve", json!({ "mode": "length", "Length": 2.0, "keepLastSegment": "false" }))),
        case("resampleEvaluated", after(|tree| bezier(tree, 5), "resampleCurve", json!({ "mode": "evaluated" }))),
        case("resampleSelected", {
            let mut tree = Tree::new();
            let (a, b) = (line(&mut tree), circle5(&mut tree));
            let both = join(&mut tree, &[(a, "Curve"), (b, "Curve")]);
            let resample = then(&mut tree, (both, "Geometry"), "resampleCurve", json!({ "Count": 5 }), "Curve");
            let first = index_is(&mut tree, 0);
            tree.link(first, "Result", resample, "Selection");
            tree.output(resample, "Curve");
            tree
        }),
        case("trimSpiral", after(spiral, "trimCurve", json!({ "Start": 0.15, "End": 0.7 }))),
        case("trimBezier", after(|tree| bezier(tree, 6), "trimCurve", json!({ "Start": 0.2, "End": 0.75 }))),
        case("trimBezierShort", after(|tree| bezier(tree, 6), "trimCurve", json!({ "Start": 0.31, "End": 0.43 }))),
        case("trimBezierCyclic", {
            let mut tree = Tree::new();
            let curve = bezier(&mut tree, 6);
            let closed = then(&mut tree, (curve, "Curve"), "setSplineCyclic", json!({ "Cyclic": true }), "Geometry");
            let trim = then(&mut tree, (closed, "Geometry"), "trimCurve", json!({ "Start": 0.3, "End": 0.8 }), "Curve");
            tree.output(trim, "Curve");
            tree
        }),
        case("trimBezierWrap", {
            let mut tree = Tree::new();
            let curve = bezier(&mut tree, 6);
            let closed = then(&mut tree, (curve, "Curve"), "setSplineCyclic", json!({ "Cyclic": true }), "Geometry");
            let trim = then(&mut tree, (closed, "Geometry"), "trimCurve", json!({ "Start": 0.85, "End": 0.35 }), "Curve");
            tree.output(trim, "Curve");
            tree
        }),
        case("trimLength", after(line, "trimCurve", json!({ "mode": "length", "Start_001": 0.25, "End_001": 0.6 }))),
        case("trimWrap", after(|tree| tree.add("curveCircle", json!({ "Resolution": 7 })), "trimCurve", json!({ "Start": 0.83, "End": 0.31 }))),
        case("trimBackwards", after(line, "trimCurve", json!({ "Start": 0.7, "End": 0.2 }))),
        case("reverseBezier", after(|tree| bezier(tree, 6), "reverseCurve", json!({}))),
        case("reverseSelected", {
            let mut tree = Tree::new();
            let (a, b) = (line(&mut tree), spiral(&mut tree));
            let both = join(&mut tree, &[(a, "Curve"), (b, "Curve")]);
            let reverse = then(&mut tree, (both, "Geometry"), "reverseCurve", json!({}), "Curve");
            let second = index_is(&mut tree, 1);
            tree.link(second, "Result", reverse, "Selection");
            tree.output(reverse, "Curve");
            tree
        }),
        case("radiusByFactor", {
            let mut tree = Tree::new();
            let curve = spiral(&mut tree);
            let set = then(&mut tree, (curve, "Curve"), "setCurveRadius", json!({}), "Curve");
            let parameter = tree.add("splineParameter", json!({}));
            tree.link(parameter, "Factor", set, "Radius");
            tree.output(set, "Curve");
            tree
        }),
        case("tiltEnds", {
            let mut tree = Tree::new();
            let curve = spiral(&mut tree);
            let set = then(&mut tree, (curve, "Curve"), "setCurveTilt", json!({ "Tilt": 0.5 }), "Curve");
            let ends = tree.add("endpointSelection", json!({ "Start Size": 2 }));
            tree.link(ends, "Selection", set, "Selection");
            tree.output(set, "Curve");
            tree
        }),
        case("cyclicWire", {
            let mut tree = Tree::new();
            let curve = tree.add("curveLine", json!({ "End": [1.0, 0.0, 1.0] }));
            let set = then(&mut tree, (curve, "Curve"), "setSplineCyclic", json!({ "Cyclic": true }), "Geometry");
            let points = then(&mut tree, (set, "Geometry"), "resampleCurve", json!({ "Count": 6 }), "Curve");
            tree.output(points, "Curve");
            tree
        }),
        case("resolutionWire", {
            let mut tree = Tree::new();
            let curve = bezier(&mut tree, 12);
            let set = then(&mut tree, (curve, "Curve"), "setSplineResolution", json!({ "Resolution": 3 }), "Geometry");
            let mesh = then(&mut tree, (set, "Geometry"), "curveToMesh", json!({}), "Curve");
            tree.output(mesh, "Mesh");
            tree
        }),
        case("splineParameter", {
            let mut tree = Tree::new();
            let (a, b, c) = (spiral(&mut tree), bezier(&mut tree, 4), circle5(&mut tree));
            let all = join(&mut tree, &[(a, "Curve"), (b, "Curve"), (c, "Curve")]);
            let parameter = tree.add("splineParameter", json!({}));
            let mut last = (all, "Geometry");
            for (name, socket, data_type, domain) in [
                ("factor", "Factor", "float", "point"),
                ("length", "Length", "float", "point"),
                ("index", "Index", "int", "point"),
                ("curveFactor", "Factor", "float", "curve"),
                ("curveLength", "Length", "float", "curve"),
                ("curveIndex", "Index", "int", "curve"),
            ] {
                last = (store(&mut tree, last, (parameter, socket), name, data_type, domain), "Geometry");
            }
            tree.output(last.0, last.1);
            tree
        }),
        case("curveLength", {
            let mut tree = Tree::new();
            let (a, b) = (bezier(&mut tree, 6), circle5(&mut tree));
            let both = join(&mut tree, &[(a, "Curve"), (b, "Curve")]);
            let length = then(&mut tree, (both, "Geometry"), "curveLength", json!({}), "Curve");
            let stored = store(&mut tree, (both, "Geometry"), (length, "Length"), "length", "float", "curve");
            tree.output(stored, "Geometry");
            tree
        }),
        case("endpoints", {
            let mut tree = Tree::new();
            let (a, b) = (spiral(&mut tree), circle5(&mut tree));
            let both = join(&mut tree, &[(a, "Curve"), (b, "Curve")]);
            let ends = tree.add("endpointSelection", json!({ "Start Size": 2, "End Size": 3 }));
            let stored = store(&mut tree, (both, "Geometry"), (ends, "Selection"), "end", "boolean", "point");
            tree.output(stored, "Geometry");
            tree
        }),
        // Curves through Join Geometry, Transform Geometry, Set Position, and instances.
        case("transformJoin", {
            let mut tree = Tree::new();
            let (a, b) = (line(&mut tree), bezier(&mut tree, 4));
            let moved = then(
                &mut tree,
                (b, "Curve"),
                "transform",
                json!({ "Translation": [1.0, 2.0, 3.0], "Rotation": [0.3, 0.0, 0.5], "Scale": [2.0, 1.0, 1.0] }),
                "Geometry",
            );
            let both = join(&mut tree, &[(moved, "Geometry"), (a, "Curve")]);
            tree.output(both, "Geometry");
            tree
        }),
        case("moveBezier", {
            let mut tree = Tree::new();
            let curve = bezier(&mut tree, 4);
            let set = then(&mut tree, (curve, "Curve"), "setPosition", json!({ "Offset": [0.0, 0.5, 1.0] }), "Geometry");
            tree.output(set, "Geometry");
            tree
        }),
        case("realizeCurves", {
            let mut tree = Tree::new();
            let points = tree.add("meshLine", json!({ "Count": 3, "Offset": [1.0, 0.0, 0.5] }));
            let curve = bezier(&mut tree, 4);
            let instances = then(&mut tree, (points, "Mesh"), "instanceOnPoints", json!({ "Rotation": [0.0, 0.0, 0.4] }), "Points");
            tree.link(curve, "Curve", instances, "Instance");
            let realized = then(&mut tree, (instances, "Instances"), "realizeInstances", json!({}), "Geometry");
            tree.output(realized, "Geometry");
            tree
        }),
    ];
    out
}

fn vec3(value: &Value) -> Vec3 {
    Vec3::new(value[0].as_f64().unwrap() as f32, value[1].as_f64().unwrap() as f32, value[2].as_f64().unwrap() as f32)
}

fn vectors(value: &Value) -> Vec<Vec3> {
    value.as_array().unwrap().iter().map(vec3).collect()
}

fn numbers(value: &Value) -> Vec<f32> {
    value.as_array().unwrap().iter().map(|value| value.as_f64().unwrap() as f32).collect()
}

/// The largest distance between matching points; the lists must be as long.
fn worst_apart(what: &str, ours: &[Vec3], theirs: &[Vec3]) -> f32 {
    assert_eq!(ours.len(), theirs.len(), "{what}: count");
    ours.iter().zip(theirs).map(|(a, b)| a.distance(*b)).fold(0.0, f32::max)
}

fn worst_numbers(what: &str, ours: &[f32], theirs: &[f32]) -> f32 {
    assert_eq!(ours.len(), theirs.len(), "{what}: count");
    ours.iter().zip(theirs).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max)
}

fn handle_code(kind: HandleType) -> i64 {
    match kind {
        HandleType::Free => 0,
        HandleType::Auto => 1,
        HandleType::Vector => 2,
        HandleType::Aligned => 3,
    }
}

/// Compares named attributes; returns the worst number difference.
fn compare_attributes(what: &str, ours: &Attributes, theirs: &Value, rotation: bool) -> f32 {
    let mut worst = 0.0f32;
    for (name, recorded) in theirs.as_object().unwrap() {
        let what = format!("{what} attribute {name}");
        let attribute = ours.get(name).unwrap_or_else(|| panic!("{what}: missing"));
        let domain = if recorded["domain"] == "curve" { Domain::Curve } else { Domain::Point };
        assert!(attribute.domain == domain, "{what}: domain");
        let values = recorded["values"].as_array().unwrap();
        assert_eq!(attribute.values.len(), values.len(), "{what}: count");
        for (mine, theirs) in attribute.values.iter().zip(values) {
            let difference = match (mine, theirs) {
                (Scalar::Vector(mine), Value::Array(_)) if rotation && name == "rotation" => {
                    // Rotations as matrices: Euler angles near ±π may differ by a turn.
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
            };
            worst = worst.max(difference);
        }
    }
    worst
}

fn compare_curves(what: &str, ours: &CurvesComponent, theirs: &Value) -> f32 {
    let offsets: Vec<usize> = theirs["offsets"].as_array().unwrap().iter().map(|value| value.as_u64().unwrap() as usize).collect();
    assert_eq!(ours.offsets, offsets, "{what}: curve offsets");
    let curves = ours.curves_len();
    let types: Vec<i64> = match theirs["types"].as_array() {
        Some(types) => types.iter().map(|value| value.as_i64().unwrap()).collect(),
        None => vec![1; curves],
    };
    let mine: Vec<i64> = ours.kinds.iter().map(|kind| if *kind == CurveType::Bezier { 2 } else { 1 }).collect();
    assert_eq!(mine, types, "{what}: curve types");
    let cyclic: Vec<bool> = match theirs["cyclic"].as_array() {
        Some(cyclic) => cyclic.iter().map(|value| value.as_bool().unwrap()).collect(),
        None => vec![false; curves],
    };
    assert_eq!(ours.cyclic, cyclic, "{what}: cyclic");
    if let Some(resolution) = theirs["resolution"].as_array() {
        for (curve, recorded) in resolution.iter().enumerate() {
            if ours.kinds[curve] == CurveType::Bezier {
                assert_eq!(i64::from(ours.resolution[curve]), recorded.as_i64().unwrap(), "{what}: resolution");
            }
        }
    }
    let mut worst = worst_apart(&format!("{what} positions"), &ours.positions, &vectors(&theirs["positions"]));
    let points = ours.points_len();
    let radius = if theirs["radius"].is_null() { vec![1.0; points] } else { numbers(&theirs["radius"]) };
    worst = worst.max(worst_numbers(&format!("{what} radius"), &ours.radius, &radius));
    let tilt = if theirs["tilt"].is_null() { vec![0.0; points] } else { numbers(&theirs["tilt"]) };
    worst = worst.max(worst_numbers(&format!("{what} tilt"), &ours.tilt, &tilt));
    if !theirs["handle_left"].is_null() {
        let bezier: Vec<bool> = ours.point_curves().iter().map(|curve| ours.kinds[*curve] == CurveType::Bezier).collect();
        let pick = |values: &[Vec3]| -> Vec<Vec3> { values.iter().zip(&bezier).filter(|(_, bezier)| **bezier).map(|(value, _)| *value).collect() };
        let theirs_left = pick(&vectors(&theirs["handle_left"]));
        let theirs_right = pick(&vectors(&theirs["handle_right"]));
        worst = worst.max(worst_apart(&format!("{what} left handles"), &pick(&ours.handle_left), &theirs_left));
        worst = worst.max(worst_apart(&format!("{what} right handles"), &pick(&ours.handle_right), &theirs_right));
        let codes = |values: &[HandleType]| -> Vec<i64> {
            values.iter().zip(&bezier).filter(|(_, bezier)| **bezier).map(|(value, _)| handle_code(*value)).collect()
        };
        let recorded = |key: &str| -> Vec<i64> {
            theirs[key].as_array().unwrap().iter().zip(&bezier).filter(|(_, bezier)| **bezier).map(|(value, _)| value.as_i64().unwrap()).collect()
        };
        assert_eq!(codes(&ours.handle_type_left), recorded("handle_type_left"), "{what}: left handle types");
        assert_eq!(codes(&ours.handle_type_right), recorded("handle_type_right"), "{what}: right handle types");
    }
    let named: Attributes =
        ours.attributes.iter().filter(|(name, _)| !name.starts_with('.')).map(|(name, value)| (name.clone(), value.clone())).collect();
    worst.max(compare_attributes(what, &named, &theirs["attributes"], false))
}

fn area(mesh: &Mesh) -> f32 {
    mesh.faces
        .iter()
        .map(|face| {
            let corner = |at: usize| mesh.positions[face[at] as usize];
            (1..face.len() - 1).map(|at| (corner(at) - corner(0)).cross(corner(at + 1) - corner(0)).z / 2.0).sum::<f32>()
        })
        .sum()
}

fn compare_mesh(what: &str, ours: &Mesh, theirs: &Value, faces: Faces) -> f32 {
    let positions = vectors(&theirs["positions"]);
    let worst = worst_apart(&format!("{what} vertices"), &ours.positions, &positions);
    let recorded: Vec<Vec<u32>> = theirs["faces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|face| face.as_array().unwrap().iter().map(|vertex| vertex.as_u64().unwrap() as u32).collect())
        .collect();
    assert_eq!(ours.faces.len(), recorded.len(), "{what}: faces");
    assert_eq!(ours.edges().len() as u64, theirs["edges"].as_u64().unwrap(), "{what}: edges");
    match faces {
        Faces::Exact => assert_eq!(ours.faces, recorded, "{what}: faces"),
        Faces::Rotated => {
            for (mine, theirs) in ours.faces.iter().zip(&recorded) {
                let start = mine.iter().position(|vertex| *vertex == theirs[0]).unwrap_or_else(|| panic!("{what}: face {mine:?} against {theirs:?}"));
                let turned: Vec<u32> = mine.iter().cycle().skip(start).take(mine.len()).copied().collect();
                assert_eq!(&turned, theirs, "{what}: face");
            }
        }
        Faces::Fill => {
            let reference = Mesh { positions: positions.clone(), faces: recorded, ..Mesh::default() };
            let (mine, theirs) = (area(ours), area(&reference));
            assert!((mine - theirs).abs() < 1e-4 * theirs.abs().max(1.0), "{what}: area {mine} against {theirs}");
            for face in &ours.faces {
                assert!(ours.face_normal(face).z > 0.0, "{what}: face {face:?} faces down");
            }
            // Each triangle is a positive part of the fill: the parts add up to the area.
            let parts: f32 = ours
                .faces
                .iter()
                .map(|face| area(&Mesh { positions: ours.positions.clone(), faces: vec![face.clone()], ..Mesh::default() }).abs())
                .sum();
            assert!((parts - mine).abs() < 1e-4 * mine.abs().max(1.0), "{what}: triangles overlap");
        }
    }
    worst
}

fn compare(case: &Case, ours: &Geometry, theirs: &Value) -> f32 {
    let what = case.name;
    let mut worst = 0.0f32;
    assert_eq!(ours.curves.as_ref().is_some_and(|curves| curves.curves_len() > 0), !theirs["curves"].is_null(), "{what}: curves");
    if let Some(curves) = ours.curves.as_ref().filter(|curves| curves.curves_len() > 0) {
        worst = worst.max(compare_curves(what, curves, &theirs["curves"]));
    }
    let our_mesh = ours.mesh.as_ref().filter(|mesh| !mesh.mesh.positions.is_empty());
    assert_eq!(our_mesh.is_some(), !theirs["mesh"].is_null(), "{what}: mesh");
    if let Some(mesh) = our_mesh {
        worst = worst.max(compare_mesh(what, &mesh.mesh, &theirs["mesh"], case.faces));
    }
    let our_points = ours.points.as_ref().filter(|points| !points.positions.is_empty());
    assert_eq!(our_points.is_some(), !theirs["points"].is_null(), "{what}: points");
    if let Some(points) = our_points {
        let theirs = &theirs["points"];
        worst = worst.max(worst_apart(&format!("{what} points"), &points.positions, &vectors(&theirs["positions"])));
        if !theirs["radius"].is_null() {
            worst = worst.max(worst_numbers(&format!("{what} point radius"), &points.radius, &numbers(&theirs["radius"])));
        }
        let named: Attributes =
            points.attributes.iter().filter(|(name, _)| !name.starts_with('.')).map(|(name, value)| (name.clone(), value.clone())).collect();
        worst = worst.max(compare_attributes(what, &named, &theirs["attributes"], true));
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
fn curve_nodes_match_the_reference() {
    let recorded = fixture();
    let cases = cases();
    let names: Vec<&str> = cases.iter().map(|case| case.name).collect();
    let mut recorded_names: Vec<&str> = recorded["cases"].as_object().unwrap().keys().map(String::as_str).collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    recorded_names.sort_unstable();
    assert_eq!(sorted, recorded_names, "the recorded cases are not this test's: record them again");
    for case in &cases {
        let entry = &recorded["cases"][case.name];
        assert_eq!(entry["group"], serde_json::to_value(&case.tree.group).unwrap(), "{}: the recorded group is not this case's", case.name);
        let ours = case.tree.run_geometry(base()).unwrap_or_else(|error| panic!("{}: {error}", case.name));
        let worst = compare(case, &ours, &entry["result"]);
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
    let dir = std::env::temp_dir().join(format!("kition-scene-curves-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (input, out) = (dir.join("groups.json"), dir.join("out.json"));
    let cases = cases();
    let groups: serde_json::Map<String, Value> =
        cases.iter().map(|case| (case.name.to_owned(), serde_json::to_value(&case.tree.group).unwrap())).collect();
    std::fs::write(&input, Value::Object(groups.clone()).to_string()).unwrap();
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
        groups.into_iter().map(|(name, group)| (name.clone(), json!({ "group": group, "result": fresh["cases"][&name] }))).collect();
    let made = json!({ "blender": fresh["blender"], "cases": entries });
    if std::env::var_os("KITION_BLENDER_RECORD").is_some() {
        std::fs::write(FIXTURE, serde_json::to_string_pretty(&made).unwrap() + "\n").unwrap();
        return;
    }
    assert_eq!(made, fixture(), "the reference no longer gives the recorded results");
}

#[test]
fn rotations_compare_as_matrices() {
    let turned = euler_matrix(Vec3::new(-std::f32::consts::FRAC_PI_2, 0.0, std::f32::consts::PI));
    let other = euler_matrix(Vec3::new(-std::f32::consts::FRAC_PI_2, 0.0, -std::f32::consts::PI));
    assert!((turned - other).to_cols_array().iter().all(|value| value.abs() < 1e-5));
    assert_eq!(Mat3::IDENTITY, euler_matrix(Vec3::ZERO));
}
