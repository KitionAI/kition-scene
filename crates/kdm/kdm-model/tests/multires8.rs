//! Recovered subdivision levels stay atomic mesh blobs under both fixed actor and arrival orders.
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
use kition_scene_model::{ObjectData, Primitive, Scene, modifiers::catmull_clark, multires};

fn keep(blobs: &mut MemoryBlobs, written: Written) {
    for blob in written.blobs {
        blobs.put(&blob.bytes, "application/octet-stream");
    }
}
fn read(replica: &Replica, blobs: &MemoryBlobs) -> Scene {
    read_scene(replica, blobs, &mut MeshCache::default()).unwrap()
}
fn scene() -> Scene {
    let mesh = catmull_clark(&catmull_clark(&Primitive::Cube { size: 2.0 }.build().unwrap()));
    let mut scene = Scene::empty();
    scene.add_object("First", ObjectData::Mesh(mesh.clone()));
    scene.add_object("Second", ObjectData::Mesh(mesh));
    scene
}
fn change(replica: &mut Replica, blobs: &mut MemoryBlobs, object: usize, rebuild: bool) -> Scene {
    let before = read(replica, blobs);
    let mut after = before.clone();
    let mesh = after.objects[object].mesh_mut().unwrap();
    if rebuild {
        multires::rebuild_subdivisions(mesh).unwrap();
    } else {
        multires::unsubdivide(mesh).unwrap();
    }
    let written = replica.transact(&Origin::person("editor"), |tx| write_changes(tx, &before, &after, None)).unwrap();
    keep(blobs, written);
    after
}
#[test]
fn concurrent_recovery_blobs_converge_without_mixing_topology_or_detail_twenty_times() {
    for _ in 0..20 {
        for (first, second) in [(1, 2), (2, 1)] {
            for same_mesh in [false, true] {
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
                let right = change(&mut b, &mut blobs, usize::from(!same_mesh), true);
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
                for object in &merged.objects {
                    let mesh = object.mesh().unwrap();
                    assert_eq!(mesh.multires.len(), multires::levels(mesh) as usize);
                    let highest = multires::evaluate(mesh, multires::levels(mesh));
                    assert_eq!(highest.positions.len(), 98);
                    assert_eq!(highest.faces.len(), 96);
                    assert!(highest.positions.iter().all(|p| p.is_finite()));
                }
                if same_mesh {
                    assert!(merged.objects[0].data == left.objects[0].data || merged.objects[0].data == right.objects[0].data);
                } else {
                    assert_eq!(merged.objects[0].data, left.objects[0].data);
                    assert_eq!(merged.objects[1].data, right.objects[1].data);
                }
            }
        }
    }
}
