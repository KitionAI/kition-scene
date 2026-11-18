//! Export, import, and compare: every format gives back the geometry it was given.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

use kition_scene_exchange::{Format, export, import, place};
use kition_scene_model::glam::{Mat4, Vec3};
use kition_scene_model::{Fill, LightKind, Mesh, Modifier, ModifierKind, ObjectData, Primitive, Scene};

fn sample() -> Scene {
    let mut scene = Scene::starter();
    let add = |scene: &mut Scene, name: &str, primitive: Primitive| scene.add_object(name, ObjectData::Mesh(primitive.build().unwrap()));
    let sphere = add(&mut scene, "Sphere", Primitive::UvSphere { segments: 12, rings: 6, radius: 0.75 });
    let cylinder = add(&mut scene, "Cylinder", Primitive::Cylinder { vertices: 10, radius: 0.5, depth: 1.5, fill: Fill::Ngon });
    let ring = add(&mut scene, "Ring", Primitive::Circle { vertices: 8, radius: 0.4, fill: Fill::Nothing });
    let hidden = add(&mut scene, "Hidden", Primitive::Cube { size: 1.0 });
    let sun = scene.add_object("Sun", ObjectData::Light(kition_scene_model::Light::new(LightKind::Sun)));
    {
        let object = scene.object_mut(sphere).unwrap();
        object.location = Vec3::new(3.0, 1.0, 0.5);
        object.rotation = Vec3::new(0.3, 0.2, 1.1);
        object.scale = Vec3::new(1.0, 2.0, 1.0);
        object.modifiers.push(Modifier::new(ModifierKind::Subdivision { levels: 1 }));
    }
    scene.object_mut(cylinder).unwrap().location = Vec3::new(-2.0, 0.5, 1.0);
    scene.object_mut(cylinder).unwrap().rotation = Vec3::new(0.0, 0.7, 0.0);
    scene.set_parent(cylinder, Some(sphere)).unwrap();
    scene.object_mut(ring).unwrap().location = Vec3::new(0.0, -3.0, 0.0);
    scene.object_mut(hidden).unwrap().hidden = true;
    scene.object_mut(sun).unwrap().rotation = Vec3::new(0.5, 0.1, 0.2);
    let red = scene.add_material("Red");
    scene.material_mut(red).unwrap().base_color = [0.8, 0.1, 0.1, 1.0];
    scene.material_mut(red).unwrap().metallic = 0.25;
    scene.object_mut(sphere).unwrap().material = Some(red);
    scene
}

/// The world-space positions of a mesh, its sorted face sizes, and its loose edge count.
fn shape(scene: &Scene, name: &str) -> (Vec<Vec3>, Vec<usize>, usize) {
    let object = scene.object_named(name).unwrap_or_else(|| panic!("no {name}"));
    let mut mesh: Mesh = object.evaluated_mesh(scene).unwrap();
    mesh.transform(scene.world_matrix(object.id).unwrap());
    let mut sizes: Vec<usize> = mesh.faces.iter().map(Vec::len).collect();
    sizes.sort_unstable();
    (mesh.positions, sizes, mesh.loose_edges.len())
}

/// Both meshes have the same face sizes and loose edges, and every point of each lies within
/// `tolerance` of a point of the other.
fn assert_same_shape(got: (Vec<Vec3>, Vec<usize>, usize), want: (Vec<Vec3>, Vec<usize>, usize), tolerance: f32, what: &str) {
    assert_eq!((&got.1, got.2), (&want.1, want.2), "{what}: faces");
    for (from, to) in [(&got.0, &want.0), (&want.0, &got.0)] {
        for point in from {
            assert!(to.iter().any(|other| other.distance(*point) <= tolerance), "{what}: {point} has no match");
        }
    }
}

fn reimport(scene: &Scene, format: Format) -> Scene {
    let bytes = export(scene, format, None).unwrap();
    let mut back = Scene::empty();
    place(&mut back, import(&bytes, format).unwrap()).unwrap();
    back.validate().unwrap();
    back
}

#[test]
fn gltf_keeps_objects_hierarchy_materials_cameras_and_lights() {
    let scene = sample();
    for format in [Format::Glb, Format::Gltf] {
        let back = reimport(&scene, format);
        assert!(back.object_named("Hidden").is_none(), "hidden objects stay out");
        for name in ["Cube", "Sphere", "Cylinder", "Ring"] {
            assert_same_shape(shape(&back, name), shape(&scene, name), 1e-5, &format!("{format:?}: {name}"));
        }
        // Polygons come back as polygons (the cube's quads, the cylinder's caps).
        assert_eq!(back.object_named("Cube").unwrap().mesh().unwrap().faces.len(), 6);
        assert!(back.object_named("Sphere").unwrap().mesh().unwrap().smooth);
        let cylinder = back.object_named("Cylinder").unwrap();
        assert_eq!(back.object(cylinder.parent.unwrap()).unwrap().name, "Sphere");
        for name in ["Camera", "Light", "Sun", "Cylinder"] {
            let (a, b) = (scene.object_named(name).unwrap(), back.object_named(name).unwrap());
            let (wa, wb) = (scene.world_matrix(a.id).unwrap(), back.world_matrix(b.id).unwrap());
            assert!(wa.abs_diff_eq(wb, 1e-4), "{format:?}: {name} moved\n{wa}\n{wb}");
        }
        let camera = back.object_named("Camera").unwrap();
        let ObjectData::Camera(lens) = &camera.data else { panic!() };
        assert!((lens.lens - 50.0).abs() < 1e-3 && (lens.clip_end - 100.0).abs() < 1e-3);
        assert_eq!(back.camera, Some(camera.id));
        let ObjectData::Light(light) = &back.object_named("Light").unwrap().data else { panic!() };
        assert_eq!(light.kind, LightKind::Point);
        assert!((light.power - 1000.0).abs() < 0.05, "{}", light.power);
        let ObjectData::Light(sun) = &back.object_named("Sun").unwrap().data else { panic!() };
        assert_eq!(sun.kind, LightKind::Sun);
        assert!((sun.power - 1.0).abs() < 1e-4);
        let red = back.material(back.object_named("Sphere").unwrap().material.unwrap()).unwrap();
        assert_eq!(red.name, "Red");
        assert!((red.base_color[1] - 0.1).abs() < 1e-6 && (red.metallic - 0.25).abs() < 1e-6);
    }
}

#[test]
fn only_exports_the_named_objects_and_their_parents() {
    let scene = sample();
    let cylinder = scene.object_named("Cylinder").unwrap().id;
    let back = reimport_only(&scene, Format::Glb, &[cylinder]);
    let names: Vec<&str> = back.objects.iter().map(|object| object.name.as_str()).collect();
    assert_eq!(names, ["Sphere", "Cylinder"]);
    assert!(matches!(back.object_named("Sphere").unwrap().data, ObjectData::Empty { .. }), "the parent is only a node");
    assert_same_shape(shape(&back, "Cylinder"), shape(&scene, "Cylinder"), 1e-5, "Cylinder");
}

fn reimport_only(scene: &Scene, format: Format, only: &[u32]) -> Scene {
    let bytes = export(scene, format, Some(only)).unwrap();
    let mut back = Scene::empty();
    place(&mut back, import(&bytes, format).unwrap()).unwrap();
    back
}

#[test]
fn obj_keeps_polygons_in_world_space() {
    let scene = sample();
    let back = reimport(&scene, Format::Obj);
    for name in ["Cube", "Sphere", "Cylinder", "Ring"] {
        let object = back.object_named(name).unwrap();
        assert_eq!(object.location, Vec3::ZERO);
        assert_same_shape(shape(&back, name), shape(&scene, name), 2e-6 * 4.0, name);
    }
    assert_eq!(back.material(back.object_named("Sphere").unwrap().material.unwrap()).unwrap().name, "Red");
    // Y-up on disk: the ring sits at y = -3 in the scene, so at z = +3 in the file.
    let text = String::from_utf8(export(&scene, Format::Obj, Some(&[scene.object_named("Ring").unwrap().id])).unwrap()).unwrap();
    assert!(text.lines().any(|line| line.starts_with("v ") && line.ends_with(" 3.000000")), "{text}");
}

#[test]
fn stl_keeps_triangles_in_world_space() {
    let mut scene = Scene::empty();
    let cube = scene.add_object("Cube", ObjectData::Mesh(Primitive::Cube { size: 2.0 }.build().unwrap()));
    scene.object_mut(cube).unwrap().location = Vec3::new(1.0, 2.0, 3.0);
    let bytes = export(&scene, Format::Stl, None).unwrap();
    assert_eq!(bytes.len(), 84 + 12 * 50);
    let imported = import(&bytes, Format::Stl).unwrap();
    let ObjectData::Mesh(mesh) = &imported.objects[0].data else { panic!() };
    assert_eq!((mesh.positions.len(), mesh.faces.len()), (8, 12));
    let bounds = mesh.bounds(Mat4::IDENTITY);
    assert!(bounds.min.abs_diff_eq(Vec3::new(0.0, 1.0, 2.0), 1e-6) && bounds.max.abs_diff_eq(Vec3::new(2.0, 3.0, 4.0), 1e-6));
}

#[test]
fn ascii_stl_and_hand_written_obj_are_read() {
    let stl = b"solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid t\n";
    let imported = import(stl, Format::Stl).unwrap();
    let ObjectData::Mesh(mesh) = &imported.objects[0].data else { panic!() };
    assert_eq!(mesh.faces.len(), 1);
    let obj = b"# square and a triangle\nv 0 0 0\nv 1 0 0\nv 1 0 -1\nv 0 0 -1\ng first\nusemtl Paint\nf 1/1/1 2/2/1 3/3/1 4/4/1\ng second\nf -4 -3 -2\nl 1 3\n";
    let imported = import(obj, Format::Obj).unwrap();
    assert_eq!(imported.objects.len(), 2);
    assert_eq!(imported.objects[0].name, "first");
    let ObjectData::Mesh(square) = &imported.objects[0].data else { panic!() };
    assert_eq!(square.faces, vec![vec![0, 1, 2, 3]]);
    // OBJ is Y-up: (1, 0, -1) is (1, 1, 0) in the scene.
    assert!(square.positions[2].abs_diff_eq(Vec3::new(1.0, 1.0, 0.0), 1e-6));
    assert_eq!(imported.materials[0].name, "Paint");
    let ObjectData::Mesh(second) = &imported.objects[1].data else { panic!() };
    assert_eq!((second.faces.len(), second.loose_edges.len()), (1, 1));
}

#[test]
fn formats_name_themselves() {
    for format in [Format::Glb, Format::Gltf, Format::Obj, Format::Stl] {
        assert_eq!(Format::from_extension(format.extension()), Some(format));
        assert!(format.mime().starts_with("model/"));
    }
    assert_eq!(Format::from_extension(".GLB"), Some(Format::Glb));
    assert_eq!(Format::from_extension("fbx"), None);
}
