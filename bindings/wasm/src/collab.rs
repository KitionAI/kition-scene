//! The members a host uses to share a scene (engine contract 2): open a scene from a replica the
//! host stored, the replica's id, the blob exchange (`blobs`, `blobsSince`, `addBlobs`; meshes and
//! images are blobs), and the object model served over the change exchange (`ReplicaPort`) for
//! `kition-scene`. Follows Kition Board's `bindings/wasm/src/collab.rs`.

use std::collections::BTreeSet;
use std::sync::Arc;

use kition_kit::kdm::{ActorId, Origin, Replica, Schema, Version};
use kition_kit::session::{BlobSource, KitSession, KitSessionDocument};
use kition_kit::{KitCommand, KitDocument};
use kition_scene_kdm::blobs::{SharedBlobs, decode_bundle, encode_bundle};
use kition_scene_kdm::{SceneModel, read};
use serde_json::Value;

use crate::SceneDocument;

/// Which blobs arrived with which version, so `blobsSince` answers what a peer at a version lacks.
#[derive(Clone, Debug, Default)]
pub struct BlobLog(Vec<(Version, Vec<String>)>);

impl BlobLog {
    /// Records `hashes` as new at `version`.
    pub fn record(&mut self, version: Version, hashes: Vec<String>) {
        if !hashes.is_empty() {
            self.0.push((version, hashes));
        }
    }

    /// Every hash logged.
    pub fn known(&self) -> BTreeSet<String> {
        self.0.iter().flat_map(|(_, hashes)| hashes.iter().cloned()).collect()
    }

    /// The hashes logged at versions `since` does not include.
    fn after(&self, since: &Version) -> BTreeSet<String> {
        self.0.iter().filter(|(version, _)| !since.includes(version)).flat_map(|(_, hashes)| hashes.iter().cloned()).collect()
    }
}

impl SceneDocument {
    /// The replica's document id (stable across saves; never the file's path).
    pub fn doc_id(&self) -> String {
        self.replica.replica().doc_id().to_string()
    }

    /// Opens a scene with the replica the host stored for it (`snapshot`, its history) and that
    /// replica's blobs (a blob bundle). Empty `bytes` open the replica alone. When both are given
    /// and are the same document, the file (which holds the content and no history) is taken into
    /// the stored replica (`kition_kit::session::take_file`): nothing when the replica has the
    /// file's version; the changes the file holds beyond it when the file's history starts at a
    /// version the replica has; and otherwise (the file was saved elsewhere, from changes the store
    /// never saw) the file's state, as one `system` transaction keyed by object id. Edits the
    /// replica has and the file lacks are kept. A file of another document, or a replica that does
    /// not open, leaves the file as [`KitDocument::open`] opens it.
    pub fn open_with_replica(_name: &str, bytes: &[u8], snapshot: &[u8], blobs: &[u8]) -> Result<Self, String> {
        let store = SharedBlobs::default();
        for (hash, bytes) in decode_bundle(blobs)? {
            store.insert(&hash, &bytes)?;
        }
        if bytes.is_empty() {
            let replica = KitSession::<SceneModel>::open_snapshot(snapshot, Box::new(store.clone()), ActorId::random())
                .map_err(|error| format!("the stored replica does not open: {error}"))?;
            return Ok(Self::attach(replica, store, crate::Editor::default()));
        }
        let (_, opened) = crate::open_file(bytes, &store)?;
        match Replica::load(kition_scene_kdm::schema(), snapshot, ActorId::random()) {
            Ok(mut stored) if stored.doc_id() == opened.replica().doc_id() => {
                kition_kit::session::take_file(&mut stored, opened.replica()).map_err(|error| format!("damaged: {error}"))?;
                let stored = KitSession::<SceneModel>::from_replica(stored, Box::new(store.clone())).map_err(|error| format!("damaged: {error}"))?;
                Ok(Self::attach(stored, store, crate::Editor::default()))
            }
            _ => Ok(Self::attach(opened, store, crate::Editor::default())),
        }
    }

    /// Every blob the scene references that the store holds, as a blob bundle.
    pub fn blobs_bundle(&self) -> Vec<u8> {
        encode_bundle(&self.blob_pairs(read::blob_hashes(self.replica.replica())))
    }

    /// The blobs a peer at `since` (an encoded version; empty for none) needs to reach this
    /// document's state, as a blob bundle.
    pub fn blobs_since(&self, since: &[u8]) -> Result<Vec<u8>, String> {
        if since.is_empty() {
            return Ok(self.blobs_bundle());
        }
        let since = Version::decode(since).map_err(|error| error.to_string())?;
        let mut hashes = self.blob_log.after(&since);
        // Blobs that came another way (a file, a peer) count as new: a peer that has them ignores them.
        let known = self.blob_log.known();
        hashes.extend(read::blob_hashes(self.replica.replica()).into_iter().filter(|hash| !known.contains(hash)));
        Ok(encode_bundle(&self.blob_pairs(hashes)))
    }

    fn blob_pairs(&self, hashes: BTreeSet<String>) -> Vec<(String, Arc<[u8]>)> {
        hashes.into_iter().filter_map(|hash| self.blobs.get(&hash).map(|bytes| (hash, bytes))).collect()
    }

    /// Takes a peer's blobs (before or after their changes). Answers whether the view changed: a
    /// mesh or an image the scene references that was missing fills in.
    pub fn add_blobs(&mut self, bundle: &[u8]) -> Result<bool, String> {
        let wanted = read::blob_hashes(self.replica.replica());
        let mut added = Vec::new();
        let mut late = false;
        for (hash, bytes) in decode_bundle(bundle)? {
            if self.blobs.insert(&hash, &bytes)? {
                late |= wanted.contains(&hash);
                added.push(hash);
            }
        }
        self.blob_log.record(self.replica.version(), added);
        if late {
            let stored = read::read_scene(self.replica.replica(), &self.blobs, &mut kition_scene_kdm::mesh_blob::MeshCache::default())?;
            self.replica.model_mut().adopt(stored);
            self.view_from_replica(None);
            self.revision += 1;
        }
        Ok(late)
    }

    /// [`KitDocument::reconcile_reread`]: `fresh` is this clean session's file read again after
    /// another program saved it. A `.kscene` file holds the scene's content and, of its history,
    /// only the seal (rule M16b): when its history starts at a version this replica has, the
    /// file's difference is the changes it holds beyond it, checked on a copy and imported as they
    /// are; otherwise (another writer saved changes this replica never saw) it is the file's state,
    /// written by object id as one `system` transaction labelled `import`. Either way it stays
    /// outside both undo histories and is repaired like any import, with the file's blobs. Object
    /// ids, both undo histories and the editor state stay. An unchanged file writes nothing. A file
    /// of another scene is not reconciled (`false`, nothing changed). Refuses, changing nothing,
    /// when the file no longer has what the editor state names (`deleted selected object 4`), and
    /// when the changes do not import.
    pub(crate) fn reread_file(&mut self, fresh: &Self) -> Result<bool, String> {
        if fresh.replica.replica().doc_id() != self.replica.replica().doc_id() {
            return Ok(false);
        }
        if let Some(reason) = self.unresolved_in(&fresh.replica.model().stored) {
            return Err(reason);
        }
        let ours = self.replica.version();
        let theirs_blobs = fresh.blob_pairs(fresh.blobs.hashes().into_iter().collect());
        let difference = if ours.includes(&fresh.replica.version()) {
            Difference::None
        } else if fresh.replica.replica().reaches(&ours) {
            let changes = fresh.replica.export_changes(&ours).map_err(|error| error.to_string())?;
            // Imported into a copy first (with this session's and the file's blobs), so changes
            // that do not import leave this session as it was.
            let trial = SharedBlobs::default();
            for (hash, bytes) in self.blob_pairs(self.blobs.hashes().into_iter().collect()).iter().chain(&theirs_blobs) {
                trial.insert(hash, bytes)?;
            }
            self.replica
                .fork(ActorId::random(), Box::new(trial))
                .and_then(|mut copy| copy.import_changes(&changes))
                .map_err(|error| error.to_string())?;
            Difference::Changes(changes)
        } else {
            let before = kition_kit::session::state(self.replica.replica()).map_err(|error| error.to_string())?;
            let after = kition_kit::session::state(fresh.replica.replica()).map_err(|error| error.to_string())?;
            if before == after { Difference::None } else { Difference::State(before, after) }
        };
        let mut added = Vec::new();
        for (hash, bytes) in theirs_blobs {
            if self.blobs.insert(&hash, &bytes)? {
                added.push(hash);
            }
        }
        match difference {
            Difference::None => {}
            Difference::Changes(changes) => {
                KitSessionDocument::import_changes(self, &changes)?;
            }
            Difference::State(before, after) => {
                let schema = kition_scene_kdm::schema();
                self.replica
                    .execute(&Origin::system("import"), |_, tx| kition_kit::session::write_state(tx, &schema, &before, &after))
                    .map_err(|error| error.to_string())?;
                self.replica_changed();
            }
        }
        self.blob_log.record(self.replica.version(), added);
        Ok(true)
    }
}

/// How a file read again reaches the live replica ([`SceneDocument::reread_file`]).
enum Difference {
    /// The replica has every change of the file, or the same state.
    None,
    /// The changes the file holds beyond the replica: its history reaches the replica's version.
    Changes(Vec<u8>),
    /// The file was saved from changes the replica lacks: the replica's state and the file's.
    State(kition_kit::session::State, kition_kit::session::State),
}

/// The object model (`KitObjectModel 1.0`) headless, over the document's change exchange.
impl kition_kit::session::model::ReplicaPort for SceneDocument {
    fn model_schema(&self) -> Arc<Schema> {
        kition_scene_kdm::schema()
    }

    fn model_export(&mut self, since: &[u8]) -> Result<Vec<u8>, String> {
        kition_kit::session::__export_changes(self, since)
    }

    fn model_import(&mut self, bytes: &[u8]) -> Result<(), String> {
        KitSessionDocument::import_changes(self, bytes).map(|_| ())
    }

    fn model_command(&mut self, command: &str, params: Value, origin: &Origin) -> Result<Value, String> {
        self.execute_as(command, params, origin)
    }

    fn model_commands(&self) -> Vec<KitCommand> {
        self.commands()
    }

    /// A shared scene with no file yet: the replica and its blobs alone, failing rather than
    /// falling back to an empty scene.
    fn model_open_replica(name: &str, snapshot: &[u8], blobs: &[u8]) -> Result<Self, String> {
        Self::open_with_replica(name, &[], snapshot, blobs)
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use kition_kit::wasm_bindgen::prelude::*;

    use crate::__kition_kit_export::WebDocument;
    use crate::SceneDocument;

    fn error(message: String) -> JsError {
        JsError::new(&message)
    }

    #[wasm_bindgen(js_class = "SceneDocument", wasm_bindgen = kition_kit::wasm_bindgen)]
    impl WebDocument {
        #[wasm_bindgen(js_name = openWithReplica)]
        pub fn open_with_replica(name: &str, bytes: &[u8], snapshot: &[u8], blobs: &[u8]) -> Result<WebDocument, JsError> {
            SceneDocument::open_with_replica(name, bytes, snapshot, blobs).map(WebDocument).map_err(error)
        }

        #[wasm_bindgen(getter, js_name = hasReplica)]
        pub fn has_replica(&self) -> bool {
            true
        }

        #[wasm_bindgen(getter, js_name = docId)]
        pub fn doc_id(&self) -> String {
            self.0.doc_id()
        }

        pub fn blobs(&self) -> Vec<u8> {
            self.0.blobs_bundle()
        }

        #[wasm_bindgen(js_name = blobsSince)]
        pub fn blobs_since(&self, since: &[u8]) -> Result<Vec<u8>, JsError> {
            self.0.blobs_since(since).map_err(error)
        }

        #[wasm_bindgen(js_name = addBlobs)]
        pub fn add_blobs(&mut self, bytes: &[u8]) -> Result<bool, JsError> {
            self.0.add_blobs(bytes).map_err(error)
        }
    }
}
