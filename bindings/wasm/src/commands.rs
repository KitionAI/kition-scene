//! Every command: the catalogue (titles, menus, shortcuts, parameter schemas, when each can
//! run) and what each does. Commands that change the scene run on the document and become one
//! undo step each; a failing command leaves everything as it was. Rotations are in degrees.

use std::collections::BTreeSet;

use glam::{Mat3, Mat4, Vec3};
use kition_kit::KitCommand;
use kition_scene_model::scene::{euler_matrix, matrix_euler};
use kition_scene_model::{Camera, Id, Light, LightKind, Modifier, ModifierKind, Object, ObjectData, Primitive, Scene, ops};
use kition_scene_raster::{Axis, Shading};
use serde_json::{Value, json};

use crate::params::{self, bad, schema as s};
use crate::{Mode, SceneDocument, SelectMode};

/// When a command can run.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Needs {
    Always,
    /// Object mode with at least one selected object.
    Selection,
    /// Object mode.
    ObjectMode,
    /// Edit mode.
    EditMode,
    /// Edit mode with at least one selected vertex.
    EditSelection,
    /// A selected (or active) mesh object, or edit mode.
    Mesh,
    Undo,
    Redo,
    /// The scene has a camera.
    Camera,
    Rendered,
}

pub(crate) struct Spec {
    id: &'static str,
    title: &'static str,
    menu: &'static [&'static str],
    shortcut: Option<&'static str>,
    mutates: bool,
    needs: Needs,
    schema: fn() -> Value,
}

pub(crate) const fn spec(
    id: &'static str,
    title: &'static str,
    menu: &'static [&'static str],
    shortcut: Option<&'static str>,
    mutates: bool,
    needs: Needs,
    schema: fn() -> Value,
) -> Spec {
    Spec { id, title, menu, shortcut, mutates, needs, schema }
}

fn none() -> Value {
    s::of(vec![])
}

fn targets() -> Value {
    s::of(vec![("objects", s::objects("The objects; the selection when left out"))])
}

fn placement() -> Vec<(&'static str, Value)> {
    vec![
        ("location", s::vec3("Where to put it, in metres (the origin when left out)")),
        ("rotation", s::vec3("Rotation in degrees, XYZ Euler")),
        ("scale", s::vec3("Scale")),
        ("name", s::string("The object's name")),
    ]
}

fn with_placement(mut fields: Vec<(&'static str, Value)>) -> Value {
    fields.extend(placement());
    s::of(fields)
}

fn modifier_ref() -> Vec<(&'static str, Value)> {
    vec![
        ("object", s::object("The mesh object; the active one when left out")),
        ("index", s::integer("The modifier's position in the stack, from 0")),
        ("name", s::string("The modifier's name (instead of index)")),
    ]
}

fn modifier_settings() -> Vec<(&'static str, Value)> {
    vec![
        ("axes", json!({ "type": "array", "items": { "type": "boolean" }, "description": "mirror: [x, y, z] axes to mirror across" })),
        ("merge", s::boolean("mirror and array: join vertices where copies meet")),
        ("mergeThreshold", s::number("mirror: the distance that joins vertices on the plane")),
        ("count", s::integer("array: the number of copies")),
        ("relativeOffset", s::vec3("array: the offset between copies as a fraction of the mesh size")),
        ("constantOffset", s::vec3("array: an extra offset in metres")),
        ("levels", s::integer("subdivision: levels, 0 to 6; multires: the level shown (at most the mesh's levels)")),
        ("sculptLevels", s::integer("multires: the level sculpt mode shows and sculpts")),
        ("thickness", s::number("solidify: the shell's thickness in metres")),
        ("offset", s::number("solidify: -1 grows inwards, 1 outwards; wireframe: -1 to 1; shrinkwrap: metres kept from the surface")),
        ("rim", s::boolean("solidify: close the open edges")),
        ("distance", s::number("weld: the distance that joins vertices")),
        ("factor", s::number("smooth: how far each vertex moves towards its neighbours")),
        ("iterations", s::integer("smooth: repeats")),
        ("width", s::number("bevel: the bevel's width in metres")),
        ("segments", s::integer("bevel: segments across each bevel (1 is a flat chamfer)")),
        ("angleLimit", s::number("bevel: edges sharper than this many degrees are bevelled; decimate planar: faces flatter than this join (5)")),
        ("operation", s::choice("boolean: what to keep", &["difference", "union", "intersect"])),
        ("operand", s::object("boolean and shrinkwrap: the other mesh object; lattice: the lattice object; armature: the armature")),
        ("steps", s::integer("screw: steps around one turn")),
        ("angle", s::number("screw: degrees to turn")),
        ("screwOffset", s::number("screw: how far along the axis one turn moves, metres")),
        ("axis", s::choice("screw and simpleDeform: the local axis", &["x", "y", "z"])),
        ("strength", s::number("displace: how far, in metres, at full value; lattice: how much of the bend (1)")),
        ("midLevel", s::number("displace: the value that does not move (0.5)")),
        ("direction", s::choice("displace: where to push", &["normal", "x", "y", "z"])),
        ("noiseScale", s::number("displace: noise feature size in metres; 0 pushes every vertex alike")),
        ("image", s::string("displace: an image's hash (from image.add) whose brightness pushes; null for none")),
        (
            "coordinates",
            s::choice("displace: where the image is looked up (local: -1 to 1 in object axes covers it once)", &["local", "global", "uv"]),
        ),
        ("ratio", s::number("decimate collapse: the share of triangles to keep, 0 to 1")),
        ("iterations", s::integer("decimate unsubdivide: half steps of subdivision to undo, 0 to 16")),
        ("wrapMethod", s::choice("shrinkwrap: where vertices go", &["nearestSurface", "project", "nearestVertex"])),
        ("positive", s::boolean("shrinkwrap project: cast along the vertex normal (true)")),
        ("negative", s::boolean("shrinkwrap project: cast against it (false)")),
        (
            "mode",
            s::choice(
                "simpleDeform: what it does; decimate: collapse, unsubdivide, or planar",
                &["twist", "bend", "taper", "stretch", "collapse", "unsubdivide", "planar"],
            ),
        ),
        ("preserveVolume", s::boolean("armature: blend the bones as dual quaternions, so joints keep their volume (false)")),
        ("vertexGroups", s::boolean("armature: vertices follow the bones of their groups (true)")),
        ("envelopes", s::boolean("armature: vertices in no bone's group follow the bones whose envelopes reach them (false)")),
        ("vertexGroup", s::string("armature: a group whose weights say how much of the deformation each vertex takes; null for all")),
        ("invertVertexGroup", s::boolean("armature: use one minus that group's weights")),
        ("multiModifier", s::boolean("armature: deform what the Armature modifier above got and mix over its result by the group")),
        ("group", s::integer("geometryNodes: the node group's id (node.newGroup makes one)")),
        ("inputs", json!({ "type": "object", "additionalProperties": true, "description": "geometryNodes: the group's inputs by name" })),
        (
            "limits",
            json!({ "type": "array", "items": { "type": "number" }, "description": "simpleDeform: [low, high], the share of the range that deforms, 0 to 1" }),
        ),
    ]
}

const ADD_MESH: &[&str] = &["Add", "Mesh"];

static SPECS: &[Spec] = &[
    spec("edit.undo", "Undo", &["Edit"], Some("Cmd+Z"), true, Needs::Undo, || {
        s::of(vec![("scope", s::choice("Whose steps to undo", &["person", "agent"]))])
    }),
    spec("edit.redo", "Redo", &["Edit"], Some("Cmd+Shift+Z"), true, Needs::Redo, || {
        s::of(vec![("scope", s::choice("Whose steps to redo", &["person", "agent"]))])
    }),
    spec("edit.delete", "Delete", &["Edit"], Some("X"), true, Needs::Always, || {
        s::of(vec![(
            "objects",
            s::objects("Object mode: the objects to delete; the selection when left out. Edit mode deletes the selected faces or vertices"),
        )])
    }),
    spec("edit.duplicate", "Duplicate", &["Edit"], Some("Shift+D"), true, Needs::Always, || {
        s::of(vec![
            ("objects", s::objects("In object mode, the objects to copy; the selected ones when left out")),
            ("offset", s::vec3("Move the copies by this much, in metres")),
        ])
    }),
    spec("edit.selectAll", "Select All", &["Edit", "Select"], Some("A"), false, Needs::Always, || {
        s::of(vec![("action", s::choice("What to do", &["select", "deselect", "invert"]))])
    }),
    spec("edit.deselectAll", "Select None", &["Edit", "Select"], Some("Alt+A"), false, Needs::Always, none),
    spec("select.objects", "Select Objects", &[], None, false, Needs::Always, || {
        s::of(vec![
            ("objects", s::required(s::objects("The objects to select"))),
            ("extend", s::boolean("Add to the selection instead of replacing it")),
            ("deselect", s::boolean("Take them out of the selection")),
        ])
    }),
    // Add.
    spec("object.addPlane", "Plane", ADD_MESH, None, true, Needs::Always, || with_placement(vec![("size", s::number("Side length in metres (2)"))])),
    spec("object.addBackdrop", "Backdrop", ADD_MESH, None, true, Needs::ObjectMode, || {
        with_placement(vec![
            ("name", s::string("The new object's name (Backdrop)")),
            ("width", s::number("Width across X in metres (20)")),
            ("depth", s::number("Depth of the floor in front of the curve in metres, centred on the origin (10)")),
            ("height", s::number("Height of the wall in metres (8)")),
            ("radius", s::number("Radius of the curve from floor to wall in metres (3)")),
            ("segments", s::integer("Faces round the curve, 1 to 256 (16)")),
        ])
    }),
    spec("object.addCube", "Cube", ADD_MESH, None, true, Needs::Always, || with_placement(vec![("size", s::number("Side length in metres (2)"))])),
    spec("object.addCircle", "Circle", ADD_MESH, None, true, Needs::Always, || {
        with_placement(vec![
            ("vertices", s::integer("Vertices (32)")),
            ("radius", s::number("Radius in metres (1)")),
            ("fill", s::choice("How to fill it", &["nothing", "ngon", "triangleFan"])),
        ])
    }),
    spec("object.addUvSphere", "UV Sphere", ADD_MESH, None, true, Needs::Always, || {
        with_placement(vec![
            ("segments", s::integer("Segments around (32)")),
            ("rings", s::integer("Rings from pole to pole (16)")),
            ("radius", s::number("Radius in metres (1)")),
        ])
    }),
    spec("object.addIcoSphere", "Ico Sphere", ADD_MESH, None, true, Needs::Always, || {
        with_placement(vec![("subdivisions", s::integer("Subdivisions, 1 to 8 (2)")), ("radius", s::number("Radius in metres (1)"))])
    }),
    spec("object.addCylinder", "Cylinder", ADD_MESH, None, true, Needs::Always, || {
        with_placement(vec![
            ("vertices", s::integer("Vertices around (32)")),
            ("radius", s::number("Radius in metres (1)")),
            ("depth", s::number("Height in metres (2)")),
            ("fill", s::choice("How the ends are closed", &["nothing", "ngon", "triangleFan"])),
        ])
    }),
    spec("object.addCone", "Cone", ADD_MESH, None, true, Needs::Always, || {
        with_placement(vec![
            ("vertices", s::integer("Vertices around (32)")),
            ("radius1", s::number("Bottom radius in metres (1)")),
            ("radius2", s::number("Top radius in metres (0)")),
            ("depth", s::number("Height in metres (2)")),
            ("fill", s::choice("How the ends are closed", &["nothing", "ngon", "triangleFan"])),
        ])
    }),
    spec("object.addTorus", "Torus", ADD_MESH, None, true, Needs::Always, || {
        with_placement(vec![
            ("majorSegments", s::integer("Segments around the ring (48)")),
            ("minorSegments", s::integer("Segments around the tube (12)")),
            ("majorRadius", s::number("Ring radius in metres (1)")),
            ("minorRadius", s::number("Tube radius in metres (0.25)")),
        ])
    }),
    spec("object.addGrid", "Grid", ADD_MESH, None, true, Needs::Always, || {
        with_placement(vec![
            ("xSubdivisions", s::integer("Faces along X (10)")),
            ("ySubdivisions", s::integer("Faces along Y (10)")),
            ("size", s::number("Side length in metres (2)")),
        ])
    }),
    spec("object.addLight", "Light", &["Add"], None, true, Needs::ObjectMode, || {
        with_placement(vec![
            ("kind", s::choice("The kind of light", &["point", "sun", "spot", "area"])),
            ("power", s::number("Watts (W/m² for a sun)")),
            ("color", s::color("Colour")),
        ])
    }),
    spec("object.addCamera", "Camera", &["Add"], None, true, Needs::ObjectMode, || {
        with_placement(vec![("lens", s::number("Focal length in millimetres (50)"))])
    }),
    spec("object.addEmpty", "Empty", &["Add"], None, true, Needs::ObjectMode, || {
        with_placement(vec![("size", s::number("Axis length in metres (1)"))])
    }),
    // Object.
    spec("object.transform", "Set Transform", &["Object", "Transform"], None, true, Needs::Always, || {
        s::of(vec![
            ("objects", s::objects("The objects; the selection when left out")),
            ("location", s::vec3("Location in metres")),
            ("rotation", s::vec3("Rotation in degrees, XYZ Euler")),
            ("scale", s::vec3("Scale")),
        ])
    }),
    spec("object.previewTransform", "Preview Transform", &[], None, false, Needs::Always, || {
        s::of(vec![
            ("objects", s::objects("The objects; the selection when left out")),
            ("location", s::vec3("Location in metres")),
            ("rotation", s::vec3("Rotation in degrees, XYZ Euler")),
            ("scale", s::vec3("Scale")),
            ("end", s::boolean("End the preview, putting the objects back as they were")),
        ])
    }),
    spec("object.translate", "Move", &["Object", "Transform"], None, true, Needs::Selection, || {
        s::of(vec![("objects", s::objects("The objects; the selection when left out")), ("offset", s::required(s::vec3("How far, in metres")))])
    }),
    spec("object.rotate", "Rotate", &["Object", "Transform"], None, true, Needs::Selection, || {
        s::of(vec![
            ("objects", s::objects("The objects; the selection when left out")),
            ("axis", s::required(s::choice("The world axis to turn about", &["x", "y", "z"]))),
            ("angle", s::required(s::number("Degrees, counter-clockwise looking down the axis"))),
        ])
    }),
    spec("object.scale", "Scale", &["Object", "Transform"], None, true, Needs::Selection, || {
        s::of(vec![
            ("objects", s::objects("The objects; the selection when left out")),
            ("factor", s::required(json!({ "type": "json", "description": "A number, or [x, y, z]" }))),
        ])
    }),
    spec("object.applyTransform", "Apply Transform", &["Object", "Apply"], Some("Cmd+A"), true, Needs::Selection, || {
        s::of(vec![
            ("objects", s::objects("The objects; the selection when left out")),
            ("location", s::boolean("Apply the location (false)")),
            ("rotation", s::boolean("Apply the rotation (true)")),
            ("scale", s::boolean("Apply the scale (true)")),
        ])
    }),
    spec("object.originToGeometry", "Origin to Geometry", &["Object", "Set Origin"], None, true, Needs::Selection, targets),
    spec("object.order", "Move in Outliner", &[], None, true, Needs::Always, || {
        s::of(vec![
            ("object", s::required(s::object("The object to move"))),
            ("index", s::required(s::integer("Its new place in the outliner, from 0"))),
        ])
    }),
    spec("object.rename", "Rename", &["Object"], Some("F2"), true, Needs::Always, || {
        s::of(vec![("object", s::object("The object; the active one when left out")), ("name", s::required(s::string("The new name")))])
    }),
    spec("object.parent", "Parent", &["Object", "Parent"], None, true, Needs::Selection, || {
        s::of(vec![
            ("objects", s::objects("The children; the selection without the active object when left out")),
            ("parent", s::object("The parent; the active object when left out")),
        ])
    }),
    spec("object.clearParent", "Clear Parent", &["Object", "Parent"], Some("Alt+P"), true, Needs::Selection, targets),
    spec("object.hide", "Hide", &["Object", "Show/Hide"], Some("H"), true, Needs::Selection, targets),
    spec("object.reveal", "Show Hidden", &["Object", "Show/Hide"], Some("Alt+H"), true, Needs::ObjectMode, || {
        s::of(vec![("objects", s::objects("The hidden objects to show; every hidden object when left out"))])
    }),
    spec("object.shadeSmooth", "Shade Smooth", &["Object"], None, true, Needs::Mesh, targets),
    spec("object.shadeSmoothByAngle", "Shade Smooth by Angle", &["Object"], None, true, Needs::Mesh, || {
        s::of(vec![
            ("objects", s::objects("The objects; the selection when left out")),
            ("angle", s::number("Edges whose faces meet at more than this many degrees stay sharp, 0 to 180 (30)")),
        ])
    }),
    spec("object.shadeFlat", "Shade Flat", &["Object"], None, true, Needs::Mesh, targets),
    spec("object.join", "Join", &["Object"], Some("Cmd+J"), true, Needs::Selection, none),
    spec("object.setCamera", "Set Active Camera", &["Object"], None, true, Needs::Always, || {
        s::of(vec![("object", s::object("The camera; the active object when left out"))])
    }),
    spec("object.lookAt", "Look At", &["Object", "Transform"], None, true, Needs::Always, || {
        s::of(vec![
            ("object", s::object("The object to turn (a camera or a light); the active one when left out")),
            ("target", s::required(s::vec3("The point to look at, in metres"))),
        ])
    }),
    // Edit mode.
    spec("mode.toggle", "Toggle Edit Mode", &["Object"], Some("Tab"), false, Needs::Mesh, none),
    spec("mode.set", "Set Mode", &[], None, false, Needs::Always, || {
        s::of(vec![
            ("mode", s::required(s::choice("The mode", &["object", "edit"]))),
            ("object", s::object("The mesh to edit; the active one when left out")),
        ])
    }),
    spec("mesh.selectMode", "Select Mode", &[], None, false, Needs::EditMode, || {
        s::of(vec![("mode", s::required(s::choice("What clicks select", &["vertex", "edge", "face"])))])
    }),
    spec("mesh.selectVertexMode", "Vertex Select", &["Mesh", "Select Mode"], Some("1"), false, Needs::EditMode, none),
    spec("mesh.selectEdgeMode", "Edge Select", &["Mesh", "Select Mode"], Some("2"), false, Needs::EditMode, none),
    spec("mesh.selectFaceMode", "Face Select", &["Mesh", "Select Mode"], Some("3"), false, Needs::EditMode, none),
    spec("mesh.select", "Select Elements", &[], None, false, Needs::EditMode, || {
        s::of(vec![
            ("vertices", s::indices("Vertex indices")),
            ("faces", s::indices("Face indices (selects their vertices)")),
            ("extend", s::boolean("Add to the selection instead of replacing it")),
            ("deselect", s::boolean("Take them out of the selection")),
        ])
    }),
    spec("mesh.extrude", "Extrude Faces", &["Mesh"], Some("E"), true, Needs::EditSelection, || {
        s::of(vec![("distance", s::number("How far along the faces' mean normal, in metres (1)")), ("offset", s::vec3("An extra offset in metres"))])
    }),
    spec("mesh.inset", "Inset Faces", &["Mesh"], Some("I"), true, Needs::EditSelection, || {
        s::of(vec![
            ("thickness", s::number("How far in, in metres (0.1)")),
            ("depth", s::number("How far the inset faces then move along their normals, in metres; negative is inwards (0)")),
        ])
    }),
    spec("mesh.bevel", "Bevel Edges", &["Mesh"], Some("Cmd+B"), true, Needs::EditSelection, || {
        s::of(vec![("width", s::number("The bevel's width in metres (0.1)")), ("segments", s::integer("Segments across the bevel (1)"))])
    }),
    spec("mesh.subdivide", "Subdivide", &["Mesh"], None, true, Needs::EditSelection, none),
    spec("mesh.loopCut", "Loop Cut", &["Mesh"], None, true, Needs::EditMode, || {
        s::of(vec![
            ("edge", s::required(s::indices("The edge the cut crosses, as [vertex, vertex]"))),
            ("cuts", s::integer("Number of loops (1)")),
            ("factor", s::number("Where a single loop goes between the sides, 0 to 1 (0.5)")),
        ])
    }),
    spec("mesh.merge", "Merge at Center", &["Mesh"], Some("M"), true, Needs::EditSelection, none),
    spec("mesh.deleteFaces", "Delete Faces", &["Mesh", "Delete"], None, true, Needs::EditSelection, none),
    spec("mesh.deleteVertices", "Delete Vertices", &["Mesh", "Delete"], None, true, Needs::EditSelection, none),
    spec("mesh.fill", "Fill Face", &["Mesh"], Some("F"), true, Needs::EditSelection, || {
        s::of(vec![("vertices", s::indices("The face's corners in order; the selection when left out"))])
    }),
    spec("mesh.flip", "Flip Normals", &["Mesh", "Normals"], None, true, Needs::EditSelection, none),
    spec("mesh.recalculateNormals", "Recalculate Outside", &["Mesh", "Normals"], Some("Shift+N"), true, Needs::Mesh, || {
        s::of(vec![(
            "objects",
            s::objects("Object mode: the mesh objects to turn outward; the selection when left out. Edit mode works on the edited mesh"),
        )])
    }),
    spec("mesh.translate", "Move Elements", &["Mesh", "Transform"], None, true, Needs::EditSelection, || {
        s::of(vec![("offset", s::required(s::vec3("How far in the object's own axes, in metres")))])
    }),
    spec("mesh.scale", "Scale Elements", &["Mesh", "Transform"], None, true, Needs::EditSelection, || {
        s::of(vec![
            ("factor", s::required(json!({ "type": "json", "description": "A number, or [x, y, z]" }))),
            ("pivot", s::vec3("The point to scale about; the selection's centre when left out")),
        ])
    }),
    spec("mesh.setGeometry", "Set Geometry", &[], None, true, Needs::Always, || {
        s::of(vec![
            ("object", s::object("The mesh object; the active one when left out")),
            (
                "positions",
                s::required(
                    json!({ "type": "array", "items": { "type": "json" }, "description": "Vertex positions as [x, y, z], in the object's own axes" }),
                ),
            ),
            (
                "faces",
                s::required(
                    json!({ "type": "array", "items": { "type": "json" }, "description": "Faces as lists of vertex indices, counter-clockwise seen from outside" }),
                ),
            ),
            ("smooth", s::boolean("Smooth shading")),
        ])
    }),
    // Modifiers.
    spec("modifier.add", "Add Modifier", &["Modifier"], None, true, Needs::Mesh, || {
        let mut fields = vec![
            ("object", s::object("The mesh object; the active one when left out")),
            (
                "type",
                s::required(s::choice(
                    "The modifier",
                    &[
                        "mirror",
                        "array",
                        "subdivision",
                        "solidify",
                        "triangulate",
                        "weld",
                        "smooth",
                        "bevel",
                        "boolean",
                        "screw",
                        "displace",
                        "decimate",
                        "wireframe",
                        "simpleDeform",
                        "cast",
                        "multires",
                        "shrinkwrap",
                        "lattice",
                        "geometryNodes",
                    ],
                )),
            ),
        ];
        fields.extend(modifier_settings());
        s::of(fields)
    }),
    spec("modifier.set", "Change Modifier", &[], None, true, Needs::Always, || {
        let mut fields = modifier_ref();
        fields.push(("enabled", s::boolean("Shown in the viewport and renders")));
        fields.push(("rename", s::string("A new name")));
        fields.extend(modifier_settings());
        s::of(fields)
    }),
    spec("modifier.remove", "Remove Modifier", &[], None, true, Needs::Always, || s::of(modifier_ref())),
    spec("modifier.apply", "Apply Modifier", &[], None, true, Needs::Always, || s::of(modifier_ref())),
    spec("modifier.move", "Move Modifier", &[], None, true, Needs::Always, || {
        let mut fields = modifier_ref();
        fields.push(("to", s::required(s::integer("The new position, from 0"))));
        s::of(fields)
    }),
    // Materials.
    spec("material.new", "New Material", &["Material"], None, true, Needs::Always, || {
        s::of(vec![
            ("name", s::string("The material's name")),
            ("baseColor", s::color("Base colour")),
            ("metallic", s::number("0 to 1")),
            ("roughness", s::number("0 to 1")),
            ("assign", s::boolean("Give it to the selected objects: true unless false")),
        ])
    }),
    spec("material.set", "Change Material", &[], None, true, Needs::Always, || {
        s::of(vec![
            ("material", s::required(json!({ "oneOf": [{ "type": "integer" }, { "type": "string" }], "description": "The material's id or name" }))),
            ("name", s::string("A new name")),
            ("baseColor", s::color("Base colour")),
            ("metallic", s::number("0 to 1")),
            ("roughness", s::number("0 to 1")),
            ("emission", s::color("Emission colour")),
            ("emissionStrength", s::number("Emission strength")),
            ("normalStrength", s::number("How strongly the normal map bends the surface (1)")),
            ("transmission", s::number("How much light passes through, 0 to 1 (glass at 1)")),
            ("ior", s::number("Index of refraction (1.5)")),
            ("alphaMode", s::choice("How the base colour's alpha is used", &["opaque", "clip", "blend"])),
            ("alphaCutoff", s::number("clip: the alpha below which the surface is not there (0.5)")),
            (
                "shader",
                json!({ "oneOf": [{ "type": "integer" }, { "type": "string" }, { "type": "null" }], "description": "A shader group (id or name) giving the surface and volume in path-traced renders, or null to let go of it" }),
            ),
        ])
    }),
    spec("material.assign", "Assign Material", &[], None, true, Needs::Always, || {
        s::of(vec![
            ("material", s::required(json!({ "oneOf": [{ "type": "integer" }, { "type": "string" }], "description": "The material's id or name" }))),
            ("objects", s::objects("The objects; the selection when left out")),
        ])
    }),
    spec("material.clear", "Remove Material", &["Material"], None, true, Needs::Selection, targets),
    spec("material.delete", "Delete Material", &[], None, true, Needs::Always, || {
        s::of(vec![
            ("material", s::required(json!({ "oneOf": [{ "type": "integer" }, { "type": "string" }], "description": "The material's id or name" }))),
            ("force", s::boolean("Delete it even though objects use it; they are left without a material (false)")),
        ])
    }),
    // Lights, cameras, world, render.
    spec("light.set", "Light Settings", &[], None, true, Needs::Always, || {
        s::of(vec![
            ("object", s::object("The light; the active object when left out")),
            ("kind", s::choice("The kind of light", &["point", "sun", "spot", "area"])),
            ("color", s::color("Colour")),
            ("power", s::number("Watts (W/m² for a sun)")),
            ("radius", s::number("Soft shadow radius in metres")),
            ("spotSize", s::number("Spot cone angle in degrees")),
        ])
    }),
    spec("camera.set", "Camera Settings", &[], None, true, Needs::Always, || {
        s::of(vec![
            ("object", s::object("The camera; the active object when left out")),
            ("lens", s::number("Focal length in millimetres")),
            ("sensor", s::number("Sensor width in millimetres, used by Auto and Horizontal")),
            ("sensorHeight", s::number("Sensor height in millimetres, 1 to 200; used by Vertical")),
            (
                "sensorFit",
                s::choice(
                    "Frame dimension held by the sensor or orthographic scale; Auto holds the longer dimension",
                    &["auto", "horizontal", "vertical"],
                ),
            ),
            ("clipStart", s::number("Nearest distance drawn, metres")),
            ("clipEnd", s::number("Farthest distance drawn, metres")),
            ("orthographic", s::boolean("Orthographic instead of perspective")),
            ("orthoScale", s::number("Span along the fitted frame dimension, metres")),
            ("depthOfField", s::boolean("Blur what is out of focus in path-traced renders")),
            ("fStop", s::number("Aperture as an f-number (2.8); smaller blurs more")),
            ("focusDistance", s::number("Metres in front of the camera that are sharp (10)")),
            ("blades", s::integer("The opening's blades: 0 for round (the default), 3 to 64 for a polygon")),
            ("bladeRotation", s::number("The blades' turn in degrees (0)")),
            ("ratio", s::number("The opening's height over its width for anamorphic bokeh (1); above 1 narrows it")),
            (
                "focusObject",
                json!({ "oneOf": [{ "type": "integer" }, { "type": "string" }, { "type": "null" }], "description": "An object to keep in focus (an id or a name), or null for the distance" }),
            ),
        ])
    }),
    spec("world.set", "World Settings", &["Render"], None, true, Needs::Always, || {
        s::of(vec![
            ("color", s::color("Background colour")),
            ("strength", s::number("How strongly it (or its image) lights the scene")),
            ("rotation", s::number("Degrees the world image is turned about the up axis")),
            (
                "volume",
                json!({ "oneOf": [{ "type": "integer" }, { "type": "string" }, { "type": "null" }], "description": "A shader group (id or name) whose Volume fills the world in path-traced renders, or null for none" }),
            ),
        ])
    }),
    spec("render.set", "Render Settings", &["Render"], None, true, Needs::Always, || {
        s::of(vec![
            ("width", s::integer("Pixels")),
            ("height", s::integer("Pixels")),
            ("motionBlur", s::boolean("Blur what moves while the shutter is open, in path-traced renders")),
            ("shutter", s::number("How long the shutter is open, in frames (0.5)")),
            ("shutterPosition", s::choice("Where the shutter opens around the frame (center)", &["center", "start", "end"])),
            (
                "view",
                s::choice(
                    "Colour management's view transform (standard): standard clips at white, agx and khronos-pbr-neutral roll highlights off",
                    &kition_scene_model::color::ViewTransform::NAMES,
                ),
            ),
            ("look", s::choice("A grade after the view transform (none)", &kition_scene_model::color::Look::NAMES)),
            ("ao", s::boolean("Ambient occlusion in fast renders: creases and covered places get less of the world's light (on)")),
            (
                "aoDistance",
                json!({ "oneOf": [{ "type": "number" }, { "type": "null" }], "description": "How far around a point surfaces occlude it, in metres (up to 1000); null works it out from the scene's size (6 % of its diagonal, 0.05 to 2 m)" }),
            ),
            ("aoStrength", s::number("How much of the occlusion shows, 0 to 4 (1)")),
            ("exposure", s::number("Stops of exposure before the view transform, -32 to 32 (0)")),
        ])
    }),
    spec("render.image", "Render Image", &["Render"], Some("F12"), false, Needs::Camera, || {
        s::of(vec![
            ("samples", s::integer("Samples per pixel side, 1 to 4 (2)")),
            ("frame", s::number("The frame to render (the current frame when left out)")),
        ])
    }),
    // Files.
    spec("file.export", "Export", &["File"], None, false, Needs::Always, || {
        s::of(vec![
            ("format", s::required(s::choice("The file format", &["glb", "gltf", "obj", "stl", "fbx", "usda", "usdc", "usdz", "png"]))),
            ("selectedOnly", s::boolean("Only the selected objects")),
            ("samples", s::integer("png: samples per pixel side, 1 to 4 (2)")),
            ("frame", s::number("png: the frame to render (the current frame when left out)")),
        ])
    }),
    spec("file.import", "Import", &["File"], None, true, Needs::ObjectMode, || {
        s::of(vec![
            ("path", s::string("A .glb, .gltf, .obj, .stl, .fbx, .usd, .usda, .usdc, .usdz, or .blend file, relative to this scene's folder")),
            ("base64", s::string("The file's bytes, instead of a path")),
            ("format", s::choice("The format of base64 bytes", &["glb", "gltf", "obj", "stl", "fbx", "usd", "usda", "usdc", "usdz", "blend"])),
        ])
    }),
    // View.
    spec("view.shading", "Viewport Shading", &[], None, false, Needs::Always, || {
        s::of(vec![("mode", s::required(s::choice("How surfaces are drawn", &["wireframe", "solid", "material", "rendered"])))])
    }),
    spec("view.wireframe", "Wireframe", &["View", "Shading"], Some("Shift+Z"), false, Needs::Always, none),
    spec("view.solid", "Solid", &["View", "Shading"], None, false, Needs::Always, none),
    spec("view.material", "Material Preview", &["View", "Shading"], None, false, Needs::Always, none),
    spec("view.rendered", "Rendered", &["View", "Shading"], None, false, Needs::Always, none),
    spec("view.renderedStep", "Continue Rendered Viewport", &[], None, false, Needs::Rendered, || {
        s::of(vec![
            ("budgetMs", s::number("Worker time budget in milliseconds, clamped to 1 to 32 (8)")),
            ("samples", s::integer("Samples per preview pixel, 1 to 256 (32); changing this retains the compiled scene")),
            ("wait", s::boolean("Finish the preview before answering, for headless callers (false)")),
            (
                "maxSide",
                s::integer("The preview's longest side in pixels, 64 to 4096 (512, so a live view stays quick); the viewport's size at most"),
            ),
            ("denoise", s::boolean("Smooth the remaining noise away once the preview is done (false)")),
        ])
    }),
    spec("view.front", "Front", &["View", "Viewpoint"], Some("Alt+1"), false, Needs::Always, none),
    spec("view.right", "Right", &["View", "Viewpoint"], Some("Alt+3"), false, Needs::Always, none),
    spec("view.top", "Top", &["View", "Viewpoint"], Some("Alt+7"), false, Needs::Always, none),
    spec("view.back", "Back", &["View", "Viewpoint"], None, false, Needs::Always, none),
    spec("view.left", "Left", &["View", "Viewpoint"], None, false, Needs::Always, none),
    spec("view.bottom", "Bottom", &["View", "Viewpoint"], None, false, Needs::Always, none),
    spec("view.camera", "Camera", &["View", "Viewpoint"], Some("Alt+0"), false, Needs::Camera, none),
    spec("view.perspective", "Perspective/Orthographic", &["View"], Some("Alt+5"), false, Needs::Always, none),
    spec("view.frameAll", "Frame All", &["View"], Some("Home"), false, Needs::Always, none),
    spec("view.frameSelected", "Frame Selected", &["View"], Some("."), false, Needs::Always, none),
    spec("view.grid", "Grid Floor", &["View"], None, false, Needs::Always, || {
        s::of(vec![("on", s::boolean("Show the grid; toggles when left out"))])
    }),
    spec("view.overlays", "Overlays", &["View"], Some("Shift+Alt+Z"), false, Needs::Always, || {
        s::of(vec![("on", s::boolean("Show the overlays; toggles when left out"))])
    }),
    spec("view.orbit", "Orbit", &[], None, false, Needs::Always, || {
        s::of(vec![("turn", s::number("Degrees about the up axis")), ("tilt", s::number("Degrees up or down"))])
    }),
    spec("view.pan", "Pan", &[], None, false, Needs::Always, || {
        s::of(vec![("right", s::number("Fraction of the view's height")), ("up", s::number("Fraction of the view's height"))])
    }),
    spec("view.zoom", "Zoom", &[], None, false, Needs::Always, || {
        s::of(vec![("factor", s::required(s::number("Below 1 moves closer, above 1 farther")))])
    }),
    spec("view.set", "Set View", &[], None, false, Needs::Always, || {
        s::of(vec![
            ("target", s::vec3("The point the view turns around")),
            ("distance", s::number("Metres from the target")),
            ("tilt", s::number("Degrees: 0 looks down, 90 level")),
            ("turn", s::number("Degrees about the up axis")),
            ("perspective", s::boolean("Perspective or orthographic")),
            ("cameraZoom", s::number("Looking through a camera: how large its frame shows, 0.1 to 16 (1 fits the viewport)")),
            ("cameraOffset", s::number_pair("Looking through a camera: its frame's centre, in half viewports right and up of the centre")),
        ])
    }),
    spec("view.scheme", "Viewport Color Scheme", &[], None, false, Needs::Always, || {
        s::of(vec![("scheme", s::required(s::choice("The host's color scheme", &["light", "dark"])))])
    }),
    spec("view.resize", "Resize View", &[], None, false, Needs::Always, || {
        s::of(vec![("width", s::required(s::integer("Pixels"))), ("height", s::required(s::integer("Pixels")))])
    }),
    // The person's view state, which the editor keeps with the host (Kit UI protocol 7n), never in
    // the file.
    spec("view.state", "View State", &[], None, false, Needs::Always, none),
    spec("view.restore", "Restore View State", &[], None, false, Needs::Always, || {
        s::of(vec![(
            "state",
            s::required(serde_json::json!({ "type": "json", "description": "A view state view.state answered, in this session or another" })),
        )])
    }),
];

/// Whether `file.import` names a `.blend` file (by path or by `format`).
fn blend_import(params: &Value) -> Result<bool, String> {
    if let Some(path) = params::string(params, "path")? {
        return Ok(path.to_ascii_lowercase().ends_with(".blend"));
    }
    Ok(params::string(params, "format")? == Some("blend"))
}

/// Every command spec: these, then the second set's (`commands_model.rs`).
fn all_specs() -> impl Iterator<Item = &'static Spec> {
    SPECS.iter().chain(crate::commands_model::SPECS.iter())
}

fn spec_of(id: &str) -> Option<&'static Spec> {
    all_specs().find(|spec| spec.id == id)
}

fn has_selection(doc: &SceneDocument) -> bool {
    !doc.session.editor.selected.is_empty()
}

fn active_mesh(doc: &SceneDocument) -> Option<Id> {
    let scene = &doc.scene;
    doc.session.editor.active.filter(|id| scene.object(*id).is_ok_and(|object| object.mesh().is_some()))
}

fn enabled(doc: &SceneDocument, needs: Needs) -> bool {
    let editing = matches!(doc.session.mode, Mode::Edit(_));
    // While an armature is in edit or pose mode, object and mesh commands wait.
    let rigging = doc.session.rig.is_some();
    match needs {
        Needs::Always => true,
        Needs::Selection => !editing && !rigging && has_selection(doc),
        Needs::ObjectMode => !editing && !rigging,
        Needs::EditMode => editing,
        Needs::EditSelection => editing && !doc.session.vertices.is_empty(),
        Needs::Mesh => {
            !rigging
                && (editing
                    || active_mesh(doc).is_some()
                    || doc.session.editor.selected.iter().any(|id| doc.scene.object(*id).is_ok_and(|object| object.mesh().is_some())))
        }
        Needs::Undo => doc.can_step(None, true),
        Needs::Redo => doc.can_step(None, false),
        Needs::Camera => doc.scene.camera.is_some(),
        Needs::Rendered => doc.session.editor.shading == Shading::Rendered,
    }
}

/// Whether a command can run now. Sculpt mode takes H and Alt+H for face sets, so hiding and
/// showing objects waits until it ends.
fn spec_enabled(doc: &SceneDocument, spec: &Spec) -> bool {
    enabled(doc, spec.needs) && !(doc.session.sculpt.is_some() && matches!(spec.id, "object.hide" | "object.reveal"))
}

/// Whether command `id` changes the edited mesh's selected elements in its edit mode (`mesh.extrude`,
/// `mesh.bevel`, `mesh.loopCut`, …), and so takes `elements` (the mesh and its elements in one step).
fn takes_elements(id: &str) -> bool {
    spec_of(id).is_some_and(|spec| spec.mutates && matches!(spec.needs, Needs::EditMode | Needs::EditSelection))
}

/// Every command, with whether it can run now.
pub fn catalogue(doc: &SceneDocument) -> Vec<KitCommand> {
    let mut commands: Vec<KitCommand> = all_specs()
        .map(|spec| KitCommand {
            id: spec.id.into(),
            title: spec.title.into(),
            menu: (!spec.menu.is_empty()).then(|| spec.menu.iter().map(|part| (*part).to_string()).collect()),
            shortcut: spec.shortcut.map(Into::into),
            params: None,
            schema: Some((spec.schema)()),
            mutates: spec.mutates,
            enabled: spec_enabled(doc, spec),
            ..Default::default()
        })
        .chain(crate::commands_uv::catalogue(doc))
        .chain(crate::commands_world::catalogue(doc))
        .chain(crate::commands_render::catalogue(doc))
        .chain(crate::commands_uvedit::catalogue(doc))
        .chain(crate::modal::catalogue(doc))
        .chain(crate::cursor::catalogue(doc))
        .chain(crate::select::catalogue(doc))
        .chain(crate::operation::catalogue(doc))
        .chain(std::iter::once(crate::render::command()))
        .collect();
    // The edit-mode commands take the mesh and its elements in one step.
    let uv_edit: Vec<String> =
        crate::commands_uvedit::catalogue(doc).into_iter().filter(|command| command.mutates).map(|command| command.id).collect();
    for command in &mut commands {
        if (takes_elements(&command.id) || uv_edit.contains(&command.id))
            && let Some(Value::Object(properties)) = command.schema.as_mut()
        {
            properties.insert("elements".into(), crate::context::elements_schema());
        }
    }
    commands.extend(crate::commands_anim::catalogue(doc));
    commands.extend(crate::commands_anim2::catalogue(doc));
    commands.extend(crate::commands_nla::catalogue(doc));
    commands.extend(crate::commands_rig::catalogue(doc));
    commands.extend(crate::commands_rig2::catalogue(doc));
    commands.extend(crate::commands_sculpt::catalogue(doc));
    commands.extend(crate::commands_multires::catalogue(doc));
    commands.extend(crate::commands_particles::catalogue(doc));
    commands.extend(crate::commands_presence::catalogue(doc));
    commands.extend(crate::commands_curve::catalogue(doc));
    commands.extend(crate::commands_lattice::catalogue(doc));
    commands.extend(crate::commands_physics::catalogue(doc));
    commands.extend(crate::commands_nodes::catalogue(doc));
    commands.extend(crate::commands_compositor::catalogue(doc));
    commands.extend(crate::commands_promo::catalogue(doc));
    commands
}

/// The commands that cannot run now.
pub fn disabled(doc: &SceneDocument) -> Vec<String> {
    let mut ids: Vec<String> = all_specs()
        .filter(|spec| !spec_enabled(doc, spec))
        .map(|spec| spec.id.to_string())
        .chain(crate::commands_uv::disabled(doc))
        .chain(crate::commands_world::disabled(doc))
        .chain(crate::commands_render::disabled(doc))
        .chain(crate::commands_uvedit::disabled(doc))
        .chain(crate::modal::disabled(doc))
        .chain(crate::cursor::disabled(doc))
        .chain(crate::select::disabled(doc))
        .collect();
    ids.extend(crate::commands_anim::disabled(doc));
    ids.extend(crate::commands_anim2::disabled(doc));
    ids.extend(crate::commands_nla::disabled(doc));
    ids.extend(crate::commands_rig::disabled(doc));
    ids.extend(crate::commands_rig2::disabled(doc));
    ids.extend(crate::commands_sculpt::disabled(doc));
    ids.extend(crate::commands_multires::disabled(doc));
    ids.extend(crate::commands_particles::disabled(doc));
    ids.extend(crate::commands_presence::disabled(doc));
    ids.extend(crate::commands_curve::disabled(doc));
    ids.extend(crate::commands_lattice::disabled(doc));
    ids.extend(crate::commands_physics::disabled(doc));
    ids.extend(crate::commands_nodes::disabled(doc));
    ids.extend(crate::commands_compositor::disabled(doc));
    ids.extend(crate::commands_promo::disabled(doc));
    ids
}

/// Refuses a parameter the command does not take (a typo or a guessed name) instead of running
/// the command without it, and names the parameter it most resembles (kit requirement AX5,
/// engine rule E7a). The schemas are the ones the catalogue offers agents, read once.
fn refuse_unknown(doc: &SceneDocument, command: &str, params: &Value) -> Result<(), String> {
    schema_of(doc, command).map_or(Ok(()), |schema| kition_kit::schema::refuse_unknown(command, schema, params).map_err(bad))
}

/// The schema the catalogue offers agents for `command`, read once.
pub(crate) fn schema_of(doc: &SceneDocument, command: &str) -> Option<&'static Value> {
    static SCHEMAS: std::sync::OnceLock<std::collections::HashMap<String, Value>> = std::sync::OnceLock::new();
    let schemas = SCHEMAS.get_or_init(|| catalogue(doc).into_iter().filter_map(|command| Some((command.id, command.schema?))).collect());
    schemas.get(command)
}

/// Runs a command.
/// A number field's drag shown in the viewport without an undo step (`object.previewTransform`).
const PREVIEW_TRANSFORM: &str = "object.previewTransform";

pub fn execute(doc: &mut SceneDocument, command: &str, params: &Value) -> Result<Value, String> {
    // A world point names the object there now (`pick`, a place): it becomes the command's
    // `objects` or `object` before anything else reads the parameters.
    let params = &crate::place::resolve_pick(&doc.scene, schema_of(doc, command), params)?;
    refuse_unknown(doc, command, params)?;
    let params = &params::ids_as_numbers(&doc.scene, schema_of(doc, command), params);
    // A transform under way ends (cancelled) before any other change: the scene that change
    // starts from is the scene as it was. The view, presence and the transform's own commands
    // leave it running.
    if !keeps_modal(command) {
        doc.modal_cancel()?;
    }
    // An edit-mode command given its mesh and elements runs in that edit mode and puts it back.
    if let Some(elements) = params.get("elements").filter(|value| !value.is_null()) {
        return crate::context::with_elements(doc, command, params, elements);
    }
    // A number field's preview ends before any other change, which then starts from the scene
    // as it was (a field's final value is an `object.transform` of its own, one undo step).
    if command != PREVIEW_TRANSFORM
        && (spec_of(command).is_none_or(|spec| spec.mutates) || matches!(command, "edit.undo" | "edit.redo"))
        && let Some(before) = doc.field_preview.take()
    {
        doc.scene = before.scene;
        doc.touched();
    }
    // The agent's preview reads the scene and changes nothing.
    if command == crate::render::RENDER {
        return crate::render::render(doc, params);
    }
    // `edit.delete` naming its `objects` deletes those objects in every mode: no edit mode or
    // session takes it over to delete its own elements instead.
    let deletes_named = command == "edit.delete" && params.get("objects").is_some_and(|value| !value.is_null());
    if !deletes_named && let Some(result) = run_module(doc, command, params) {
        return result;
    }
    let Some(spec) = spec_of(command) else {
        return Err(format!("unknown-command: {command}"));
    };
    if !params.is_object() && !params.is_null() {
        return Err(bad("the parameters must be an object"));
    }
    match command {
        "view.renderedStep" => return doc.viewport_trace_step(params),
        "view.state" => return Ok(json!({ "state": doc.view_state() })),
        "view.restore" => {
            let state = params.get("state").ok_or_else(|| bad("`state` is required"))?;
            return Ok(json!({ "restored": doc.restore_view_state(state) }));
        }
        "edit.undo" => return undo(doc, params),
        "edit.redo" => return redo(doc, params),
        _ => {}
    }
    if !spec_enabled(doc, spec) && !names_targets(doc, spec, params) {
        return Err(format!("not-allowed: {} cannot run now", spec.title));
    }
    if !spec.mutates {
        return run_view(doc, command, params);
    }
    let before = doc.step(spec.title);
    let session = doc.session.clone();
    let selected = crate::operation::Selected::of(doc);
    let content = doc.content;
    match run_change(doc, command, params) {
        Ok(value) => {
            doc.tidy_session();
            if let Err(error) = doc.scene.validate() {
                doc.restore(before);
                doc.session = session;
                return Err(format!("failed: the change would break the scene ({error})"));
            }
            doc.committed(before)?;
            // A step of the person's: the last operation, which Adjust Last Operation reruns.
            if doc.content != content {
                crate::operation::record_command(doc, command, params, selected, spec.title);
            }
            Ok(value)
        }
        Err(error) => {
            doc.restore(before);
            doc.session = session;
            Err(error)
        }
    }
}

/// Runs `command` when one of the other command files has it; `None` for this file's commands.
fn run_module(doc: &mut SceneDocument, command: &str, params: &Value) -> Option<Result<Value, String>> {
    if let Some(result) = crate::modal::execute(doc, command, params) {
        return Some(result);
    }
    if let Some(result) = crate::cursor::execute(doc, command, params) {
        return Some(result);
    }
    if let Some(result) = crate::select::execute(doc, command, params) {
        return Some(result);
    }
    if let Some(result) = crate::operation::execute(doc, command, params) {
        return Some(result);
    }
    if let Some(result) = crate::commands_presence::execute(doc, command, params) {
        return Some(result);
    }
    if let Some(result) = crate::commands_promo::execute(doc, command, params) {
        return Some(result);
    }
    if let Some(result) = crate::commands_particles::execute(doc, command, params) {
        return Some(result);
    }
    if let Some(result) = crate::commands_curve::execute(doc, command, params) {
        return Some(result);
    }
    if let Some(result) = crate::commands_lattice::execute(doc, command, params) {
        return Some(result);
    }
    if let Some(result) = crate::commands_physics::execute(doc, command, params) {
        return Some(result);
    }
    if let Some(result) = crate::commands_nodes::execute(doc, command, params) {
        return Some(result);
    }
    if let Some(result) = crate::commands_compositor::execute(doc, command, params) {
        return Some(result);
    }
    if let Some(result) = crate::commands_sculpt::execute(doc, command, params) {
        return Some(result);
    }
    if let Some(result) = crate::commands_multires::execute(doc, command, params) {
        return Some(result);
    }
    if let Some(result) = crate::commands_rig2::execute(doc, command, params) {
        return Some(result);
    }
    if let Some(result) = crate::commands_render::execute(doc, command, params) {
        return Some(result);
    }
    if let Some(result) = crate::commands_uvedit::execute(doc, command, params) {
        return Some(result);
    }
    if let Some(result) = crate::commands_rig::execute(doc, command, params) {
        return Some(result);
    }
    if let Some(result) = crate::commands_uv::execute(doc, command, params) {
        return Some(result);
    }
    if let Some(result) = crate::commands_world::execute(doc, command, params) {
        return Some(result);
    }
    if let Some(result) = crate::commands_anim::execute(doc, command, params) {
        return Some(result);
    }
    if let Some(result) = crate::commands_nla::execute(doc, command, params) {
        return Some(result);
    }
    if let Some(result) = crate::commands_anim2::execute(doc, command, params) {
        return Some(result);
    }
    None
}

/// Whether `command` leaves a transform under way running: its own commands, the view's,
/// presence, proportional editing's settings, and the agent's preview.
fn keeps_modal(command: &str) -> bool {
    ["modal.", "view.", "presence.", "cursor.", "transform."].iter().any(|prefix| command.starts_with(prefix))
        || command == "operation.get"
        || command == "mesh.proportional"
        || command == crate::render::RENDER
}

/// Whether the call names what it acts on, so the gate's look at the selection does not apply:
/// a command for a mesh may name one (`object` or `objects`) when no mesh is selected, and an
/// object-mode command for the selection may name its `objects` when nothing is selected.
fn names_targets(doc: &SceneDocument, spec: &Spec, params: &Value) -> bool {
    let named = |key: &str| -> Vec<Id> {
        match params.get(key).filter(|value| !value.is_null()) {
            Some(_) => params::objects(&doc.scene, params, key).ok().flatten().unwrap_or_default(),
            None => Vec::new(),
        }
    };
    // The gate without its look at the selection: the same command needing only object mode.
    let relaxed = |needs: Needs| Spec { needs, ..*spec };
    match spec.needs {
        Needs::Mesh => {
            doc.session.rig.is_none()
                && named("object").into_iter().chain(named("objects")).any(|id| doc.scene.object(id).is_ok_and(|object| object.mesh().is_some()))
        }
        Needs::Selection => !named("objects").is_empty() && spec_enabled(doc, &relaxed(Needs::ObjectMode)),
        _ => false,
    }
}

/// `edit.undo` in its scope (KitModel M12a): `scope` names `person` or `agent`, else the
/// running command's origin decides. An agent's undo takes back only the agent's steps.
fn undo(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let scope = kition_kit::session::history_scope(params, &doc.origin).map_err(|error| error.to_string())?;
    let label = doc.step_label(scope, true);
    if !doc.step_history(scope, true)? {
        return Err("not-allowed: nothing to undo".into());
    }
    Ok(json!({ "undone": label }))
}

fn redo(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let scope = kition_kit::session::history_scope(params, &doc.origin).map_err(|error| error.to_string())?;
    let label = doc.step_label(scope, false);
    if !doc.step_history(scope, false)? {
        return Err("not-allowed: nothing to redo".into());
    }
    Ok(json!({ "redone": label }))
}

fn degrees(value: Vec3) -> Vec3 {
    Vec3::new(value.x.to_radians(), value.y.to_radians(), value.z.to_radians())
}

/// The objects a command acts on: `objects` when given, else the selection.
fn chosen(doc: &SceneDocument, params: &Value) -> Result<Vec<Id>, String> {
    match params::objects(&doc.scene, params, "objects")? {
        Some(ids) => Ok(ids),
        None if !doc.session.editor.selected.is_empty() => Ok(doc.session.editor.selected.clone()),
        None => Err("not-allowed: select an object first, or name `objects`".into()),
    }
}

/// The one object a command acts on: `key` when given, else the active object.
fn one(doc: &SceneDocument, params: &Value, key: &str) -> Result<Id, String> {
    match params.get(key).filter(|value| !value.is_null()) {
        Some(value) => params::resolve(&doc.scene, value),
        None => doc.session.editor.active.ok_or_else(|| format!("not-allowed: there is no active object; name `{key}`")),
    }
}

fn select_only(doc: &mut SceneDocument, ids: Vec<Id>) {
    doc.session.editor.active = ids.last().copied();
    doc.session.editor.selected = ids;
}

fn run_view(doc: &mut SceneDocument, command: &str, params: &Value) -> Result<Value, String> {
    let view = &mut doc.session.editor.view;
    match command {
        "edit.selectAll" => {
            let action = params::string(params, "action")?.unwrap_or("select");
            match doc.session.mode {
                Mode::Object => {
                    let visible: Vec<Id> = doc.scene.objects.iter().filter(|object| !object.hidden).map(|object| object.id).collect();
                    let selected = match action {
                        "select" => visible,
                        "deselect" => Vec::new(),
                        "invert" => visible.into_iter().filter(|id| !doc.session.editor.selected.contains(id)).collect(),
                        other => return Err(bad(format!("unknown action {other}"))),
                    };
                    doc.session.editor.active = doc.session.editor.active.filter(|id| selected.contains(id)).or_else(|| selected.last().copied());
                    doc.session.editor.selected = selected;
                }
                Mode::Edit(id) => {
                    let count = doc.scene.object(id).ok().and_then(|object| object.mesh()).map_or(0, |mesh| mesh.vertex_count());
                    doc.session.vertices = match action {
                        "select" => (0..count).collect(),
                        "deselect" => BTreeSet::new(),
                        "invert" => (0..count).filter(|vertex| !doc.session.vertices.contains(vertex)).collect(),
                        other => return Err(bad(format!("unknown action {other}"))),
                    };
                }
            }
        }
        "edit.deselectAll" => return run_view(doc, "edit.selectAll", &json!({ "action": "deselect" })),
        "object.previewTransform" => return preview_transform(doc, params),
        "select.objects" => {
            let ids = params::objects(&doc.scene, params, "objects")?.ok_or_else(|| bad("`objects` is required"))?;
            let extend = params::boolean(params, "extend")?.unwrap_or(false);
            let deselect = params::boolean(params, "deselect")?.unwrap_or(false);
            if deselect {
                doc.session.editor.selected.retain(|id| !ids.contains(id));
                if doc.session.editor.active.is_some_and(|id| ids.contains(&id)) {
                    doc.session.editor.active = doc.session.editor.selected.last().copied();
                }
            } else if extend {
                for id in ids {
                    doc.session.editor.selected.retain(|other| *other != id);
                    doc.session.editor.selected.push(id);
                    doc.session.editor.active = Some(id);
                }
            } else {
                select_only(doc, ids);
            }
        }
        "mode.toggle" => {
            let next = if matches!(doc.session.mode, Mode::Edit(_)) { "object" } else { "edit" };
            return run_view(doc, "mode.set", &json!({ "mode": next }));
        }
        "mode.set" => match params::string(params, "mode")? {
            Some("object") => {
                doc.session.mode = Mode::Object;
                doc.session.vertices.clear();
            }
            Some("edit") => {
                let id = match params.get("object").filter(|value| !value.is_null()) {
                    Some(value) => params::resolve(&doc.scene, value)?,
                    None => active_mesh(doc)
                        .or_else(|| {
                            doc.session.editor.selected.iter().copied().find(|id| doc.scene.object(*id).is_ok_and(|object| object.mesh().is_some()))
                        })
                        .ok_or("not-allowed: select a mesh object to edit")?,
                };
                let object = doc.scene.object(id).map_err(|error| error.to_string())?;
                let count = object.mesh().ok_or_else(|| bad(format!("{} is not a mesh", object.name)))?.vertex_count();
                if !doc.session.editor.selected.contains(&id) {
                    doc.session.editor.selected.push(id);
                }
                doc.session.editor.active = Some(id);
                if doc.session.mode != Mode::Edit(id) {
                    doc.session.vertices = (0..count).collect();
                }
                doc.session.mode = Mode::Edit(id);
            }
            _ => return Err(bad("`mode` must be object or edit")),
        },
        "mesh.selectMode" => {
            doc.session.select_mode = match params::string(params, "mode")? {
                Some("vertex") => SelectMode::Vertex,
                Some("edge") => SelectMode::Edge,
                Some("face") => SelectMode::Face,
                _ => return Err(bad("`mode` must be vertex or face")),
            }
        }
        "mesh.selectVertexMode" => doc.session.select_mode = SelectMode::Vertex,
        "mesh.selectEdgeMode" => doc.session.select_mode = SelectMode::Edge,
        "mesh.selectFaceMode" => doc.session.select_mode = SelectMode::Face,
        "mesh.select" => {
            let Mode::Edit(id) = doc.session.mode else { return Err("not-allowed: not in edit mode".into()) };
            let mesh = doc.scene.object(id).ok().and_then(|object| object.mesh()).ok_or("not-found: the edited mesh")?;
            let mut chosen: BTreeSet<u32> = BTreeSet::new();
            for vertex in params::indices(params, "vertices")?.unwrap_or_default() {
                if vertex >= mesh.vertex_count() {
                    return Err(format!("not-found: vertex {vertex}"));
                }
                chosen.insert(vertex);
            }
            for face in params::indices(params, "faces")?.unwrap_or_default() {
                let corners = mesh.faces.get(face as usize).ok_or_else(|| format!("not-found: face {face}"))?;
                chosen.extend(corners.iter().copied());
            }
            if params::boolean(params, "deselect")?.unwrap_or(false) {
                doc.session.vertices.retain(|vertex| !chosen.contains(vertex));
            } else if params::boolean(params, "extend")?.unwrap_or(false) {
                doc.session.vertices.extend(chosen);
            } else {
                doc.session.vertices = chosen;
            }
        }
        "view.shading" => {
            doc.session.editor.shading = match params::string(params, "mode")? {
                Some("wireframe") => Shading::Wireframe,
                Some("solid") => Shading::Solid,
                Some("material") => Shading::Material,
                Some("rendered") => Shading::Rendered,
                _ => return Err(bad("`mode` must be wireframe, solid, material, or rendered")),
            }
        }
        "view.wireframe" => {
            doc.session.editor.shading = if doc.session.editor.shading == Shading::Wireframe { Shading::Solid } else { Shading::Wireframe }
        }
        "view.solid" => doc.session.editor.shading = Shading::Solid,
        "view.material" => doc.session.editor.shading = Shading::Material,
        "view.rendered" => doc.session.editor.shading = Shading::Rendered,
        "view.front" => view.align(Axis::Front),
        "view.right" => view.align(Axis::Right),
        "view.top" => view.align(Axis::Top),
        "view.back" => view.align(Axis::Back),
        "view.left" => view.align(Axis::Left),
        "view.bottom" => view.align(Axis::Bottom),
        "view.camera" => {
            view.camera = if view.camera.is_some() { None } else { doc.scene.camera };
        }
        "view.perspective" => {
            view.camera = None;
            view.perspective = !view.perspective;
        }
        "view.frameAll" => frame(doc, None),
        "view.frameSelected" => {
            let ids = doc.session.editor.selected.clone();
            frame(doc, if ids.is_empty() { None } else { Some(ids) });
        }
        "view.grid" => doc.session.editor.grid = params::boolean(params, "on")?.unwrap_or(!doc.session.editor.grid),
        "view.overlays" => doc.session.editor.overlays = params::boolean(params, "on")?.unwrap_or(!doc.session.editor.overlays),
        "view.orbit" => {
            view.orbit(params::number(params, "turn")?.unwrap_or(0.0).to_radians(), params::number(params, "tilt")?.unwrap_or(0.0).to_radians())
        }
        "view.pan" => view.pan(params::number(params, "right")?.unwrap_or(0.0), params::number(params, "up")?.unwrap_or(0.0)),
        "view.zoom" => view.zoom(params::need_number(params, "factor")?),
        "view.set" => {
            let mut orbits = false;
            if let Some(target) = params::vec3(params, "target")? {
                view.target = target;
                orbits = true;
            }
            if let Some(distance) = params::number(params, "distance")? {
                view.distance = distance.clamp(0.01, 100_000.0);
                orbits = true;
            }
            if let Some(tilt) = params::number(params, "tilt")? {
                view.tilt = tilt.to_radians().clamp(0.0, std::f32::consts::PI);
                orbits = true;
            }
            if let Some(turn) = params::number(params, "turn")? {
                view.turn = turn.to_radians().rem_euclid(std::f32::consts::TAU);
                orbits = true;
            }
            if let Some(perspective) = params::boolean(params, "perspective")? {
                view.perspective = perspective;
                orbits = true;
            }
            let zoom = params::number(params, "cameraZoom")?;
            let offset = params::number_pair(params, "cameraOffset")?;
            if let Some(zoom) = zoom {
                let (low, high) = kition_scene_raster::view::CAMERA_ZOOM;
                view.camera_zoom = zoom.clamp(low, high);
            }
            if let Some([x, y]) = offset {
                let reach = kition_scene_raster::view::CAMERA_OFFSET;
                view.camera_offset = [x.clamp(-reach, reach), y.clamp(-reach, reach)];
            }
            // The camera's frame zoomed or moved keeps the camera view; anything else leaves it.
            if orbits || (zoom.is_none() && offset.is_none()) {
                view.camera = None;
            }
        }
        "view.scheme" => {
            let name = params::string(params, "scheme")?.ok_or_else(|| bad("`scheme` is required"))?;
            let scheme = kition_scene_raster::Scheme::parse(name).ok_or_else(|| bad(format!("`scheme` must be light or dark, not {name}")))?;
            if scheme == doc.session.scheme {
                return Ok(json!({ "changed": false }));
            }
            doc.session.scheme = scheme;
        }
        "view.resize" => {
            let width = params::integer(params, "width")?.ok_or_else(|| bad("`width` is required"))?;
            let height = params::integer(params, "height")?.ok_or_else(|| bad("`height` is required"))?;
            if !(1..=crate::MAX_VIEWPORT).contains(&width) || !(1..=crate::MAX_VIEWPORT).contains(&height) {
                return Err(bad(format!("the view must be between 1 and {} pixels on each side", crate::MAX_VIEWPORT)));
            }
            if (width, height) == (doc.session.width, doc.session.height) {
                return Ok(json!({ "changed": false }));
            }
            doc.session.width = width;
            doc.session.height = height;
        }
        "render.image" => {
            let samples = params::integer(params, "samples")?.unwrap_or(2);
            let png = doc.render_png_at(samples, params::number(params, "frame")?)?;
            return Ok(
                json!({ "mime": "image/png", "extension": "png", "base64": base64(&png), "width": doc.scene.render.width, "height": doc.scene.render.height }),
            );
        }
        "file.export" => return export(doc, params),
        other => return crate::commands_model::run_view(doc, other, params),
    }
    doc.touched();
    Ok(json!({ "changed": true }))
}

fn frame(doc: &mut SceneDocument, ids: Option<Vec<Id>>) {
    let bounds = match (doc.session.mode, &ids) {
        (Mode::Edit(id), Some(_)) => {
            let (Ok(object), Ok(world)) = (doc.scene.object(id), doc.scene.world_matrix(id)) else { return };
            let Some(mesh) = object.mesh() else { return };
            let chosen: Vec<_> = if doc.session.vertices.is_empty() {
                mesh.positions.clone()
            } else {
                doc.session.vertices.iter().filter_map(|vertex| mesh.positions.get(*vertex as usize).copied()).collect()
            };
            chosen.iter().fold(kition_scene_model::Bounds::EMPTY, |bounds, point| bounds.with(world.transform_point3(*point)))
        }
        _ => doc.scene.bounds(ids.as_deref()),
    };
    if bounds.is_empty() {
        return;
    }
    doc.session.editor.view.camera = None;
    doc.session.editor.view.frame(bounds.center(), bounds.size().length() / 2.0);
}

pub(crate) fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let (a, b, c) = (chunk.first().copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0));
        let triple = (u32::from(a) << 16) | (u32::from(b) << 8) | u32::from(c);
        for (index, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            if index <= chunk.len() {
                out.push(char::from(TABLE.get(((triple >> shift) & 63) as usize).copied().unwrap_or(b'A')));
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn unbase64(text: &str) -> Result<Vec<u8>, String> {
    let value = |byte: u8| -> Option<u32> {
        match byte {
            b'A'..=b'Z' => Some(u32::from(byte - b'A')),
            b'a'..=b'z' => Some(u32::from(byte - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(byte - b'0') + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let clean: Vec<u8> = text.bytes().filter(|byte| !byte.is_ascii_whitespace() && *byte != b'=').collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        let mut triple = 0u32;
        for (index, byte) in chunk.iter().enumerate() {
            triple |= value(*byte).ok_or_else(|| bad("`base64` is not base64"))? << (18 - 6 * index);
        }
        let bytes = triple.to_be_bytes();
        out.extend(bytes.iter().skip(1).take(chunk.len().saturating_sub(1)));
    }
    Ok(out)
}

fn export(doc: &SceneDocument, params: &Value) -> Result<Value, String> {
    let format = params::string(params, "format")?.ok_or_else(|| bad("`format` is required"))?;
    let only = if params::boolean(params, "selectedOnly")?.unwrap_or(false) { Some(doc.session.editor.selected.clone()) } else { None };
    let (bytes, mime) = if format == "png" {
        (doc.render_png_at(params::integer(params, "samples")?.unwrap_or(2), params::number(params, "frame")?)?, "image/png")
    } else {
        let exchange = crate::formats::Exchange::from_extension(format)
            .filter(|_| crate::formats::EXPORTS.contains(&format))
            .ok_or_else(|| bad(format!("`format` must be {}, or png, not {format}", crate::formats::EXPORTS.join(", "))))?;
        let (scene, only) = crate::commands_particles::export_scene(doc, only);
        exchange.export(&scene, only.as_deref())?
    };
    Ok(json!({ "format": format, "extension": format, "mime": mime, "base64": base64(&bytes) }))
}

fn primitive(command: &str, params: &Value) -> Result<Primitive, String> {
    let kind = match command {
        "object.addPlane" => "plane",
        "object.addCube" => "cube",
        "object.addCircle" => "circle",
        "object.addUvSphere" => "uvSphere",
        "object.addIcoSphere" => "icoSphere",
        "object.addCylinder" => "cylinder",
        "object.addCone" => "cone",
        "object.addTorus" => "torus",
        "object.addGrid" => "grid",
        other => return Err(format!("unknown-command: {other}")),
    };
    let mut fields = serde_json::Map::new();
    fields.insert("type".into(), Value::String(kind.into()));
    let names = [
        "size",
        "vertices",
        "radius",
        "fill",
        "segments",
        "rings",
        "subdivisions",
        "depth",
        "radius1",
        "radius2",
        "majorSegments",
        "minorSegments",
        "majorRadius",
        "minorRadius",
        "xSubdivisions",
        "ySubdivisions",
    ];
    for name in names {
        if let Some(value) = params.get(name).filter(|value| !value.is_null()) {
            fields.insert(name.into(), value.clone());
        }
    }
    serde_json::from_value(Value::Object(fields)).map_err(bad)
}

/// Applies `location`, `rotation`, `scale` from the parameters to a new object.
fn place(doc: &mut SceneDocument, id: Id, params: &Value) -> Result<(), String> {
    let location = params::vec3(params, "location")?;
    let rotation = params::vec3(params, "rotation")?;
    let scale = params::vec3(params, "scale")?;
    let object = doc.scene.object_mut(id).map_err(|error| error.to_string())?;
    if let Some(location) = location {
        object.location = location;
    }
    if let Some(rotation) = rotation {
        object.rotation = degrees(rotation);
    }
    if let Some(scale) = scale {
        object.scale = scale;
    }
    Ok(())
}

fn add_object(doc: &mut SceneDocument, default_name: &str, data: ObjectData, params: &Value) -> Result<Value, String> {
    let name = params::string(params, "name")?.unwrap_or(default_name);
    let id = doc.scene.add_object(name, data);
    place(doc, id, params)?;
    select_only(doc, vec![id]);
    let name = doc.scene.object(id).map(|object| object.name.clone()).unwrap_or_default();
    Ok(json!({ "object": id, "name": name }))
}

/// The mode (other than object and edit mode) whose session is on, by name. Sculpt Mode, Particle
/// Edit and Weight Paint have no elements Delete deletes, and their selection is the object being
/// worked on; the armature's and the shapes' edit modes delete their own elements (their command
/// files take Delete over) unless it names `objects`.
fn session_on(doc: &SceneDocument) -> Option<&'static str> {
    let session = &doc.session;
    if session.sculpt.is_some() {
        Some("Sculpt Mode")
    } else if session.particle_edit.is_some() {
        Some("Particle Edit")
    } else if session.paint.is_some() {
        Some("Weight Paint")
    } else if session.rig.is_some() {
        Some("the armature's edit or pose mode")
    } else if session.shape.is_some() {
        Some("the curve's, text's or lattice's edit mode")
    } else {
        None
    }
}

fn edited(doc: &SceneDocument) -> Result<Id, String> {
    match doc.session.mode {
        Mode::Edit(id) => Ok(id),
        Mode::Object => Err("not-allowed: not in edit mode".into()),
    }
}

/// The edit selection a mesh edit works on: the selected vertices (it reads as their set), and the
/// faces `elements.faces` named when it named them.
pub(crate) struct Selection {
    vertices: BTreeSet<u32>,
    faces: Option<BTreeSet<u32>>,
}

impl std::ops::Deref for Selection {
    type Target = BTreeSet<u32>;

    fn deref(&self) -> &BTreeSet<u32> {
        &self.vertices
    }
}

impl<'a> IntoIterator for &'a Selection {
    type Item = &'a u32;
    type IntoIter = std::collections::btree_set::Iter<'a, u32>;

    fn into_iter(self) -> Self::IntoIter {
        self.vertices.iter()
    }
}

pub(crate) fn edit_mesh<T>(
    doc: &mut SceneDocument,
    change: impl FnOnce(&mut kition_scene_model::Mesh, &Selection) -> Result<T, String>,
) -> Result<T, String> {
    let id = edited(doc)?;
    let selection = Selection { vertices: doc.session.vertices.clone(), faces: doc.session.faces.clone() };
    let mesh = doc.scene.object_mut(id).map_err(|error| error.to_string())?.mesh_mut().ok_or("unsupported: the edited object is not a mesh")?;
    change(mesh, &selection)
}

/// The selected faces: the named ones when `elements.faces` named them, else every face whose
/// vertices are all selected.
pub(crate) fn selected_faces(mesh: &kition_scene_model::Mesh, selection: &Selection) -> Vec<u32> {
    named_or_within(mesh, &selection.vertices, selection.faces.as_ref()).into_iter().collect()
}

/// [`selected_faces`] of the session's edit selection.
pub(crate) fn faces_selected(session: &crate::Session, mesh: &kition_scene_model::Mesh) -> BTreeSet<u32> {
    named_or_within(mesh, &session.vertices, session.faces.as_ref())
}

pub(crate) fn named_or_within(mesh: &kition_scene_model::Mesh, vertices: &BTreeSet<u32>, named: Option<&BTreeSet<u32>>) -> BTreeSet<u32> {
    match named {
        Some(faces) => faces.iter().copied().filter(|face| (*face as usize) < mesh.faces.len()).collect(),
        None => ops::faces_within(mesh, vertices),
    }
}

fn factor3(params: &Value) -> Result<Vec3, String> {
    match params.get("factor") {
        Some(Value::Number(_)) => Ok(Vec3::splat(params::need_number(params, "factor")?)),
        Some(_) => params::vec3(params, "factor")?.ok_or_else(|| bad("`factor` must be a number or [x, y, z]")),
        None => Err(bad("`factor` is required")),
    }
}

pub(crate) fn find_modifier(doc: &SceneDocument, params: &Value) -> Result<(Id, usize), String> {
    let id = one(doc, params, "object")?;
    let object = doc.scene.object(id).map_err(|error| error.to_string())?;
    let index = match (params::integer(params, "index")?, params::string(params, "name")?) {
        (Some(index), _) => index as usize,
        (None, Some(name)) => object
            .modifiers
            .iter()
            .position(|modifier| modifier.name == name)
            .ok_or_else(|| format!("not-found: no modifier named {name} on {}", object.name))?,
        (None, None) => return Err(bad("name the modifier by `index` or `name`")),
    };
    if index >= object.modifiers.len() {
        return Err(format!("not-found: {} has no modifier {index}", object.name));
    }
    Ok((id, index))
}

/// Modifier settings people and agents give in degrees; the model keeps radians.
const DEGREE_SETTINGS: &[&str] = &["angleLimit", "angle"];

/// Applies the settings present in `params` to a modifier kind on object `target` (other
/// settings stay). Angles come in degrees; a boolean's, shrinkwrap's, lattice or armature
/// modifier's other object is `operand` (an id or a name), since `object` names the modified
/// object itself.
fn configure(scene: &Scene, target: Id, kind: &ModifierKind, params: &Value) -> Result<ModifierKind, String> {
    let mut value = serde_json::to_value(kind).map_err(|error| error.to_string())?;
    let tag = value.get("type").cloned();
    let other_object =
        matches!(kind, ModifierKind::Boolean { .. } | ModifierKind::Shrinkwrap { .. } | ModifierKind::Lattice { .. } | ModifierKind::Armature { .. });
    if let (Some(fields), Some(given)) = (value.as_object_mut(), params.as_object()) {
        let mut known: Vec<String> = fields.keys().cloned().collect();
        if other_object {
            known.push("object".into());
        }
        if matches!(kind, ModifierKind::GeometryNodes { .. }) {
            known.extend(["group".into(), "inputs".into()]);
        }
        if matches!(kind, ModifierKind::Displace { .. }) {
            known.push("image".into());
        }
        if matches!(kind, ModifierKind::Armature { .. }) {
            known.extend(["preserveVolume", "vertexGroups", "envelopes", "vertexGroup", "invertVertexGroup", "multiModifier"].map(String::from));
        }
        for (key, setting) in given {
            if key == "type" || key == "object" || !known.contains(key) {
                continue;
            }
            let setting = if DEGREE_SETTINGS.contains(&key.as_str()) {
                let degrees =
                    setting.as_f64().filter(|value| value.is_finite()).ok_or_else(|| bad(format!("`{key}` must be a number of degrees")))?;
                json!(degrees.to_radians())
            } else {
                setting.clone()
            };
            fields.insert(key.clone(), setting);
        }
        if other_object && let Some(operand) = given.get("operand").filter(|value| !value.is_null()) {
            let id = params::resolve(scene, operand)?;
            if id == target {
                return Err(bad("a modifier cannot use the object it modifies"));
            }
            let data = scene.object(id).map(|object| &object.data).map_err(|error| error.to_string())?;
            match kind {
                ModifierKind::Lattice { .. } if !matches!(data, ObjectData::Lattice(_)) => {
                    return Err(bad("a lattice modifier's operand must be a lattice"));
                }
                ModifierKind::Shrinkwrap { .. } if !scene.object(id).is_ok_and(Object::has_geometry) => {
                    return Err(bad("a shrinkwrap's operand must be a mesh, curve, or text"));
                }
                ModifierKind::Boolean { .. } if !matches!(data, ObjectData::Mesh(_)) => return Err(bad("a boolean's operand must be a mesh object")),
                ModifierKind::Armature { .. } if !matches!(data, ObjectData::Armature(_)) => {
                    return Err(bad("an armature modifier's operand must be an armature"));
                }
                _ => {}
            }
            fields.insert("object".into(), json!(id));
        }
        if let Some(Value::String(hash)) = given.get("image")
            && scene.image(hash).is_none()
        {
            return Err(format!("not-found: no image {hash}"));
        }
        if let Some(tag) = tag {
            fields.insert("type".into(), tag);
        }
    }
    let kind: ModifierKind = serde_json::from_value(value).map_err(bad)?;
    kind.check().map_err(|error| error.to_string())?;
    Ok(kind)
}

/// A modifier as inspect shows it: angles in degrees, another object (a boolean's, a
/// shrinkwrap's, a lattice) as `operand`.
pub fn modifier_json(modifier: &Modifier) -> Value {
    let mut value = serde_json::to_value(modifier).unwrap_or(Value::Null);
    if let Some(fields) = value.as_object_mut() {
        for key in DEGREE_SETTINGS {
            if let Some(radians) = fields.get(*key).and_then(Value::as_f64) {
                fields.insert((*key).into(), json!((radians.to_degrees() * 1000.0).round() / 1000.0));
            }
        }
        if matches!(fields.get("type").and_then(Value::as_str), Some("boolean" | "shrinkwrap" | "lattice"))
            && let Some(operand) = fields.remove("object")
        {
            fields.insert("operand".into(), operand);
        }
    }
    value
}

fn material_ref(scene: &Scene, params: &Value) -> Result<Id, String> {
    material_ref_value(scene, params.get("material").unwrap_or(&Value::Null))
}

/// The material `value` names (an id or a name).
pub(crate) fn material_ref_value(scene: &Scene, value: &Value) -> Result<Id, String> {
    match value {
        Value::Number(number) => {
            let id = number.as_u64().and_then(|id| u32::try_from(id).ok()).ok_or_else(|| bad("`material` must be an id or a name"))?;
            scene.material(id).map(|material| material.id).map_err(|_| format!("not-found: no material {id}"))
        }
        Value::String(name) => scene
            .materials
            .iter()
            .find(|material| &material.name == name)
            .map(|material| material.id)
            .ok_or_else(|| format!("not-found: no material named {name}")),
        _ => Err(bad("`material` is required (an id or a name)")),
    }
}

fn set_material(scene: &mut Scene, id: Id, params: &Value) -> Result<(), String> {
    let name = params::string(params, "name")?.map(str::to_string);
    let base = params::color(params, "baseColor")?;
    let metallic = params::number(params, "metallic")?;
    let roughness = params::number(params, "roughness")?;
    let emission = params::color(params, "emission")?;
    let strength = params::number(params, "emissionStrength")?;
    let normal_strength = params::number(params, "normalStrength")?;
    let transmission = params::number(params, "transmission")?;
    let ior = params::number(params, "ior")?;
    let alpha_mode = match params::string(params, "alphaMode")? {
        None => None,
        Some("opaque") => Some(kition_scene_model::AlphaMode::Opaque),
        Some("clip") => Some(kition_scene_model::AlphaMode::Clip),
        Some("blend") => Some(kition_scene_model::AlphaMode::Blend),
        Some(other) => return Err(bad(format!("`alphaMode` must be opaque, clip, or blend, not {other}"))),
    };
    let alpha_cutoff = params::number(params, "alphaCutoff")?;
    let shader = match params.get("shader") {
        Some(value) => Some(crate::commands_nodes::shader_ref(scene, value)?),
        None => None,
    };
    let taken = name.as_ref().is_some_and(|name| scene.materials.iter().any(|material| &material.name == name && material.id != id));
    let material = scene.material_mut(id).map_err(|error| error.to_string())?;
    if let Some(normal_strength) = normal_strength {
        material.normal_strength = normal_strength.clamp(-10.0, 10.0);
    }
    if let Some(transmission) = transmission {
        material.transmission = transmission.clamp(0.0, 1.0);
    }
    if let Some(ior) = ior {
        material.ior = ior.clamp(1.0, 10.0);
    }
    if let Some(mode) = alpha_mode {
        material.alpha_mode = mode;
    }
    if let Some(cutoff) = alpha_cutoff {
        material.alpha_cutoff = cutoff.clamp(0.0, 1.0);
    }
    if let Some(name) = name {
        if taken {
            return Err(bad(format!("a material is already called {name}")));
        }
        material.name = name;
    }
    if let Some(base) = base {
        material.base_color = base;
    }
    if let Some(metallic) = metallic {
        material.metallic = metallic.clamp(0.0, 1.0);
    }
    if let Some(roughness) = roughness {
        material.roughness = roughness.clamp(0.0, 1.0);
    }
    if let Some([r, g, b, _]) = emission {
        material.emission = Vec3::new(r, g, b);
    }
    if let Some(strength) = strength {
        material.emission_strength = strength.max(0.0);
    }
    if let Some(shader) = shader {
        material.shader = shader;
    }
    Ok(())
}

/// Rotation that makes an object's -Z look at `target` with +Y as close to world up as can be.
pub(crate) fn look_rotation(from: Vec3, target: Vec3) -> Option<Vec3> {
    let forward = (target - from).normalize_or_zero();
    if forward == Vec3::ZERO {
        return None;
    }
    let back = -forward;
    let up_hint = if back.cross(Vec3::Z).length_squared() < 1e-8 { Vec3::Y } else { Vec3::Z };
    let right = up_hint.cross(back).normalize_or_zero();
    let up = back.cross(right);
    Some(matrix_euler(Mat3::from_cols(right, up, back)))
}

/// Shows `object.transform`'s change while a number field is dragged, without an undo step or an
/// unsaved change: each call starts again from the scene as it was before the drag, and `end`
/// (or any change, an undo, a redo) puts that scene back. The field's final value is then an
/// `object.transform`, one undo step.
fn preview_transform(doc: &mut SceneDocument, params: &Value) -> Result<Value, String> {
    let end = params::boolean(params, "end")?.unwrap_or(false);
    if !end {
        chosen(doc, params)?;
    }
    let before = match doc.field_preview.take() {
        Some(before) => {
            doc.scene = before.scene.clone();
            before
        }
        None => doc.step("Preview Transform"),
    };
    if end {
        doc.touched();
        return Ok(json!({ "previewing": false }));
    }
    let shown = run_change(doc, "object.transform", params)
        .and_then(|_| doc.scene.validate().map_err(|error| format!("failed: the change would break the scene ({error})")));
    match shown {
        Ok(()) => {
            doc.field_preview = Some(before);
            doc.touched();
            Ok(json!({ "previewing": true }))
        }
        Err(error) => {
            doc.scene = before.scene;
            doc.touched();
            Err(error)
        }
    }
}

pub(crate) fn run_change(doc: &mut SceneDocument, command: &str, params: &Value) -> Result<Value, String> {
    match command {
        "edit.delete" if params.get("objects").is_some_and(|value| !value.is_null()) => {
            let ids = chosen(doc, params)?;
            if ids.is_empty() {
                return Err("not-allowed: `objects` names no object to delete".into());
            }
            doc.scene.remove_objects(&ids).map_err(|error| error.to_string())?;
            if doc.session.mode == Mode::Object && session_on(doc).is_none() {
                doc.session.editor.selected.clear();
                doc.session.editor.active = None;
            }
            // An edit mode or session on an object that is gone ends (tidy_session); one on another
            // object goes on.
            Ok(json!({ "deleted": ids }))
        }
        "edit.delete" => match doc.session.mode {
            Mode::Object => {
                if let Some(session) = session_on(doc) {
                    return Err(format!(
                        "not-allowed: Delete has nothing to delete in {session}; name the `objects` to delete, or leave {session} first"
                    ));
                }
                let ids = chosen(doc, params)?;
                doc.scene.remove_objects(&ids).map_err(|error| error.to_string())?;
                doc.session.editor.selected.clear();
                doc.session.editor.active = None;
                Ok(json!({ "deleted": ids }))
            }
            Mode::Edit(_) => match doc.session.select_mode {
                SelectMode::Face => run_change(doc, "mesh.deleteFaces", params),
                SelectMode::Vertex | SelectMode::Edge => run_change(doc, "mesh.deleteVertices", params),
            },
        },
        "edit.duplicate" => {
            let offset = params::vec3(params, "offset")?.unwrap_or(Vec3::ZERO);
            match doc.session.mode {
                Mode::Object => {
                    let ids = chosen(doc, params)?;
                    let mut made = Vec::new();
                    for id in ids {
                        let mut copy = doc.scene.object(id).map_err(|error| error.to_string())?.clone();
                        copy.id = doc.scene.take_id();
                        copy.name = doc.scene.unique_object_name(&copy.name);
                        copy.location += offset;
                        made.push(copy.id);
                        let copied = copy.id;
                        doc.scene.objects.push(copy);
                        doc.scene.copy_animation(id, copied);
                    }
                    select_only(doc, made.clone());
                    Ok(json!({ "objects": made }))
                }
                Mode::Edit(_) => {
                    let selection = doc.session.vertices.clone();
                    let made = edit_mesh(doc, |mesh, vertices| {
                        let faces = selected_faces(mesh, vertices);
                        let mut part = mesh.part(vertices, &faces).map_err(|error| error.to_string())?;
                        for point in &mut part.positions {
                            *point += offset;
                        }
                        let first = mesh.append(&part);
                        Ok((first..first + part.vertex_count()).collect::<BTreeSet<u32>>())
                    })?;
                    let _ = selection;
                    doc.session.vertices = made.clone();
                    Ok(json!({ "vertices": made }))
                }
            }
        }
        "object.addPlane"
        | "object.addCube"
        | "object.addCircle"
        | "object.addUvSphere"
        | "object.addIcoSphere"
        | "object.addCylinder"
        | "object.addCone"
        | "object.addTorus"
        | "object.addGrid" => {
            let shape = primitive(command, params)?;
            let mut mesh = shape.build().map_err(|error| error.to_string())?;
            if let Mode::Edit(id) = doc.session.mode {
                // In edit mode a primitive joins the mesh being edited, where the placement says.
                let placement = Mat4::from_scale_rotation_translation(
                    params::vec3(params, "scale")?.unwrap_or(Vec3::ONE),
                    glam::Quat::from_mat3(&euler_matrix(degrees(params::vec3(params, "rotation")?.unwrap_or(Vec3::ZERO)))),
                    params::vec3(params, "location")?.unwrap_or(Vec3::ZERO),
                );
                let inverse = doc.scene.world_matrix(id).map_err(|error| error.to_string())?.inverse();
                mesh.transform(inverse * placement);
                let made = edit_mesh(doc, |target, _| {
                    let first = target.append(&mesh);
                    Ok((first..first + mesh.vertex_count()).collect::<BTreeSet<u32>>())
                })?;
                let count = made.len();
                doc.session.vertices = made;
                return Ok(json!({ "object": id, "vertices": count }));
            }
            add_object(doc, shape.display_name(), ObjectData::Mesh(mesh), params)
        }
        "object.addBackdrop" => {
            let number = |key: &str, default: f32| -> Result<f32, String> { Ok(params::number(params, key)?.unwrap_or(default)) };
            let segments = params::integer(params, "segments")?.unwrap_or(16);
            let mesh = kition_scene_model::primitives::backdrop(
                number("width", 20.0)?,
                number("depth", 10.0)?,
                number("height", 8.0)?,
                number("radius", 3.0)?,
                segments,
            )
            .map_err(|error| error.to_string())?;
            add_object(doc, "Backdrop", ObjectData::Mesh(mesh), params)
        }
        "object.addLight" => {
            let kind = match params::string(params, "kind")?.unwrap_or("point") {
                "point" => LightKind::Point,
                "sun" => LightKind::Sun,
                "spot" => LightKind::Spot,
                "area" => LightKind::Area,
                other => return Err(bad(format!("unknown light kind {other}"))),
            };
            let mut light = Light::new(kind);
            if let Some(power) = params::number(params, "power")? {
                light.power = power.max(0.0);
            }
            if let Some([r, g, b, _]) = params::color(params, "color")? {
                light.color = Vec3::new(r, g, b);
            }
            add_object(doc, "Light", ObjectData::Light(light), params)
        }
        "object.addCamera" => {
            let mut camera = Camera::default();
            if let Some(lens) = params::number(params, "lens")? {
                if !(1.0..=5000.0).contains(&lens) {
                    return Err(bad("`lens` must be between 1 and 5000 mm"));
                }
                camera.lens = lens;
            }
            let value = add_object(doc, "Camera", ObjectData::Camera(camera), params)?;
            if doc.scene.camera.is_none() {
                doc.scene.camera = value.get("object").and_then(Value::as_u64).and_then(|id| u32::try_from(id).ok());
            }
            Ok(value)
        }
        "object.addEmpty" => {
            let size = params::number(params, "size")?.unwrap_or(1.0);
            if size.is_nan() || size <= 0.0 {
                return Err(bad("`size` must be positive"));
            }
            add_object(doc, "Empty", ObjectData::Empty { size }, params)
        }
        "object.transform" => {
            let ids = chosen(doc, params)?;
            let location = params::vec3(params, "location")?;
            let rotation = params::vec3(params, "rotation")?;
            let scale = params::vec3(params, "scale")?;
            if location.is_none() && rotation.is_none() && scale.is_none() {
                return Err(bad("give `location`, `rotation`, or `scale`"));
            }
            for id in &ids {
                let object = doc.scene.object_mut(*id).map_err(|error| error.to_string())?;
                if let Some(location) = location {
                    object.location = location;
                }
                if let Some(rotation) = rotation {
                    object.rotation = degrees(rotation);
                }
                if let Some(scale) = scale {
                    object.scale = scale;
                }
            }
            Ok(moved(&doc.scene, ids))
        }
        "object.translate" => {
            let ids = chosen(doc, params)?;
            let offset = params::vec3(params, "offset")?.ok_or_else(|| bad("`offset` is required"))?;
            translate_objects(&mut doc.scene, &ids, offset)?;
            Ok(moved(&doc.scene, ids))
        }
        "object.rotate" => {
            let ids = chosen(doc, params)?;
            let axis = match params::string(params, "axis")? {
                Some("x") => Vec3::X,
                Some("y") => Vec3::Y,
                Some("z") => Vec3::Z,
                _ => return Err(bad("`axis` must be x, y, or z")),
            };
            let angle = params::need_number(params, "angle")?.to_radians();
            rotate_objects(&mut doc.scene, &ids, Mat3::from_axis_angle(axis, angle))?;
            Ok(moved(&doc.scene, ids))
        }
        "object.scale" => {
            let ids = chosen(doc, params)?;
            let factor = factor3(params)?;
            // A child whose parent is scaled too goes with it (scaled once).
            for id in top_level(&doc.scene, &ids) {
                doc.scene.object_mut(id).map_err(|error| error.to_string())?.scale *= factor;
            }
            Ok(moved(&doc.scene, ids))
        }
        "object.applyTransform" => {
            let ids = chosen(doc, params)?;
            let (location, rotation, scale) = (
                params::boolean(params, "location")?.unwrap_or(false),
                params::boolean(params, "rotation")?.unwrap_or(true),
                params::boolean(params, "scale")?.unwrap_or(true),
            );
            for id in &ids {
                let object = doc.scene.object(*id).map_err(|error| error.to_string())?;
                if object.mesh().is_none() {
                    continue;
                }
                let applied = Mat4::from_translation(if location { object.location } else { Vec3::ZERO })
                    * Mat4::from_mat3(if rotation { euler_matrix(object.rotation) } else { Mat3::IDENTITY })
                    * Mat4::from_scale(if scale { object.scale } else { Vec3::ONE });
                let kept = Mat4::from_translation(if location { Vec3::ZERO } else { object.location })
                    * Mat4::from_mat3(if rotation { Mat3::IDENTITY } else { euler_matrix(object.rotation) })
                    * Mat4::from_scale(if scale { Vec3::ONE } else { object.scale });
                // The kept parts go back on the object; the applied ones into the mesh, expressed so
                // the world stays put: local = kept · inner, with inner = kept⁻¹ · applied-order.
                let full = doc.scene.object(*id).map_err(|error| error.to_string())?.local_matrix();
                let inner = kept.inverse() * full;
                let _ = applied;
                rebase(&mut doc.scene, *id, inner)?;
                let object = doc.scene.object_mut(*id).map_err(|error| error.to_string())?;
                object.set_local_matrix(kept);
            }
            Ok(json!({ "objects": ids }))
        }
        "object.originToGeometry" => {
            let ids = chosen(doc, params)?;
            for id in &ids {
                let object = doc.scene.object(*id).map_err(|error| error.to_string())?;
                let Some(mesh) = object.mesh() else { continue };
                let centre = mesh.bounds(Mat4::IDENTITY);
                if centre.is_empty() {
                    continue;
                }
                let centre = centre.center();
                let local = object.local_matrix();
                // The mesh moves back by the centre; the origin moves to it (and the children with it).
                rebase(&mut doc.scene, *id, Mat4::from_translation(-centre))?;
                let object = doc.scene.object_mut(*id).map_err(|error| error.to_string())?;
                object.location = local.transform_point3(centre);
            }
            Ok(json!({ "objects": ids }))
        }
        "object.order" => {
            let id = one(doc, params, "object")?;
            let index = params::integer(params, "index")?.ok_or_else(|| bad("`index` is required"))? as usize;
            let from = doc.scene.objects.iter().position(|object| object.id == id).ok_or("not-found: the object")?;
            let object = doc.scene.objects.remove(from);
            let to = index.min(doc.scene.objects.len());
            doc.scene.objects.insert(to, object);
            Ok(json!({ "object": id, "index": to }))
        }
        "object.rename" => {
            let id = one(doc, params, "object")?;
            let name = params::string(params, "name")?.map(str::trim).filter(|name| !name.is_empty()).ok_or_else(|| bad("`name` is required"))?;
            let unique = if doc.scene.object_named(name).is_some_and(|object| object.id != id) {
                doc.scene.unique_object_name(name)
            } else {
                name.to_string()
            };
            doc.scene.object_mut(id).map_err(|error| error.to_string())?.name = unique.clone();
            Ok(json!({ "object": id, "name": unique }))
        }
        "object.parent" => {
            let parent = one(doc, params, "parent")?;
            let children = match params::objects(&doc.scene, params, "objects")? {
                Some(ids) => ids,
                None => doc.session.editor.selected.iter().copied().filter(|id| *id != parent).collect(),
            };
            if children.is_empty() {
                return Err("not-allowed: select the children and then the parent".into());
            }
            for child in &children {
                doc.scene.set_parent(*child, Some(parent)).map_err(|error| format!("invalid-params: {error}"))?;
            }
            Ok(json!({ "parent": parent, "children": children }))
        }
        "object.clearParent" => {
            let ids = chosen(doc, params)?;
            for id in &ids {
                doc.scene.set_parent(*id, None).map_err(|error| error.to_string())?;
            }
            Ok(json!({ "objects": ids }))
        }
        "object.hide" => {
            let ids = chosen(doc, params)?;
            for id in &ids {
                doc.scene.object_mut(*id).map_err(|error| error.to_string())?.hidden = true;
            }
            doc.session.editor.selected.retain(|id| !ids.contains(id));
            if doc.session.editor.active.is_some_and(|id| ids.contains(&id)) {
                doc.session.editor.active = None;
            }
            Ok(json!({ "hidden": ids }))
        }
        "object.reveal" => {
            let only = params::objects(&doc.scene, params, "objects")?;
            let mut shown = Vec::new();
            for object in &mut doc.scene.objects {
                if object.hidden && only.as_ref().is_none_or(|ids| ids.contains(&object.id)) {
                    object.hidden = false;
                    shown.push(object.id);
                }
            }
            if shown.is_empty() {
                return Err("not-allowed: nothing is hidden".into());
            }
            doc.session.editor.selected = shown.clone();
            doc.session.editor.active = shown.last().copied();
            Ok(json!({ "shown": shown }))
        }
        "object.shadeSmooth" | "object.shadeSmoothByAngle" | "object.shadeFlat" => {
            let smooth = command != "object.shadeFlat";
            let angle = if command == "object.shadeSmoothByAngle" {
                let degrees = params::number(params, "angle")?.unwrap_or(30.0);
                if !(degrees.is_finite() && (0.0..=180.0).contains(&degrees)) {
                    return Err(bad("`angle` must be between 0 and 180 degrees"));
                }
                Some(degrees.to_radians().min(std::f32::consts::PI))
            } else {
                None
            };
            // Named objects win; in edit mode the edited mesh is the default.
            let ids = match (doc.session.mode, params::objects(&doc.scene, params, "objects")?) {
                (_, Some(ids)) => ids,
                (Mode::Edit(id), None) => vec![id],
                (Mode::Object, None) => chosen(doc, params)?,
            };
            let mut changed = Vec::new();
            for id in ids {
                if let Some(mesh) = doc.scene.object_mut(id).map_err(|error| error.to_string())?.mesh_mut() {
                    mesh.smooth = smooth;
                    mesh.smooth_angle = angle;
                    changed.push(id);
                }
            }
            if changed.is_empty() {
                return Err("not-allowed: no mesh is selected".into());
            }
            Ok(json!({ "objects": changed }))
        }
        "object.join" => {
            let target = active_mesh(doc).ok_or("not-allowed: the active object must be a mesh")?;
            let others: Vec<Id> = doc
                .session
                .editor
                .selected
                .iter()
                .copied()
                .filter(|id| *id != target && doc.scene.object(*id).is_ok_and(|object| object.mesh().is_some()))
                .collect();
            if others.is_empty() {
                return Err("not-allowed: select other meshes, then the one to join them into".into());
            }
            let inverse = doc.scene.world_matrix(target).map_err(|error| error.to_string())?.inverse();
            let mut parts = Vec::new();
            for id in &others {
                let world = doc.scene.world_matrix(*id).map_err(|error| error.to_string())?;
                if let Some(mesh) = doc.scene.object(*id).ok().and_then(|object| object.mesh()) {
                    let mut part = mesh.clone();
                    part.transform(inverse * world);
                    parts.push(part);
                }
            }
            if let Some(mesh) = doc.scene.object_mut(target).map_err(|error| error.to_string())?.mesh_mut() {
                for part in &parts {
                    mesh.append(part);
                }
            }
            doc.scene.remove_objects(&others).map_err(|error| error.to_string())?;
            select_only(doc, vec![target]);
            Ok(json!({ "object": target, "joined": others }))
        }
        "object.setCamera" => {
            let id = one(doc, params, "object")?;
            if !matches!(doc.scene.object(id).map(|object| &object.data), Ok(ObjectData::Camera(_))) {
                return Err(bad("that object is not a camera"));
            }
            doc.scene.camera = Some(id);
            Ok(json!({ "camera": id }))
        }
        "object.lookAt" => {
            let id = one(doc, params, "object")?;
            let target = params::vec3(params, "target")?.ok_or_else(|| bad("`target` is required"))?;
            let world = doc.scene.world_matrix(id).map_err(|error| error.to_string())?;
            let from = world.transform_point3(Vec3::ZERO);
            let rotation = look_rotation(from, target).ok_or_else(|| bad("the target is where the object is"))?;
            // The turn expressed in the parent's frame (not through the object's own constraints);
            // the location and scale fields stay (a constraint may move it: it is aimed from where
            // it is seen).
            let parent_world = doc.scene.parent_frame(id).map_err(|error| error.to_string())?;
            let local = parent_world.inverse() * Mat4::from_rotation_translation(glam::Quat::from_mat3(&euler_matrix(rotation)), from);
            let object = doc.scene.object_mut(id).map_err(|error| error.to_string())?;
            let (scale, location) = (object.scale, object.location);
            object.set_local_matrix(local);
            object.scale = scale;
            object.location = location;
            Ok(json!({ "object": id, "rotation": rotation.to_array().map(f32::to_degrees) }))
        }
        "mesh.extrude" if crate::commands_model::extrudes_loose(doc) => crate::commands_model::extrude_loose(doc, params),
        "mesh.extrude" => {
            let distance = params::number(params, "distance")?.unwrap_or(1.0);
            let offset = params::vec3(params, "offset")?.unwrap_or(Vec3::ZERO);
            let moved = edit_mesh(doc, |mesh, vertices| {
                let faces = selected_faces(mesh, vertices);
                if faces.is_empty() {
                    return Err("not-allowed: select faces to extrude".into());
                }
                ops::extrude_region(mesh, &faces, distance, offset).map_err(|error| error.to_string())?;
                Ok(ops::face_vertices(mesh, &faces.into_iter().collect()))
            })?;
            doc.session.vertices = moved;
            Ok(json!({ "vertices": doc.session.vertices.len() }))
        }
        "mesh.inset" => {
            let thickness = params::number(params, "thickness")?.unwrap_or(0.1);
            let depth = params::number(params, "depth")?.unwrap_or(0.0);
            let inner = edit_mesh(doc, |mesh, vertices| {
                let faces = selected_faces(mesh, vertices);
                if faces.is_empty() {
                    return Err("not-allowed: select faces to inset".into());
                }
                ops::inset(mesh, &faces, thickness).map_err(|error| error.to_string())?;
                // Each inset face (it keeps its index, and corners of its own) moves along its
                // normal.
                if depth != 0.0 {
                    for face in &faces {
                        let Some(corners) = mesh.faces.get(*face as usize).cloned() else { continue };
                        let along = mesh.face_normal(&corners).normalize_or_zero() * depth;
                        for corner in corners {
                            if let Some(point) = mesh.positions.get_mut(corner as usize) {
                                *point += along;
                            }
                        }
                    }
                }
                Ok(ops::face_vertices(mesh, &faces.into_iter().collect()))
            })?;
            doc.session.vertices = inner;
            Ok(json!({ "vertices": doc.session.vertices.len() }))
        }
        "mesh.bevel" => {
            let width = params::number(params, "width")?.unwrap_or(0.1);
            let segments = params::integer(params, "segments")?.unwrap_or(1);
            let made = edit_mesh(doc, |mesh, vertices| {
                let edges: Vec<(u32, u32)> =
                    mesh.edges().into_iter().filter(|[a, b]| vertices.contains(a) && vertices.contains(b)).map(|[a, b]| (a, b)).collect();
                if edges.is_empty() {
                    return Err("not-allowed: select edges to bevel".into());
                }
                let faces = ops::bevel_edges(mesh, &edges, width, segments).map_err(|error| error.to_string())?;
                Ok(ops::face_vertices(mesh, &faces.into_iter().collect()))
            })?;
            doc.session.vertices = made;
            Ok(json!({ "vertices": doc.session.vertices.len() }))
        }
        "mesh.subdivide" => {
            let made = edit_mesh(doc, |mesh, vertices| {
                let faces = selected_faces(mesh, vertices);
                if faces.is_empty() {
                    return Err("not-allowed: select faces to subdivide".into());
                }
                let made = ops::subdivide(mesh, &faces).map_err(|error| error.to_string())?;
                Ok(ops::face_vertices(mesh, &made.into_iter().collect()))
            })?;
            doc.session.vertices = made;
            Ok(json!({ "vertices": doc.session.vertices.len() }))
        }
        "mesh.loopCut" => {
            let edge = params::indices(params, "edge")?.ok_or_else(|| bad("`edge` is required"))?;
            let [a, b] = edge.as_slice() else { return Err(bad("`edge` is [vertex, vertex]")) };
            let (a, b) = (*a, *b);
            let cuts = params::integer(params, "cuts")?.unwrap_or(1);
            let factor = params::number(params, "factor")?.unwrap_or(0.5);
            let made = edit_mesh(doc, |mesh, _| ops::loop_cut(mesh, (a, b), cuts, factor).map_err(|error| error.to_string()))?;
            doc.session.vertices = made.iter().copied().collect();
            Ok(json!({ "vertices": made }))
        }
        "mesh.merge" => {
            let merged = edit_mesh(doc, |mesh, vertices| {
                ops::merge_at_center(mesh, &vertices.iter().copied().collect::<Vec<_>>()).map_err(|error| error.to_string())
            })?;
            doc.session.vertices = BTreeSet::from([merged]);
            Ok(json!({ "vertex": merged }))
        }
        "mesh.deleteFaces" => {
            edit_mesh(doc, |mesh, vertices| {
                let faces = selected_faces(mesh, vertices);
                if faces.is_empty() {
                    return Err("not-allowed: no face is selected".into());
                }
                ops::delete_faces(mesh, &faces).map_err(|error| error.to_string())
            })?;
            doc.session.vertices.clear();
            Ok(json!({ "deleted": "faces" }))
        }
        "mesh.deleteVertices" => {
            edit_mesh(doc, |mesh, vertices| {
                ops::delete_vertices(mesh, &vertices.iter().copied().collect::<Vec<_>>()).map_err(|error| error.to_string())
            })?;
            doc.session.vertices.clear();
            Ok(json!({ "deleted": "vertices" }))
        }
        "mesh.fill" => {
            let given = params::indices(params, "vertices")?;
            let face = edit_mesh(doc, |mesh, vertices| {
                let corners: Vec<u32> = match given {
                    Some(list) => list,
                    None => order_around(mesh, vertices),
                };
                if corners.len() < 3 {
                    return Err("not-allowed: a face needs at least three vertices".into());
                }
                // Through the carry every edit takes: the new face gets texture coordinates (and
                // a face set) from the faces beside it, so a mesh with them can still be filled.
                let mut candidate = mesh.clone();
                kition_scene_model::uv::with_uvs(&mut candidate, |candidate| candidate.faces.push(corners));
                candidate.validate().map_err(|error| bad(error.to_string().replace("invalid-file: ", "")))?;
                *mesh = candidate;
                Ok(mesh.faces.len() - 1)
            })?;
            Ok(json!({ "face": face }))
        }
        "mesh.flip" => {
            edit_mesh(doc, |mesh, vertices| ops::flip(mesh, &selected_faces(mesh, vertices)).map_err(|error| error.to_string()))?;
            Ok(json!({ "flipped": true }))
        }
        "mesh.recalculateNormals" => match doc.session.mode {
            // Named meshes are recalculated whole, the edited one among them too.
            Mode::Edit(_) if params.get("objects").is_none_or(Value::is_null) => {
                edit_mesh(doc, |mesh, _| {
                    ops::recalculate_outside(mesh);
                    Ok(())
                })?;
                Ok(json!({ "recalculated": true }))
            }
            _ => {
                let ids = chosen(doc, params)?;
                for id in &ids {
                    let object = doc.scene.object_mut(*id).map_err(|error| error.to_string())?;
                    let name = object.name.clone();
                    let mesh = object.mesh_mut().ok_or_else(|| format!("unsupported: {name} is not a mesh"))?;
                    ops::recalculate_outside(mesh);
                }
                Ok(json!({ "recalculated": true, "objects": ids }))
            }
        },
        "mesh.translate" => {
            let offset = params::vec3(params, "offset")?.ok_or_else(|| bad("`offset` is required"))?;
            crate::commands_model::follow(doc, &|point| point + offset)?;
            edit_mesh(doc, |mesh, vertices| {
                ops::translate(mesh, &vertices.iter().copied().collect::<Vec<_>>(), offset).map_err(|error| error.to_string())
            })?;
            Ok(json!({ "moved": doc.session.vertices.len() }))
        }
        "mesh.scale" => {
            let factor = factor3(params)?;
            let pivot = params::vec3(params, "pivot")?;
            crate::commands_model::follow_scale(doc, factor, pivot)?;
            edit_mesh(doc, |mesh, vertices| {
                let list: Vec<u32> = vertices.iter().copied().collect();
                let centre = pivot.unwrap_or_else(|| {
                    list.iter().filter_map(|vertex| mesh.positions.get(*vertex as usize)).copied().sum::<Vec3>() / list.len().max(1) as f32
                });
                ops::scale(mesh, &list, centre, factor).map_err(|error| error.to_string())
            })?;
            Ok(json!({ "scaled": doc.session.vertices.len() }))
        }
        "mesh.setGeometry" => {
            // A named object wins; in edit mode the edited mesh is the default.
            let id = match (doc.session.mode, params.get("object").filter(|value| !value.is_null())) {
                (_, Some(value)) => params::resolve(&doc.scene, value)?,
                (Mode::Edit(id), None) => id,
                (Mode::Object, None) => one(doc, params, "object")?,
            };
            let positions = params.get("positions").and_then(Value::as_array).ok_or_else(|| bad("`positions` is a list of [x, y, z]"))?;
            let faces = params.get("faces").and_then(Value::as_array).ok_or_else(|| bad("`faces` is a list of vertex lists"))?;
            if positions.len() > 500_000 || faces.len() > 1_000_000 {
                return Err(bad("the mesh is too large"));
            }
            let mut mesh = kition_scene_model::Mesh::default();
            for (index, point) in positions.iter().enumerate() {
                mesh.positions.push(params::vec3(&json!({ "p": point }), "p")?.ok_or_else(|| bad(format!("position {index} is not [x, y, z]")))?);
            }
            for (index, face) in faces.iter().enumerate() {
                mesh.faces
                    .push(params::indices(&json!({ "f": face }), "f")?.ok_or_else(|| bad(format!("face {index} is not a list of vertex indices")))?);
            }
            mesh.validate().map_err(|error| bad(error.to_string().replace("invalid-file: ", "")))?;
            let object = doc.scene.object_mut(id).map_err(|error| error.to_string())?;
            let current = object.mesh_mut().ok_or_else(|| bad("that object is not a mesh"))?;
            mesh.smooth = params::boolean(params, "smooth")?.unwrap_or(current.smooth);
            mesh.smooth_angle = current.smooth_angle.filter(|_| mesh.smooth);
            *current = mesh;
            // The edit selection indexes the edited mesh's old vertices.
            if doc.session.mode == Mode::Edit(id) {
                doc.session.vertices.clear();
            }
            Ok(json!({ "object": id, "vertices": positions.len(), "faces": faces.len() }))
        }
        "modifier.add" => {
            // A named object wins; in edit mode the edited mesh is the default.
            let id = match (doc.session.mode, params.get("object").filter(|value| !value.is_null())) {
                (_, Some(value)) => params::resolve(&doc.scene, value)?,
                (Mode::Edit(id), None) => id,
                (Mode::Object, None) => active_mesh(doc).ok_or("not-allowed: the active object must be a mesh")?,
            };
            let kind_name = params::string(params, "type")?.ok_or_else(|| bad("`type` is required"))?;
            let kind = ModifierKind::default_of(kind_name).map_err(|error| error.to_string())?;
            let kind = configure(&doc.scene, id, &kind, params)?;
            let object = doc.scene.object_mut(id).map_err(|error| error.to_string())?;
            if object.mesh().is_none() {
                return Err(bad(format!("{} is not a mesh", object.name)));
            }
            if matches!(kind, ModifierKind::Multires { .. })
                && object.modifiers.iter().any(|other| matches!(other.kind, ModifierKind::Multires { .. }))
            {
                return Err(bad(format!("{} already has a Multires modifier", object.name)));
            }
            let mut modifier = Modifier::new(kind);
            modifier.name = kition_scene_model::scene::unique_name(&modifier.name, |name| object.modifiers.iter().any(|other| other.name == name));
            let name = modifier.name.clone();
            object.modifiers.push(modifier);
            Ok(json!({ "object": id, "index": object.modifiers.len() - 1, "name": name }))
        }
        "modifier.set" => {
            let (id, index) = find_modifier(doc, params)?;
            let enabled = params::boolean(params, "enabled")?;
            let rename = params::string(params, "rename")?.map(str::to_string);
            let configured = {
                let current = doc.scene.object(id).map_err(|error| error.to_string())?.modifiers.get(index).ok_or("not-found: the modifier")?;
                configure(&doc.scene, id, &current.kind, params)?
            };
            let object = doc.scene.object_mut(id).map_err(|error| error.to_string())?;
            let taken = rename
                .as_ref()
                .is_some_and(|name| object.modifiers.iter().enumerate().any(|(other, modifier)| other != index && &modifier.name == name));
            let modifier = object.modifiers.get_mut(index).ok_or("not-found: the modifier")?;
            modifier.kind = configured;
            if let Some(enabled) = enabled {
                modifier.enabled = enabled;
            }
            if let Some(name) = rename {
                if taken {
                    return Err(bad(format!("a modifier is already called {name}")));
                }
                modifier.name = name;
            }
            Ok(json!({ "object": id, "index": index }))
        }
        "modifier.remove" => {
            let (id, index) = find_modifier(doc, params)?;
            let object = doc.scene.object_mut(id).map_err(|error| error.to_string())?;
            if index < object.modifiers.len() {
                object.modifiers.remove(index);
            }
            Ok(json!({ "object": id, "removed": index }))
        }
        "modifier.apply" => {
            let (id, index) = find_modifier(doc, params)?;
            let snapshot = doc.scene.clone();
            let object = snapshot.object(id).map_err(|error| error.to_string())?;
            let modifier = object.modifiers.get(index).ok_or("not-found: the modifier")?;
            // A modifier applies to the mesh as the ones above it leave it.
            let mut mesh = object.mesh().cloned().ok_or("unsupported: not a mesh")?;
            let mut stack = kition_scene_model::modifiers::Stack::default();
            for earlier in object.modifiers.iter().take(index).filter(|modifier| modifier.enabled) {
                mesh = stack.apply(earlier, mesh, object, &snapshot, 0);
            }
            mesh = stack.apply(modifier, mesh, object, &snapshot, 0);
            let object = doc.scene.object_mut(id).map_err(|error| error.to_string())?;
            object.modifiers.drain(..=index);
            if let Some(current) = object.mesh_mut() {
                let (smooth, angle) = (current.smooth, current.smooth_angle);
                *current = mesh;
                current.smooth = smooth;
                current.smooth_angle = angle;
            }
            Ok(json!({ "object": id }))
        }
        "modifier.move" => {
            let (id, index) = find_modifier(doc, params)?;
            let to = params::integer(params, "to")?.ok_or_else(|| bad("`to` is required"))? as usize;
            let object = doc.scene.object_mut(id).map_err(|error| error.to_string())?;
            if to >= object.modifiers.len() {
                return Err(bad(format!("`to` must be below {}", object.modifiers.len())));
            }
            let modifier = object.modifiers.remove(index);
            object.modifiers.insert(to, modifier);
            Ok(json!({ "object": id, "index": to }))
        }
        "material.new" => {
            let name = params::string(params, "name")?.unwrap_or("Material").to_string();
            let id = doc.scene.add_material(&name);
            let mut settings = params.clone();
            if let Some(object) = settings.as_object_mut() {
                object.remove("name");
            }
            set_material(&mut doc.scene, id, &settings)?;
            let mut assigned = Vec::new();
            if params::boolean(params, "assign")?.unwrap_or(true) {
                let targets: Vec<Id> = match doc.session.mode {
                    Mode::Edit(object) => vec![object],
                    Mode::Object => doc.session.editor.selected.clone(),
                };
                for object in targets {
                    if let Ok(found) = doc.scene.object_mut(object)
                        && found.has_geometry()
                    {
                        found.material = Some(id);
                        assigned.push(object);
                    }
                }
            }
            let name = doc.scene.material(id).map(|material| material.name.clone()).unwrap_or_default();
            Ok(json!({ "material": id, "name": name, "assigned": assigned }))
        }
        "material.set" => {
            let id = material_ref(&doc.scene, params)?;
            set_material(&mut doc.scene, id, params)?;
            Ok(json!({ "material": id }))
        }
        "material.assign" => {
            let id = material_ref(&doc.scene, params)?;
            let ids = chosen(doc, params)?;
            for object in &ids {
                doc.scene.object_mut(*object).map_err(|error| error.to_string())?.material = Some(id);
            }
            Ok(json!({ "material": id, "objects": ids }))
        }
        "material.delete" => {
            let id = material_ref(&doc.scene, params)?;
            let users: Vec<Id> = doc.scene.objects.iter().filter(|object| object.material == Some(id)).map(|object| object.id).collect();
            if !users.is_empty() && !params::boolean(params, "force")?.unwrap_or(false) {
                let name = doc.scene.material(id).map_err(|error| error.to_string())?.name.clone();
                return Err(format!("not-allowed: {name} is used by {} object(s); pass force: true to delete it anyway", users.len()));
            }
            doc.scene.remove_material(id).map_err(|error| error.to_string())?;
            Ok(json!({ "material": id, "released": users }))
        }
        "material.clear" => {
            let ids = chosen(doc, params)?;
            for object in &ids {
                doc.scene.object_mut(*object).map_err(|error| error.to_string())?.material = None;
            }
            Ok(json!({ "objects": ids }))
        }
        "light.set" => {
            let id = one(doc, params, "object")?;
            let kind = params::string(params, "kind")?;
            let color = params::color(params, "color")?;
            let power = params::number(params, "power")?;
            let radius = params::number(params, "radius")?;
            let spot = params::number(params, "spotSize")?;
            let object = doc.scene.object_mut(id).map_err(|error| error.to_string())?;
            let ObjectData::Light(light) = &mut object.data else { return Err(bad(format!("{} is not a light", object.name))) };
            if let Some(kind) = kind {
                light.kind = match kind {
                    "point" => LightKind::Point,
                    "sun" => LightKind::Sun,
                    "spot" => LightKind::Spot,
                    "area" => LightKind::Area,
                    other => return Err(bad(format!("unknown light kind {other}"))),
                };
            }
            if let Some([r, g, b, _]) = color {
                light.color = Vec3::new(r, g, b);
            }
            if let Some(power) = power {
                light.power = power.max(0.0);
            }
            if let Some(radius) = radius {
                light.radius = radius.max(0.0);
            }
            if let Some(spot) = spot {
                light.spot_size = spot.clamp(1.0, 180.0).to_radians();
            }
            Ok(json!({ "object": id }))
        }
        "camera.set" => {
            let id = one(doc, params, "object")?;
            let lens = params::number(params, "lens")?;
            let sensor = params::number(params, "sensor")?;
            let sensor_height = params::number(params, "sensorHeight")?;
            if sensor_height.is_some_and(|height| !(1.0..=200.0).contains(&height)) {
                return Err(bad("`sensorHeight` must be between 1 and 200 millimetres"));
            }
            let sensor_fit = params
                .get("sensorFit")
                .map(|value| {
                    serde_json::from_value::<kition_scene_model::scene::SensorFit>(value.clone())
                        .map_err(|_| bad("`sensorFit` must be auto, horizontal, or vertical"))
                })
                .transpose()?;
            let clip_start = params::number(params, "clipStart")?;
            let clip_end = params::number(params, "clipEnd")?;
            let orthographic = params::boolean(params, "orthographic")?;
            let ortho_scale = params::number(params, "orthoScale")?;
            let depth_of_field = params::boolean(params, "depthOfField")?;
            let f_stop = params::number(params, "fStop")?;
            let focus_distance = params::number(params, "focusDistance")?;
            let blades = params::integer(params, "blades")?;
            let blade_rotation = params::number(params, "bladeRotation")?;
            let ratio = params::number(params, "ratio")?;
            if blades.is_some_and(|blades| blades == 1 || blades == 2 || blades > kition_scene_model::scene::MAX_BLADES) {
                return Err(bad(format!("`blades` is 0 (round) or 3 to {}", kition_scene_model::scene::MAX_BLADES)));
            }
            let focus_object = match params.get("focusObject") {
                None => None,
                Some(Value::Null) => Some(None),
                Some(value) => Some(Some(params::resolve(&doc.scene, value)?)),
            };
            if focus_object == Some(Some(id)) {
                return Err(bad("a camera cannot focus on itself"));
            }
            let object = doc.scene.object_mut(id).map_err(|error| error.to_string())?;
            let ObjectData::Camera(camera) = &mut object.data else { return Err(bad(format!("{} is not a camera", object.name))) };
            if let Some(lens) = lens {
                camera.lens = lens.clamp(1.0, 5000.0);
            }
            if let Some(sensor) = sensor {
                camera.sensor = sensor.clamp(1.0, 200.0);
            }
            if let Some(height) = sensor_height {
                camera.sensor_height = height;
            }
            if let Some(fit) = sensor_fit {
                camera.sensor_fit = fit;
            }
            if let Some(start) = clip_start {
                camera.clip_start = start.max(1e-4);
            }
            if let Some(end) = clip_end {
                camera.clip_end = end.max(camera.clip_start * 2.0);
            }
            if let Some(orthographic) = orthographic {
                camera.orthographic = orthographic;
            }
            if let Some(scale) = ortho_scale {
                camera.ortho_scale = scale.max(1e-3);
            }
            if let Some(on) = depth_of_field {
                camera.dof.enabled = on;
            }
            if let Some(f_stop) = f_stop {
                camera.dof.f_stop = f_stop.clamp(0.01, 1000.0);
            }
            if let Some(distance) = focus_distance {
                camera.dof.focus_distance = distance.clamp(1e-3, 1e6);
            }
            if let Some(focus) = focus_object {
                camera.dof.focus_object = focus;
            }
            if let Some(blades) = blades {
                camera.dof.blades = blades;
            }
            if let Some(rotation) = blade_rotation {
                camera.dof.rotation = rotation.to_radians().rem_euclid(std::f32::consts::TAU);
            }
            if let Some(ratio) = ratio {
                camera.dof.ratio = ratio.clamp(0.01, 100.0);
            }
            Ok(json!({ "object": id }))
        }
        "world.set" => {
            if let Some([r, g, b, _]) = params::color(params, "color")? {
                doc.scene.world.color = Vec3::new(r, g, b);
            }
            if let Some(strength) = params::number(params, "strength")? {
                doc.scene.world.strength = strength.max(0.0);
            }
            if let Some(rotation) = params::number(params, "rotation")? {
                doc.scene.world.rotation = rotation.to_radians().rem_euclid(std::f32::consts::TAU);
            }
            if let Some(value) = params.get("volume") {
                doc.scene.world.volume = crate::commands_nodes::shader_ref(&doc.scene, value)?;
            }
            Ok(json!({ "world": true }))
        }
        "render.set" => {
            if let Some(width) = params::integer(params, "width")? {
                doc.scene.render.width = width;
            }
            if let Some(height) = params::integer(params, "height")? {
                doc.scene.render.height = height;
            }
            let blur = &mut doc.scene.render.motion_blur;
            if let Some(on) = params::boolean(params, "motionBlur")? {
                blur.enabled = on;
            }
            if let Some(shutter) = params::number(params, "shutter")? {
                if !(shutter.is_finite() && shutter > 0.0) {
                    return Err(bad("`shutter` must be more than 0 frames"));
                }
                blur.shutter = shutter.min(kition_scene_model::MotionBlur::MAX_SHUTTER);
            }
            if let Some(position) = params::string(params, "shutterPosition")? {
                blur.position = match position {
                    "center" => kition_scene_model::ShutterPosition::Center,
                    "start" => kition_scene_model::ShutterPosition::Start,
                    "end" => kition_scene_model::ShutterPosition::End,
                    other => return Err(bad(format!("`shutterPosition` is center, start, or end, not {other}"))),
                };
            }
            let occlusion = &mut doc.scene.render.ambient_occlusion;
            if let Some(on) = params::boolean(params, "ao")? {
                occlusion.enabled = on;
            }
            match params.get("aoDistance") {
                Some(Value::Null) => occlusion.distance = None,
                Some(_) => {
                    let distance = params::number(params, "aoDistance")?.unwrap_or(0.0);
                    let most = kition_scene_model::AmbientOcclusion::MAX_DISTANCE;
                    if !(distance.is_finite() && distance > 0.0 && distance <= most) {
                        return Err(bad(format!("`aoDistance` must be more than 0 and at most {most} metres")));
                    }
                    occlusion.distance = Some(distance);
                }
                None => {}
            }
            if let Some(strength) = params::number(params, "aoStrength")? {
                let most = kition_scene_model::AmbientOcclusion::MAX_STRENGTH;
                if !(strength.is_finite() && (0.0..=most).contains(&strength)) {
                    return Err(bad(format!("`aoStrength` must be between 0 and {most}")));
                }
                occlusion.strength = strength;
            }
            let display = &mut doc.scene.render.color_management;
            if let Some(view) = params::string(params, "view")? {
                display.view = kition_scene_model::color::ViewTransform::from_name(view)
                    .ok_or_else(|| bad(format!("`view` is one of {}, not {view}", kition_scene_model::color::ViewTransform::NAMES.join(", "))))?;
            }
            if let Some(look) = params::string(params, "look")? {
                display.look = kition_scene_model::color::Look::from_name(look)
                    .ok_or_else(|| bad(format!("`look` is one of {}, not {look}", kition_scene_model::color::Look::NAMES.join(", "))))?;
            }
            if let Some(exposure) = params::number(params, "exposure")? {
                let most = kition_scene_model::color::ColorManagement::MAX_EXPOSURE;
                if !(exposure.is_finite() && exposure.abs() <= most) {
                    return Err(bad(format!("`exposure` must be between -{most} and {most} stops")));
                }
                display.exposure = exposure;
            }
            let max = kition_scene_model::scene::MAX_RENDER_SIDE;
            if !(1..=max).contains(&doc.scene.render.width) || !(1..=max).contains(&doc.scene.render.height) {
                return Err(bad(format!("the render size must be between 1 and {max} on each side")));
            }
            Ok(json!({ "width": doc.scene.render.width, "height": doc.scene.render.height }))
        }
        "file.import" if blend_import(params)? => {
            let bytes = match (params::string(params, "path")?, params::string(params, "base64")?) {
                (Some(path), _) => kition_kit::host::read(path).map_err(|error| format!("unavailable: {error}"))?,
                (None, Some(text)) => unbase64(text)?,
                (None, None) => return Err(bad("give `path` or `base64`")),
            };
            let ids = crate::commands_model::import_blend(doc, &bytes)?;
            select_only(doc, ids.clone());
            Ok(json!({ "objects": ids }))
        }
        "file.import" => {
            let (bytes, format) = match (params::string(params, "path")?, params::string(params, "base64")?) {
                (Some(path), _) => {
                    let extension = path.rsplit_once('.').map(|(_, extension)| extension.to_ascii_lowercase()).unwrap_or_default();
                    let format = crate::formats::Exchange::from_extension(&extension)
                        .ok_or_else(|| bad(format!("the file must be one of .{}", crate::formats::IMPORTS.join(", ."))))?;
                    (kition_kit::host::read(path).map_err(|error| format!("unavailable: {error}"))?, format)
                }
                (None, Some(text)) => {
                    let name = params::string(params, "format")?.ok_or_else(|| bad("`format` is required with `base64`"))?;
                    let format = crate::formats::Exchange::from_extension(name)
                        .filter(|_| crate::formats::IMPORTS.contains(&name))
                        .ok_or_else(|| bad(format!("`format` must be {}", crate::formats::listed(crate::formats::IMPORTS))))?;
                    (unbase64(text)?, format)
                }
                (None, None) => return Err(bad("give `path` or `base64`")),
            };
            let imported = format.import(&bytes)?;
            let ids = kition_scene_exchange::place(&mut doc.scene, imported).map_err(|error| error.to_string())?;
            select_only(doc, ids.clone());
            Ok(json!({ "objects": ids }))
        }
        other => crate::commands_model::run_change(doc, other, params),
    }
}

/// Moves objects by a world-space offset (children that move with a selected parent are left
/// alone).
pub fn translate_objects(scene: &mut Scene, ids: &[Id], offset: Vec3) -> Result<(), String> {
    for id in top_level(scene, ids) {
        // The parent's frame (never the object's own constraints or a baked pose).
        let parent_world = scene.parent_frame(id).map_err(|error| error.to_string())?;
        let local_offset = parent_world.inverse().transform_vector3(offset);
        scene.object_mut(id).map_err(|error| error.to_string())?.location += local_offset;
    }
    Ok(())
}

/// Turns objects about their own origins by a world-space rotation.
pub fn rotate_objects(scene: &mut Scene, ids: &[Id], rotation: Mat3) -> Result<(), String> {
    for id in top_level(scene, ids) {
        let parent_world = scene.parent_frame(id).map_err(|error| error.to_string())?;
        let object = scene.object(id).map_err(|error| error.to_string())?;
        let parent_rotation = Mat3::from_mat4(parent_world);
        let (scale, location) = (object.scale, object.location);
        // In the parent's axes the turn is P⁻¹ R P.
        let local = parent_rotation.inverse() * rotation * parent_rotation * euler_matrix(object.rotation);
        let object = scene.object_mut(id).map_err(|error| error.to_string())?;
        object.rotation = matrix_euler(orthonormal(local));
        object.scale = scale;
        object.location = location;
    }
    Ok(())
}

fn orthonormal(matrix: Mat3) -> Mat3 {
    let x = matrix.x_axis.normalize_or_zero();
    let y = (matrix.y_axis - x * x.dot(matrix.y_axis)).normalize_or_zero();
    Mat3::from_cols(x, y, x.cross(y))
}

/// A transform command's result: the objects it moved and, when others moved in the world with
/// them (their descendants, objects constrained to them), how many (`carried`, C8-scene-2): their
/// own fields did not change, so the receipt does not name them.
fn moved(scene: &Scene, ids: Vec<Id>) -> Value {
    match crate::place::carried(scene, &ids) {
        0 => json!({ "objects": ids }),
        count => {
            json!({ "objects": ids, "carried": { "count": count, "says": "objects whose world transform moved with these (children, objects constrained to them); their own fields are unchanged" } })
        }
    }
}

/// The ids whose parents are not also among them.
pub(crate) fn top_level(scene: &Scene, ids: &[Id]) -> Vec<Id> {
    ids.iter().copied().filter(|id| !ids.iter().any(|other| other != id && scene.is_ancestor(*other, *id))).collect()
}

/// Moves `change` from an object into its mesh: the mesh is transformed by `change`, and the
/// children's parent inverses compensate so nothing moves in the world.
fn rebase(scene: &mut Scene, id: Id, change: Mat4) -> Result<(), String> {
    let inverse = change.inverse();
    if !inverse.is_finite() {
        return Err("invalid-params: the transform cannot be applied (a scale is zero)".into());
    }
    if let Some(mesh) = scene.object_mut(id).map_err(|error| error.to_string())?.mesh_mut() {
        mesh.transform(change);
    }
    for child in scene.objects.iter_mut().filter(|object| object.parent == Some(id)) {
        child.parent_inverse = change * child.parent_inverse;
    }
    Ok(())
}

/// Selected vertices in order around their centre, for filling a face.
fn order_around(mesh: &kition_scene_model::Mesh, vertices: &BTreeSet<u32>) -> Vec<u32> {
    let points: Vec<(u32, Vec3)> = vertices.iter().filter_map(|vertex| mesh.positions.get(*vertex as usize).map(|point| (*vertex, *point))).collect();
    if points.len() < 3 {
        return points.into_iter().map(|(vertex, _)| vertex).collect();
    }
    let centre = points.iter().map(|(_, point)| *point).sum::<Vec3>() / points.len() as f32;
    let normal = kition_scene_model::mesh::newell_normal(&points.iter().map(|(_, point)| *point).collect::<Vec<_>>());
    let normal = if normal == Vec3::ZERO { Vec3::Z } else { normal };
    let (u, v) = normal.any_orthonormal_pair();
    let mut ordered = points;
    ordered.sort_by(|(_, a), (_, b)| {
        let angle = |point: &Vec3| (point.dot(v) - centre.dot(v)).atan2(point.dot(u) - centre.dot(u));
        angle(a).total_cmp(&angle(b))
    });
    ordered.into_iter().map(|(vertex, _)| vertex).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips() {
        for bytes in [&b""[..], b"a", b"ab", b"abc", b"abcd", &[0, 255, 128, 7, 9]] {
            assert_eq!(unbase64(&base64(bytes)).unwrap(), bytes);
        }
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
    }

    #[test]
    fn every_schema_is_well_formed() {
        for spec in SPECS {
            kition_kit::schema::validate(&(spec.schema)()).unwrap_or_else(|error| panic!("{}: {error}", spec.id));
        }
    }

    #[test]
    fn look_rotation_points_minus_z_at_the_target() {
        let from = Vec3::new(5.0, -5.0, 3.0);
        let rotation = look_rotation(from, Vec3::ZERO).unwrap();
        let forward = euler_matrix(rotation) * Vec3::NEG_Z;
        assert!(forward.abs_diff_eq((Vec3::ZERO - from).normalize(), 1e-5));
        let up = euler_matrix(rotation) * Vec3::Y;
        assert!(up.z > 0.0);
    }
}
