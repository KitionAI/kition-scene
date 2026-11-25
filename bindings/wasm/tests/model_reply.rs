//! `modelReply` (the SDK's `export_kit_session!` member the editor sends every `ai/inspect`,
//! `document/check` and `ai/run` reply through): in a scene with many objects, a reply naming one
//! object by its replica id gets the same tool id the whole document's table gives it, read only
//! at that object (`ToolIds::show_reply`, kition-sdk 03f6285); a reply naming none comes back as
//! it is.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use kition_kit::KitDocument;
use kition_kit::kdm::tool_ids::ToolIds;
use kition_kit::kdm::{ActorId, DocId, ObjectId, Origin, Replica};
use kition_kit::session::{__model_reply, KitSessionDocument};
use kition_scene_kit::SceneDocument;
use kition_scene_model::{ObjectData, Scene};
use serde_json::{Value, json};

const OBJECTS: usize = 2000;

/// A scene of `OBJECTS` empties, opened from its file.
fn crowded() -> SceneDocument {
    let mut scene = Scene::empty();
    for index in 0..OBJECTS {
        scene.add_object(&format!("Empty {index}"), ObjectData::Empty { size: 1.0 });
    }
    scene.validate().unwrap();
    let (replica, _) =
        Replica::create_with(kition_scene_kdm::schema(), DocId::new(), ActorId::from_u64(1).unwrap(), &Origin::system("create"), |tx| {
            kition_scene_kdm::write::write_scene(tx, &scene)
        })
        .unwrap();
    let file = kition_scene_kscene::File {
        meta: kition_scene_kscene::Meta { kdm: kition_kit::kdm::KDM_VERSION, schema_version: 1, doc_id: replica.doc_id().to_string() },
        content: replica.content_snapshot().unwrap(),
        ..Default::default()
    };
    SceneDocument::open("Crowded.kscene", &kition_scene_kscene::write(&file).unwrap()).unwrap()
}

/// The replica id of the object named `name`.
fn record(doc: &SceneDocument, name: &str) -> ObjectId {
    let replica = KitSessionDocument::session(doc).replica();
    replica.list(&kition_scene_kdm::root_id(), "objects").unwrap().into_iter().find(|id| replica.get(id, "name").unwrap() == json!(name)).unwrap()
}

fn reply(doc: &SceneDocument, value: &Value) -> Value {
    serde_json::from_str(&__model_reply(doc, &value.to_string()).unwrap()).unwrap()
}

#[test]
fn a_reply_naming_one_object_gets_the_ids_of_the_whole_read() {
    let mut doc = crowded();
    assert_eq!(doc.scene().objects.len(), OBJECTS);
    let named = record(&doc, "Empty 1234");
    let key = KitSessionDocument::session(&doc).replica().get(&named, "key").unwrap();
    let value = json!({
        "object": named.to_string(),
        "objects": [named.to_string(), "not an id"],
        "byId": { named.to_string(): { "name": "Empty 1234" } },
        "note": format!("text that mentions {named} stays"),
    });
    let whole = ToolIds::of_replica(KitSessionDocument::session(&doc).replica()).show_json(&value);
    assert_eq!(reply(&doc, &value), whole, "the same names as the whole document's table");
    assert_eq!(whole["object"], json!(key.as_u64().unwrap().to_string()), "the object is named by its key");
    assert_eq!(whole["note"], value["note"]);
    // Right after a change too (the table read before it no longer holds).
    doc.execute_as("object.rename", json!({ "object": key, "name": "Renamed" }), &kition_scene_kit::person()).unwrap();
    let whole = ToolIds::of_replica(KitSessionDocument::session(&doc).replica()).show_json(&value);
    assert_eq!(reply(&doc, &value), whole);
}

#[test]
fn a_reply_naming_no_object_comes_back_as_it_is() {
    let doc = crowded();
    let value = json!({ "objects": OBJECTS, "names": ["Empty 1", "Empty 2"], "nested": { "id": "Empty 3", "at": [1, 2, 3] } });
    assert_eq!(reply(&doc, &value), value);
}
