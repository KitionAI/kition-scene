//! What the `kition-scene` executable serves beside the engine: where a `.kscene` file keeps the
//! scene's replica (`document.identity` and `document.fork`, Kit Process Protocol section 5.6).

use std::sync::Arc;

use kition_kit::kdm::{ActorId, Replica, Schema};
use kition_kit_server::{ErrorReason, RpcError};

/// Where a `.kscene` file keeps the scene's replica (`docs/design.md` section 7.5): `replica.kdm`,
/// the replica's snapshot with its history. A fork's file holds the SDK's copy there instead, the
/// blobs that copy's state references and nothing else, and `editor.json` as it was (the view,
/// the selection and the current frame name objects by ids the copy keeps).
pub struct SceneFiles;

fn unsupported(error: impl std::fmt::Display) -> RpcError {
    RpcError::kit_detail(ErrorReason::UnsupportedFormat, error.to_string())
}

impl kition_headless::EmbeddedReplica for SceneFiles {
    fn schema(&self) -> Arc<Schema> {
        kition_scene_kdm::schema()
    }

    /// `replica.kdm`, or `None` for an empty file (one made for the kit before it saved a scene
    /// there). Bytes that are not a `.kscene` file are `unsupported_format`.
    fn replica(&self, file: &[u8]) -> Result<Option<Vec<u8>>, RpcError> {
        if file.is_empty() {
            return Ok(None);
        }
        Ok(Some(kition_scene_kscene::read(file).map_err(unsupported)?.snapshot))
    }

    /// `file` with `snapshot` as `replica.kdm` (so none of the source's history stays: the old
    /// snapshot was the only place it lived) and only the blobs the snapshot's state references,
    /// as a save writes them; `editor.json` is kept.
    fn replace(&self, file: &[u8], snapshot: &[u8]) -> Result<Vec<u8>, RpcError> {
        let mut parts = kition_scene_kscene::read(file).map_err(unsupported)?;
        let replica = Replica::load(kition_scene_kdm::schema(), snapshot, ActorId::random()).map_err(unsupported)?;
        let referenced = kition_scene_kdm::read::blob_hashes(&replica);
        parts.blobs.retain(|(hash, _)| referenced.contains(hash));
        parts.snapshot = snapshot.to_vec();
        kition_scene_kscene::write(&parts).map_err(unsupported)
    }
}
