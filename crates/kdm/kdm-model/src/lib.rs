//! The Scene kit's scene as a KDM replica (engine contract 2; `docs/design.md` section 7).
//!
//! [`SceneModel`] implements [`KitModel`]: its view is the scene as the replica stores it
//! ([`read::read_scene`]). The engine's commands keep running on a [`Scene`]; the bridge
//! ([`write::write_changes`]) writes what a command changed as one transaction, after
//! [`write::stored_form`] puts back the stored values of animated properties (the view shows
//! them at the local frame, which is never replicated). [`repair::repair`] is the deterministic
//! post-merge repair. Meshes, images, and groomed hair are blobs ([`mesh_blob`], [`blobs`],
//! [`hair_blob`]).
#![forbid(unsafe_code)]

pub mod blobs;
pub mod hair_blob;
pub mod held;
pub mod ids;
pub mod mesh_blob;
pub mod nodes;
pub mod read;
pub mod repair;
pub mod write;

use std::collections::BTreeSet;
use std::sync::{Arc, OnceLock};

use kition_kit::kdm::{ActorId, KdmError, ObjectId, Replica, Schema, Tx};
use kition_kit::session::{BlobSink, BlobSource, Damage, KitError, KitModel};
use kition_scene_model::Scene;

pub use kition_kit;

/// The schema (`model/schema.json`).
pub const SCHEMA: &str = include_str!("../model/schema.json");

/// The root object's id.
pub const ROOT: &str = "scene";

/// The root `Scene` object's id: one per replica, so every reader finds it without an index.
pub fn root_id() -> ObjectId {
    ObjectId::from_string(ROOT)
}

/// The parsed schema, read once.
pub fn schema() -> Arc<Schema> {
    static PARSED: OnceLock<Arc<Schema>> = OnceLock::new();
    PARSED
        .get_or_init(|| {
            #[allow(clippy::expect_used)]
            Arc::new(Schema::parse(SCHEMA).expect("model/schema.json is a fixed, tested file"))
        })
        .clone()
}

pub(crate) type KdmResult<T> = std::result::Result<T, KdmError>;

pub(crate) fn kdm_err(error: impl std::fmt::Display) -> KdmError {
    KdmError::Engine(error.to_string())
}

pub(crate) fn kit_err(error: impl std::fmt::Display) -> KitError {
    KitError::new(error.to_string())
}

/// The engine's view of a replica: the scene as stored (animated properties at their stored
/// values, the timeline at frame 1), and the meshes decoded so far by blob hash.
#[derive(Debug, Clone, Default)]
pub struct SceneModel {
    pub stored: Scene,
    meshes: mesh_blob::MeshCache,
}

impl SceneModel {
    /// A view of `stored`, as the bridge leaves it after writing it.
    pub fn adopt(&mut self, stored: Scene) {
        self.stored = stored;
    }

    /// Reads the replica again (after an undo, a redo, or a peer's changes).
    pub fn reload(&mut self, replica: &Replica, blobs: &dyn BlobSource) -> kition_kit::session::Result<()> {
        self.stored = read::read_scene(replica, blobs, &mut self.meshes).map_err(kit_err)?;
        Ok(())
    }

    /// Whether a blob the scene references is still missing (a peer's blobs come late).
    pub fn missing_blobs(&self, replica: &Replica, blobs: &dyn BlobSource) -> Vec<String> {
        read::blob_hashes(replica).into_iter().filter(|hash| blobs.get(hash).is_none()).collect()
    }
}

impl KitModel for SceneModel {
    fn schema() -> Arc<Schema> {
        schema()
    }

    fn load(replica: &Replica, blobs: &dyn BlobSource) -> kition_kit::session::Result<Self> {
        let mut model = Self::default();
        model.reload(replica, blobs)?;
        Ok(model)
    }

    /// The whole scene is read again: a scene's records are few, and decoded meshes are cached
    /// by their blob's hash.
    fn apply(&mut self, replica: &Replica, _changed: &BTreeSet<ObjectId>, blobs: &dyn BlobSource) -> kition_kit::session::Result<Damage> {
        self.reload(replica, blobs)?;
        Ok(Damage::Full)
    }

    fn validate(tx: &mut Tx<'_>, _changed: &BTreeSet<ObjectId>) -> kition_kit::session::Result<bool> {
        Ok(repair::repair(tx)?)
    }

    /// A `.kscene` file is the replica itself; the document opens it with its own reader, and
    /// this one opens only a replica snapshot handed over as a file.
    fn import(_name: &str, bytes: &[u8], _blobs: &mut dyn BlobSink, actor: ActorId) -> kition_kit::session::Result<Replica> {
        Ok(Replica::load(schema(), bytes, actor)?)
    }

    /// The scene's content (`content.json`, the state and no history: rule M16b), as a `.kscene`
    /// file's content entry holds it.
    fn export(&self, replica: &Replica, _name: &str, _blobs: &dyn BlobSource) -> kition_kit::session::Result<Vec<u8>> {
        Ok(replica.content_snapshot()?)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_schema_parses() {
        assert!(kition_kit::kdm::Schema::parse(super::SCHEMA).is_ok());
        assert_eq!(super::schema().kit, "kit.scene");
    }
}
