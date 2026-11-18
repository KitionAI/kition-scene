//! Node groups against the reference application: each group is rebuilt there as a geometry
//! node tree, both are evaluated, and their counts, bounds, and (for random nodes) statistics
//! are compared. Runs when `KITION_BLENDER` names a Blender executable; otherwise it is skipped.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

mod common;

use common::Tree;
use kition_scene_model::Mesh;
use kition_scene_model::glam::Vec3;
use serde_json::{Value, json};

/// Builds each group in the reference application from our JSON and reports its evaluated mesh.
const SCRIPT: &str = r#"
import bpy, json, sys
args = json.loads(open(sys.argv[sys.argv.index('--') + 1]).read())
KINDS = {
    "groupInput": "NodeGroupInput", "groupOutput": "NodeGroupOutput",
    "meshCube": "GeometryNodeMeshCube", "meshGrid": "GeometryNodeMeshGrid", "meshUvSphere": "GeometryNodeMeshUVSphere",
    "meshIcoSphere": "GeometryNodeMeshIcoSphere", "meshCylinder": "GeometryNodeMeshCylinder", "meshCone": "GeometryNodeMeshCone",
    "meshLine": "GeometryNodeMeshLine", "meshCircle": "GeometryNodeMeshCircle", "transform": "GeometryNodeTransform",
    "joinGeometry": "GeometryNodeJoinGeometry", "setPosition": "GeometryNodeSetPosition", "deleteGeometry": "GeometryNodeDeleteGeometry",
    "mergeByDistance": "GeometryNodeMergeByDistance", "subdivideMesh": "GeometryNodeSubdivideMesh",
    "subdivisionSurface": "GeometryNodeSubdivisionSurface", "extrudeMesh": "GeometryNodeExtrudeMesh",
    "meshToPoints": "GeometryNodeMeshToPoints", "distributePointsOnFaces": "GeometryNodeDistributePointsOnFaces",
    "pointsToVertices": "GeometryNodePointsToVertices", "instanceOnPoints": "GeometryNodeInstanceOnPoints",
    "realizeInstances": "GeometryNodeRealizeInstances", "position": "GeometryNodeInputPosition", "normal": "GeometryNodeInputNormal",
    "index": "GeometryNodeInputIndex", "value": "ShaderNodeValue", "vector": "FunctionNodeInputVector",
    "separateXyz": "ShaderNodeSeparateXYZ", "combineXyz": "ShaderNodeCombineXYZ", "math": "ShaderNodeMath",
    "vectorMath": "ShaderNodeVectorMath", "randomValue": "FunctionNodeRandomValue", "compare": "FunctionNodeCompare",
    "switch": "GeometryNodeSwitch", "mapRange": "ShaderNodeMapRange",
}
PROPS = {"fillType": "fill_type", "mode": "mode", "domain": "domain", "operation": "operation", "useClamp": "use_clamp",
         "dataType": "data_type", "inputType": "input_type", "interpolationType": "interpolation_type", "clamp": "clamp"}
ALIASES = {"A_INT": "A", "B_INT": "B", "A_VEC3": "A", "B_VEC3": "B", "Min_001": "Min", "Max_001": "Max", "Min_002": "Min", "Max_002": "Max"}

def socket(sockets, key):
    for s in sockets:
        if s.identifier == key and s.enabled:
            return s
    key = ALIASES.get(key, key)
    for s in sockets:
        if (s.identifier == key or s.name == key) and s.enabled:
            return s
    raise KeyError(key)

def build(group):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.mesh.primitive_cube_add(size=2)
    ob = bpy.context.object
    tree = bpy.data.node_groups.new("T", "GeometryNodeTree")
    tree.interface.new_socket("Geometry", in_out="OUTPUT", socket_type="NodeSocketGeometry")
    tree.interface.new_socket("Geometry", in_out="INPUT", socket_type="NodeSocketGeometry")
    nodes = {}
    for node in group["nodes"]:
        made = tree.nodes.new(KINDS[node["kind"]])
        values = node.get("values", {})
        for key, value in values.items():
            if key in PROPS:
                prop = PROPS[key]
                if isinstance(value, str):
                    value = value.upper()
                    if node["kind"] == "compare" and key == "dataType" and value == "FLOAT_VECTOR":
                        value = "VECTOR"
                    if key == "fillType" and value == "TRIANGLE_FAN":
                        value = "TRIANGLE_FAN"
                setattr(made, prop, value)
        for key, value in values.items():
            if key in PROPS:
                continue
            if node["kind"] == "value":
                made.outputs[0].default_value = value
            elif node["kind"] == "vector":
                made.vector = value
            else:
                socket(made.inputs, key).default_value = value
        nodes[node["id"]] = made
    for link in group["links"]:
        a, b = nodes[link["from"]], nodes[link["to"]]
        out = a.outputs[0] if link["fromSocket"] == "Geometry" and a.bl_idname == "NodeGroupInput" else socket(a.outputs, link["fromSocket"])
        into = b.inputs[0] if b.bl_idname == "NodeGroupOutput" else socket(b.inputs, link["toSocket"])
        tree.links.new(out, into)
    mod = ob.modifiers.new("GN", "NODES")
    mod.node_group = tree
    m = ob.evaluated_get(bpy.context.evaluated_depsgraph_get()).to_mesh()
    return {"positions": [list(v.co) for v in m.vertices], "faces": len(m.polygons), "edges": len(m.edges)}

json.dump({name: build(group) for name, group in args["groups"].items()}, open(args["out"], "w"))
"#;

struct Reference {
    positions: Vec<Vec3>,
    faces: usize,
}

fn reference(groups: &[(&str, &Tree)]) -> Option<Vec<Reference>> {
    let executable = std::env::var_os("KITION_BLENDER")?;
    let dir = std::env::temp_dir().join(format!("kition-scene-nodes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (script, input, out) = (dir.join("nodes.py"), dir.join("groups.json"), dir.join("out.json"));
    std::fs::write(&script, SCRIPT).unwrap();
    let map: serde_json::Map<String, Value> =
        groups.iter().map(|(name, tree)| ((*name).to_owned(), serde_json::to_value(&tree.group).unwrap())).collect();
    std::fs::write(&input, json!({ "groups": map, "out": out.to_str().unwrap() }).to_string()).unwrap();
    let status = std::process::Command::new(executable)
        .args(["-b", "--factory-startup", "--python"])
        .arg(&script)
        .arg("--")
        .arg(&input)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "Blender failed");
    let results: Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    Some(
        groups
            .iter()
            .map(|(name, _)| {
                let result = &results[*name];
                let positions = result["positions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|point| Vec3::new(point[0].as_f64().unwrap() as f32, point[1].as_f64().unwrap() as f32, point[2].as_f64().unwrap() as f32))
                    .collect();
                Reference { positions, faces: result["faces"].as_u64().unwrap() as usize }
            })
            .collect(),
    )
}

fn base() -> Mesh {
    kition_scene_model::Primitive::Cube { size: 2.0 }.build().unwrap()
}

fn single(kind: &str, values: Value) -> Tree {
    let mut tree = Tree::new();
    let node = tree.add(kind, values);
    tree.output(node, "Mesh");
    tree
}

fn bounds(points: &[Vec3]) -> (Vec3, Vec3) {
    points.iter().fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |(low, high), point| (low.min(*point), high.max(*point)))
}

fn scatter(size: f32) -> Tree {
    let mut tree = Tree::new();
    let grid = tree.add("meshGrid", json!({ "Size X": size, "Size Y": size }));
    let scatter = tree.add("distributePointsOnFaces", json!({ "Density": 10.0, "Seed": 0 }));
    let cube = tree.add("meshCube", json!({ "Size": [0.1, 0.1, 0.1] }));
    let instance = tree.add("instanceOnPoints", json!({}));
    let realize = tree.add("realizeInstances", json!({}));
    tree.link(grid, "Mesh", scatter, "Mesh");
    tree.link(scatter, "Points", instance, "Points");
    tree.link(cube, "Mesh", instance, "Instance");
    tree.link(instance, "Instances", realize, "Geometry");
    tree.output(realize, "Geometry");
    tree
}

fn noise() -> Tree {
    let mut tree = Tree::new();
    let cube = tree.add("meshCube", json!({}));
    let subdivide = tree.add("subdivideMesh", json!({ "Level": 2 }));
    let random = tree.add("randomValue", json!({ "dataType": "float_vector", "Min_001": [-0.1, -0.1, -0.1], "Max_001": [0.1, 0.1, 0.1] }));
    let set = tree.add("setPosition", json!({}));
    tree.link(cube, "Mesh", subdivide, "Mesh");
    tree.link(subdivide, "Mesh", set, "Geometry");
    tree.link(random, "Value", set, "Offset");
    tree.output(set, "Geometry");
    tree
}

fn extrude(values: Value) -> Tree {
    let mut tree = Tree::new();
    let cube = tree.add("meshCube", json!({}));
    let extrude = tree.add("extrudeMesh", values);
    tree.link(cube, "Mesh", extrude, "Mesh");
    tree.output(extrude, "Mesh");
    tree
}

/// The standard deviation of each vertex's offset from the quarter lattice of a twice
/// subdivided unit cube.
fn lattice_spread(points: &[Vec3]) -> f32 {
    let offsets: Vec<f32> = points.iter().flat_map(|point| (*point - (*point * 4.0).round() / 4.0).to_array()).collect();
    let mean = offsets.iter().sum::<f32>() / offsets.len() as f32;
    (offsets.iter().map(|offset| (offset - mean).powi(2)).sum::<f32>() / offsets.len() as f32).sqrt()
}

#[test]
fn node_groups_match_the_reference() {
    let mut cases: Vec<(&str, Tree)> = vec![
        ("cube", single("meshCube", json!({}))),
        ("cube343", single("meshCube", json!({ "Vertices X": 3, "Vertices Y": 4, "Vertices Z": 3 }))),
        ("grid", single("meshGrid", json!({ "Vertices X": 5, "Vertices Y": 4, "Size X": 2.0 }))),
        ("uvSphere", single("meshUvSphere", json!({}))),
        ("icoSphere", single("meshIcoSphere", json!({ "Subdivisions": 3 }))),
        ("cylinder", single("meshCylinder", json!({ "Side Segments": 3, "Fill Segments": 2 }))),
        ("cylinderFan", single("meshCylinder", json!({ "Fill Segments": 2, "fillType": "triangle_fan" }))),
        ("cone", single("meshCone", json!({}))),
        ("coneTruncated", single("meshCone", json!({ "Radius Top": 0.5, "Side Segments": 2, "Fill Segments": 3 }))),
        ("line", single("meshLine", json!({ "mode": "end_points", "Count": 5 }))),
        ("circle", single("meshCircle", json!({ "fillType": "triangle_fan" }))),
        ("extrudeRegion", extrude(json!({ "Offset Scale": 0.5, "Individual": false }))),
        ("extrudeIndividual", extrude(json!({ "Offset Scale": 0.5 }))),
        ("extrudeVertices", extrude(json!({ "mode": "vertices" }))),
        ("subdivisionSurface", single("subdivisionSurface", json!({ "Level": 2 }))),
    ];
    // Subdivision Surface needs a mesh: the cube.
    if let Some((_, tree)) = cases.iter_mut().find(|(name, _)| *name == "subdivisionSurface") {
        let node = tree.group.nodes.last().unwrap().id;
        let cube = tree.add("meshCube", json!({}));
        tree.link(cube, "Mesh", node, "Mesh");
    }
    let scatter = scatter(4.0);
    let noise = noise();
    let mut all: Vec<(&str, &Tree)> = cases.iter().map(|(name, tree)| (*name, tree)).collect();
    all.push(("scatter", &scatter));
    all.push(("noise", &noise));
    let Some(results) = reference(&all) else {
        eprintln!("skipped: set KITION_BLENDER to a Blender executable");
        return;
    };
    for ((name, tree), theirs) in all.iter().zip(&results) {
        let ours = tree.run(base()).unwrap();
        let (our_bounds, their_bounds) = (bounds(&ours.positions), bounds(&theirs.positions));
        match *name {
            "scatter" => {
                // Different random streams: the counts agree within the noise of a Poisson
                // process with the same mean, and both fill the same square.
                let (mine, reference) = (ours.positions.len() as f32 / 8.0, theirs.positions.len() as f32 / 8.0);
                eprintln!("scatter: {mine} points (reference {reference})");
                assert!((mine - reference).abs() < 3.0 * reference.sqrt() + 1.0, "scatter: {mine} vs {reference}");
                assert!(our_bounds.0.abs_diff_eq(their_bounds.0, 0.3) && our_bounds.1.abs_diff_eq(their_bounds.1, 0.3), "scatter bounds");
            }
            "noise" => {
                assert_eq!((ours.positions.len(), ours.faces.len()), (theirs.positions.len(), theirs.faces));
                let (mine, reference) = (lattice_spread(&ours.positions), lattice_spread(&theirs.positions));
                eprintln!("noise: offset spread {mine:.4} (reference {reference:.4})");
                assert!((mine - reference).abs() < 0.15 * reference, "noise: {mine} vs {reference}");
            }
            _ => {
                assert_eq!((ours.positions.len(), ours.faces.len()), (theirs.positions.len(), theirs.faces), "{name}");
                assert!(
                    our_bounds.0.abs_diff_eq(their_bounds.0, 1e-3) && our_bounds.1.abs_diff_eq(their_bounds.1, 1e-3),
                    "{name}: {our_bounds:?} vs {their_bounds:?}"
                );
            }
        }
    }
}
