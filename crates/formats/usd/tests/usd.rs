//! USDA written and read back, stage conventions (up axis, units, transform ops), packages,
//! and damaged or hostile files, which are refused with an error and never a panic.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_scene_exchange::Imported;
use kition_scene_model::glam::Vec3;
use kition_scene_model::{ObjectData, Scene, SceneError};

fn placed(imported: Imported) -> Scene {
    let mut scene = Scene::empty();
    kition_scene_exchange::place(&mut scene, imported).unwrap();
    scene
}

fn by_name<'a>(scene: &'a Scene, name: &str) -> &'a kition_scene_model::Object {
    scene.objects.iter().find(|object| object.name == name).unwrap_or_else(|| panic!("no {name}"))
}

#[test]
fn the_starter_scene_reads_back() {
    let scene = Scene::starter();
    let text = kition_scene_usd::write(&scene, None).unwrap();
    let back = placed(kition_scene_usd::read(&text).unwrap());
    assert_eq!(back.objects.len(), scene.objects.len());
    for object in &scene.objects {
        let other = by_name(&back, &object.name);
        assert_eq!(object.data.kind(), other.data.kind());
        let (a, b) = (scene.world_matrix(object.id).unwrap(), back.world_matrix(other.id).unwrap());
        assert!(a.abs_diff_eq(b, 1e-4), "{}: {a} {b}", object.name);
        if let (ObjectData::Mesh(a), ObjectData::Mesh(b)) = (&object.data, &other.data) {
            assert_eq!(a.faces, b.faces);
            assert_eq!(a.smooth, b.smooth);
        }
    }
}

#[test]
fn reads_y_up_centimetres_and_every_transform_op() {
    let file = br#"#usda 1.0
(
    metersPerUnit = 0.01
    upAxis = "Y"
    timeCodesPerSecond = 30
)
def Xform "World"
{
    def Xform "Moved"
    {
        double3 xformOp:translate = (0, 100, 0)
        float xformOp:rotateY = 90
        float3 xformOp:scale = (2, 2, 2)
        uniform token[] xformOpOrder = ["xformOp:translate", "xformOp:rotateY", "xformOp:scale"]
    }
    def Xform "Matrix"
    {
        matrix4d xformOp:transform = ( (1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (300, 0, 0, 1) )
        double3 xformOp:translate:pivot = (0, 0, 50)
        uniform token[] xformOpOrder = ["xformOp:transform", "xformOp:translate:pivot", "!invert!xformOp:translate:pivot"]
    }
    def Xform "Animated"
    {
        double3 xformOp:translate.timeSamples = {
            0: (0, 0, 0),
            30: (100, 0, 0),
        }
        quatf xformOp:orient = (0.70710677, 0, 0.70710677, 0)
        uniform token[] xformOpOrder = ["xformOp:translate", "xformOp:orient"]
    }
    def Mesh "Triangle" (
        prepend apiSchemas = ["MaterialBindingAPI"]
    )
    {
        int[] faceVertexCounts = [3, 2]
        int[] faceVertexIndices = [0, 1, 2, 2, 0]
        point3f[] points = [(0, 0, 0), (100, 0, 0), (0, 100, 0)]
        uniform token orientation = "leftHanded"
        texCoord2f[] primvars:uv = [(0, 0), (1, 0), (0, 1)] (
            interpolation = "vertex"
        )
        rel material:binding = </World/Looks/Red>
    }
    def Scope "Looks"
    {
        def Material "Red"
        {
            token outputs:surface.connect = </World/Looks/Red/Surface.outputs:surface>
            def Shader "Surface"
            {
                uniform token info:id = "UsdPreviewSurface"
                color3f inputs:diffuseColor = (1, 0, 0)
                float inputs:roughness = 0.25
                token outputs:surface
            }
        }
    }
}
"#;
    let imported = kition_scene_usd::read(file).unwrap();
    let scene = placed(imported.clone());
    // "World" is a transform of its own (not the default prim), so it is kept as an empty.
    let moved = by_name(&scene, "Moved");
    let world = scene.world_matrix(moved.id).unwrap();
    assert!(world.w_axis.truncate().abs_diff_eq(Vec3::new(0.0, 0.0, 1.0), 1e-5), "{world}");
    assert!((world.x_axis.length() - 0.02).abs() < 1e-5, "centimetres scale the root: {world}");
    let matrix = by_name(&scene, "Matrix");
    assert!(scene.world_matrix(matrix.id).unwrap().w_axis.truncate().abs_diff_eq(Vec3::new(3.0, 0.0, 0.0), 1e-5));
    // Time codes at 30 per second: the move takes a second (in the parent's centimetres).
    let animated = imported.objects.iter().position(|object| object.name == "Animated").unwrap();
    let keys: Vec<_> = imported.animation.iter().filter(|channel| channel.object == animated).collect();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].keys.last().unwrap().0, 1.0);
    assert!((keys[0].keys.last().unwrap().1 - 100.0).abs() < 1e-3);
    let triangle = by_name(&scene, "Triangle");
    let ObjectData::Mesh(mesh) = &triangle.data else { panic!("not a mesh") };
    assert_eq!(mesh.faces, vec![vec![2, 1, 0]]);
    assert_eq!(mesh.loose_edges, vec![[0, 2]]);
    assert_eq!(mesh.uvs[0][0].y, 1.0);
    let material = scene.material(triangle.material.unwrap()).unwrap();
    assert_eq!(material.base_color[0..3], [1.0, 0.0, 0.0]);
    assert_eq!(material.roughness, 0.25);
}

#[test]
fn binary_crate_headers_are_checked() {
    // A version older than the reader knows.
    let mut file = b"PXR-USDC".to_vec();
    file.extend_from_slice(&[0; 80]);
    assert!(matches!(kition_scene_usd::read(&file), Err(SceneError::Unsupported(_))));
    // A current version whose table of contents points past the file, or holds absurd counts.
    let mut header = b"PXR-USDC".to_vec();
    header.extend_from_slice(&[0, 8, 0, 0, 0, 0, 0, 0]);
    let mut past = header.clone();
    past.extend_from_slice(&u64::MAX.to_le_bytes());
    past.extend_from_slice(&[0; 64]);
    assert!(matches!(kition_scene_usd::read(&past), Err(SceneError::InvalidFile(_))));
    let mut huge = header.clone();
    huge.extend_from_slice(&88u64.to_le_bytes());
    huge.extend_from_slice(&[0; 64]);
    huge.extend_from_slice(&u64::MAX.to_le_bytes());
    assert!(matches!(kition_scene_usd::read(&huge), Err(SceneError::InvalidFile(_))));
    // A section whose span passes the file's end.
    let mut section = header.clone();
    section.extend_from_slice(&88u64.to_le_bytes());
    section.extend_from_slice(&[0; 64]);
    section.extend_from_slice(&1u64.to_le_bytes());
    section.extend_from_slice(b"TOKENS\0\0\0\0\0\0\0\0\0\0");
    section.extend_from_slice(&0u64.to_le_bytes());
    section.extend_from_slice(&u64::MAX.to_le_bytes());
    assert!(matches!(kition_scene_usd::read(&section), Err(SceneError::InvalidFile(_))));
}

/// A stored (uncompressed) zip holding `name`.
fn package(name: &str, data: &[u8]) -> Vec<u8> {
    let mut zip = b"PK\x03\x04".to_vec();
    zip.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    zip.extend_from_slice(&(data.len() as u32).to_le_bytes());
    zip.extend_from_slice(&(data.len() as u32).to_le_bytes());
    zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
    zip.extend_from_slice(&0u16.to_le_bytes());
    zip.extend_from_slice(name.as_bytes());
    zip.extend_from_slice(data);
    zip
}

#[test]
fn packages_are_read_through_their_first_layer() {
    let text = kition_scene_usd::write(&Scene::starter(), None).unwrap();
    let scene = placed(kition_scene_usd::read(&package("scene.usda", &text)).unwrap());
    assert_eq!(scene.objects.len(), 3);
    assert!(kition_scene_usd::read(&package("texture.png", &text)).is_err());
    // A package inside a package is not opened.
    assert!(kition_scene_usd::read(&package("a.usd", &package("b.usda", &text))).is_err());
}

#[test]
fn damaged_files_never_panic() {
    let text = kition_scene_usd::write(&Scene::starter(), None).unwrap();
    for end in (0..text.len()).step_by(3) {
        let _ = kition_scene_usd::read(&text[..end]);
    }
    let alphabet = b"0123456789-.,:()[]{}<>@\"'=#aZ \n";
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    for _ in 0..3000 {
        let mut damaged = text.clone();
        for _ in 0..1 + (state % 3) {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let at = (state % damaged.len() as u64) as usize;
            damaged[at] = alphabet[((state >> 40) % alphabet.len() as u64) as usize];
        }
        let _ = kition_scene_usd::read(&damaged);
    }
}

#[test]
fn hostile_meshes_are_refused() {
    let mesh = |body: &str| format!("#usda 1.0\ndef Mesh \"m\" {{\n{body}\n}}\n");
    let cases = [
        // A corner past the points.
        "int[] faceVertexCounts = [3]\nint[] faceVertexIndices = [0, 1, 7]\npoint3f[] points = [(0,0,0), (1,0,0), (0,1,0)]",
        // Faces that need more corners than there are.
        "int[] faceVertexCounts = [3, 4000000000]\nint[] faceVertexIndices = [0, 1, 2]\npoint3f[] points = [(0,0,0), (1,0,0), (0,1,0)]",
        // A negative face size and index.
        "int[] faceVertexCounts = [-3]\nint[] faceVertexIndices = [0, 1, 2]\npoint3f[] points = [(0,0,0), (1,0,0), (0,1,0)]",
        "int[] faceVertexCounts = [3]\nint[] faceVertexIndices = [0, 1, -2]\npoint3f[] points = [(0,0,0), (1,0,0), (0,1,0)]",
        // A point that is not a number.
        "int[] faceVertexCounts = [3]\nint[] faceVertexIndices = [0, 1, 2]\npoint3f[] points = [(0,0,0), (1,0,0), (0,inf,0)]",
    ];
    for case in cases {
        assert!(kition_scene_usd::read(mesh(case).as_bytes()).is_err(), "{case}");
    }
    // Deep nesting of prims and values.
    let deep = format!("#usda 1.0\n{}{}", "def Xform \"a\" {\n".repeat(400), "}\n".repeat(400));
    assert!(kition_scene_usd::read(deep.as_bytes()).is_err());
    // Bad UVs, normals, materials, and transform ops are ignored rather than refused.
    let lenient = mesh(
        "int[] faceVertexCounts = [3]\nint[] faceVertexIndices = [0, 1, 2]\npoint3f[] points = [(0,0,0), (1,0,0), (0,1,0)]\n\
         texCoord2f[] primvars:st = [(0, 0)] (interpolation = \"faceVarying\")\nint[] primvars:st:indices = [5, -1, 9]\n\
         normal3f[] normals = [(0, 0)]\nrel material:binding = </nowhere>\n\
         float3 xformOp:rotateXYZ = (1)\nmatrix4d xformOp:transform = ((1, 2))\nuniform token[] xformOpOrder = [\"xformOp:rotateXYZ\", \"xformOp:transform\", \"xformOp:\", \"bad\"]",
    );
    let scene = placed(kition_scene_usd::read(lenient.as_bytes()).unwrap());
    assert_eq!(scene.objects.len(), 1);
}

#[test]
fn skeletons_skins_and_blend_shapes_read_back() {
    use kition_scene_model::armature::{Armature, Bone, bind};
    use kition_scene_model::{Fill, Modifier, ModifierKind, Primitive, Property, ShapeKey};
    let mut scene = Scene::empty();
    let mut column = Primitive::Cylinder { vertices: 8, radius: 0.3, depth: 2.0, fill: Fill::Ngon }.build().unwrap();
    for point in &mut column.positions {
        point.z += 1.0;
    }
    let mut rig = Armature { bones: vec![Bone::new("Lower", Vec3::ZERO, Vec3::Z), Bone::new("Upper", Vec3::Z, Vec3::Z * 2.0)] };
    rig.bones[1].parent = Some("Lower".into());
    rig.bones[1].connected = true;
    let rig = scene.add_object("Rig", ObjectData::Armature(rig));
    let wide = column.positions.iter().map(|point| Vec3::new(point.x, point.y, 0.0) * 0.5).collect();
    column.shape_keys.push(ShapeKey { name: "Wide".into(), offsets: wide, value: 0.0 });
    let body = scene.add_object("Column", ObjectData::Mesh(column));
    bind(&mut scene, body, rig, false).unwrap();
    scene.object_mut(body).unwrap().modifiers.push(Modifier {
        name: "Armature".into(),
        enabled: true,
        kind: ModifierKind::Armature { object: Some(rig), preserve_volume: false, deform: Default::default() },
    });
    let flat = Primitive::Plane { size: 1.0 }.build().unwrap();
    let mut flag = flat.clone();
    flag.shape_keys.push(ShapeKey { name: "Lift".into(), offsets: flat.positions.iter().map(|point| Vec3::Z * point.x).collect(), value: 0.2 });
    let flag = scene.add_object("Flag", ObjectData::Mesh(flag));
    scene.insert_key_at(body, Property::ShapeKey, "Wide", 1.0).unwrap();
    scene.insert_key_at(rig, Property::BoneRotation, "Upper", 1.0).unwrap();
    scene.insert_key_at(flag, Property::ShapeKey, "Lift", 1.0).unwrap();
    for (key, value) in kition_scene_model::armature::armature_mut(&mut scene, rig).unwrap().bones.iter_mut().zip([0.0, 0.7]) {
        key.pose.rotation.x = value;
    }
    if let ObjectData::Mesh(mesh) = &mut scene.object_mut(body).unwrap().data {
        mesh.shape_keys[0].value = 1.0;
    }
    if let ObjectData::Mesh(mesh) = &mut scene.object_mut(flag).unwrap().data {
        mesh.shape_keys[0].value = 0.9;
    }
    scene.insert_key_at(body, Property::ShapeKey, "Wide", 11.0).unwrap();
    scene.insert_key_at(rig, Property::BoneRotation, "Upper", 11.0).unwrap();
    scene.insert_key_at(flag, Property::ShapeKey, "Lift", 11.0).unwrap();

    let back = placed(kition_scene_usd::read(&kition_scene_usd::write(&scene, None).unwrap()).unwrap());
    assert!(matches!(by_name(&back, "Rig").data, ObjectData::Armature(_)));
    for frame in [1.0, 4.0, 11.0] {
        let (a, b) = (scene.at_frame(frame), back.at_frame(frame));
        for name in ["Column", "Flag"] {
            let (one, other) = (by_name(&a, name), by_name(&b, name));
            let (one_world, other_world) = (a.world_matrix(one.id).unwrap(), b.world_matrix(other.id).unwrap());
            let mine = one.evaluated_mesh(&a).unwrap();
            let theirs = other.evaluated_mesh(&b).unwrap();
            assert_eq!(mine.positions.len(), theirs.positions.len());
            for (p, q) in mine.positions.iter().zip(&theirs.positions) {
                let (p, q) = (one_world.transform_point3(*p), other_world.transform_point3(*q));
                assert!(p.distance(q) < 2e-3, "{name} at frame {frame}: {p} and {q}");
            }
        }
    }
}

#[test]
fn packages_carry_their_materials_images() {
    let mut scene = Scene::starter();
    let base = scene.add_image(kition_scene_model::Image::new("Bricks.png", b"\x89PNG\r\n\x1a\nbricks".to_vec()).unwrap());
    let packed = scene.add_image(kition_scene_model::Image::new("Packed.png", b"\x89PNG\r\n\x1a\npacked".to_vec()).unwrap());
    let normal = scene.add_image(kition_scene_model::Image::new("Bumps.jpg", b"\xFF\xD8\xFFbumps".to_vec()).unwrap());
    let cube = scene.objects.iter().find(|object| object.name == "Cube").unwrap().id;
    let material = scene.object(cube).unwrap().material.unwrap();
    let material = scene.materials.iter_mut().find(|other| other.id == material).unwrap();
    material.base_color = [0.5, 0.25, 1.0, 1.0];
    material.metallic = 0.6;
    material.roughness = 0.3;
    material.normal_strength = 0.5;
    material.base_color_texture = Some(base.clone());
    material.metallic_roughness_texture = Some(packed.clone());
    material.normal_texture = Some(normal.clone());
    let package = kition_scene_usd::write_package(&scene, None).unwrap();
    let back = placed(kition_scene_usd::read(&package).unwrap());
    let material = back.material(by_name(&back, "Cube").material.unwrap()).unwrap();
    assert_eq!(material.base_color_texture.as_deref(), Some(base.as_str()));
    assert_eq!(material.metallic_roughness_texture.as_deref(), Some(packed.as_str()));
    assert_eq!(material.normal_texture.as_deref(), Some(normal.as_str()));
    assert!((material.base_color[1] - 0.25).abs() < 1e-6 && (material.metallic - 0.6).abs() < 1e-6 && (material.roughness - 0.3).abs() < 1e-6);
    assert!((material.normal_strength - 0.5).abs() < 1e-6);
    assert_eq!(back.image(&base).unwrap().bytes.as_slice(), b"\x89PNG\r\n\x1a\nbricks");
    // The text alone names no images (it has nowhere to keep them).
    let text = kition_scene_usd::write(&scene, None).unwrap();
    assert!(!String::from_utf8(text).unwrap().contains("UsdUVTexture"));
}
