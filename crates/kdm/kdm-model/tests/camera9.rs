//! Coupled camera fit, gate and lens settings converge as one data register.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
use kition_kit::{
    kdm::{ActorId, DocId, Origin, Replica},
    session::{BlobSink, MemoryBlobs},
};
use kition_scene_kdm::{
    mesh_blob::MeshCache,
    read::read_scene,
    schema,
    write::{Written, write_changes, write_scene},
};
use kition_scene_model::{Camera, ObjectData, Scene, scene::SensorFit};

fn keep(blobs: &mut MemoryBlobs, written: Written) {
    for blob in written.blobs {
        blobs.put(&blob.bytes, "application/octet-stream");
    }
}
fn read(replica: &Replica, blobs: &MemoryBlobs) -> Scene {
    read_scene(replica, blobs, &mut MeshCache::default()).unwrap()
}
fn scene() -> Scene {
    let mut scene = Scene::empty();
    scene.add_object("First", ObjectData::Camera(Camera::default()));
    scene.add_object("Second", ObjectData::Camera(Camera::default()));
    scene
}
fn change(replica: &mut Replica, blobs: &mut MemoryBlobs, object: usize, vertical: bool) -> Scene {
    let before = read(replica, blobs);
    let mut after = before.clone();
    let ObjectData::Camera(camera) = &mut after.objects[object].data else { panic!("camera") };
    camera.sensor_fit = if vertical { SensorFit::Vertical } else { SensorFit::Horizontal };
    camera.sensor_height = if vertical { 19.0 } else { 27.0 };
    camera.sensor = if vertical { 35.0 } else { 42.0 };
    camera.lens = if vertical { 71.0 } else { 39.0 };
    let written = replica.transact(&Origin::person("editor"), |tx| write_changes(tx, &before, &after, None)).unwrap();
    keep(blobs, written);
    after
}
#[test]
fn concurrent_camera_settings_converge_without_mixing_coupled_fields_twenty_times() {
    for _ in 0..20 {
        for (first, second) in [(1, 2), (2, 1)] {
            for same_camera in [false, true] {
                let mut blobs = MemoryBlobs::default();
                let (mut a, written) =
                    Replica::create_with(schema(), DocId::new(), ActorId::from_u64(first).unwrap(), &Origin::system("create"), |tx| {
                        write_scene(tx, &scene())
                    })
                    .unwrap();
                keep(&mut blobs, written);
                let base = a.snapshot().unwrap();
                let version = a.version();
                let mut b = Replica::load(schema(), &base, ActorId::from_u64(second).unwrap()).unwrap();
                let left = change(&mut a, &mut blobs, 0, false);
                let right = change(&mut b, &mut blobs, usize::from(!same_camera), true);
                let ca = a.export_changes(&version).unwrap();
                let cb = b.export_changes(&version).unwrap();
                let mut c = Replica::load(schema(), &base, ActorId::from_u64(100).unwrap()).unwrap();
                let mut d = Replica::load(schema(), &base, ActorId::from_u64(200).unwrap()).unwrap();
                c.import_changes(&ca).unwrap();
                c.import_changes(&cb).unwrap();
                d.import_changes(&cb).unwrap();
                d.import_changes(&ca).unwrap();
                for replica in [&mut c, &mut d] {
                    replica.transact(&Origin::system("repair"), kition_scene_kdm::repair::repair).unwrap();
                }
                assert_eq!(c.state_json().unwrap(), d.state_json().unwrap());
                let merged = read(&c, &blobs);
                merged.validate().unwrap();
                assert_eq!(merged, read(&d, &blobs));
                if same_camera {
                    assert!(merged.objects[0].data == left.objects[0].data || merged.objects[0].data == right.objects[0].data);
                } else {
                    assert_eq!(merged.objects[0].data, left.objects[0].data);
                    assert_eq!(merged.objects[1].data, right.objects[1].data);
                }
            }
        }
    }
}
