//! A scene's blobs (meshes and images; `docs/design.md` section 7.1): a store the session and the
//! document share, and the blob bundle hosts exchange (`blobs`, `addBlobs`, `blobsSince`).
//!
//! Ported from Kition Board's `crates/kdm/kdm-model/src/blobs.rs`.
//!
//! The bundle is kition-sdk's blob encoding (`ReplicaPort::model_open_replica`):
//! `(64-char hex SHA-256, u64 LE length, bytes) * N`.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use kition_kit::session::{BlobRef, BlobSink, BlobSource, sha256_hex};

/// Blob bytes by their SHA-256 (lower-case hex), shared by clones: the session holds one clone as
/// its store and the document another, to enumerate what the store holds.
#[derive(Clone, Debug, Default)]
pub struct SharedBlobs(Arc<Mutex<BTreeMap<String, Arc<[u8]>>>>);

impl SharedBlobs {
    fn lock(&self) -> MutexGuard<'_, BTreeMap<String, Arc<[u8]>>> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Adds bytes under `hash`; answers whether they were new. Bytes that do not hash to `hash`
    /// are refused.
    pub fn insert(&self, hash: &str, bytes: &[u8]) -> Result<bool, String> {
        if sha256_hex(bytes) != hash {
            return Err(format!("damaged: blob {hash} does not hash to its name"));
        }
        let mut store = self.lock();
        if store.contains_key(hash) {
            return Ok(false);
        }
        store.insert(hash.to_owned(), Arc::from(bytes));
        Ok(true)
    }

    pub fn contains(&self, hash: &str) -> bool {
        self.lock().contains_key(hash)
    }

    /// Every hash the store holds, sorted.
    pub fn hashes(&self) -> Vec<String> {
        self.lock().keys().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }
}

impl BlobSource for SharedBlobs {
    fn get(&self, hash: &str) -> Option<Arc<[u8]>> {
        self.lock().get(hash).cloned()
    }
}

impl BlobSink for SharedBlobs {
    fn put(&mut self, bytes: &[u8], mime: &str) -> BlobRef {
        let hash = sha256_hex(bytes);
        self.lock().entry(hash.clone()).or_insert_with(|| Arc::from(bytes));
        BlobRef { hash, mime: mime.to_owned(), size: bytes.len() as u64 }
    }
}

/// The blob bundle of `blobs`.
pub fn encode_bundle(blobs: &[(String, Arc<[u8]>)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(blobs.iter().map(|(_, bytes)| 72 + bytes.len()).sum());
    for (hash, bytes) in blobs {
        out.extend_from_slice(hash.as_bytes());
        out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        out.extend_from_slice(bytes);
    }
    out
}

/// The blobs of a bundle; refuses a truncated bundle, a malformed hash, and bytes that do not hash
/// to their name.
pub fn decode_bundle(bundle: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < bundle.len() {
        let hash = bundle.get(at..at + 64).ok_or("damaged: the blob bundle ends inside a hash")?;
        let hash = std::str::from_utf8(hash).map_err(|_| "damaged: a blob hash is not text")?;
        if !hash.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
            return Err(format!("damaged: {hash:?} is not a lower-case SHA-256"));
        }
        let length = bundle.get(at + 64..at + 72).ok_or("damaged: the blob bundle ends inside a length")?;
        let mut le = [0u8; 8];
        le.copy_from_slice(length);
        let length = usize::try_from(u64::from_le_bytes(le)).map_err(|_| "damaged: a blob is too large")?;
        let start = at + 72;
        let end = start.checked_add(length).ok_or("damaged: a blob is too large")?;
        let bytes = bundle.get(start..end).ok_or("damaged: the blob bundle ends inside a blob")?;
        if sha256_hex(bytes) != hash {
            return Err(format!("damaged: blob {hash} does not hash to its name"));
        }
        out.push((hash.to_owned(), bytes.to_vec()));
        at = end;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundle_round_trips_and_refuses_damage() {
        let one: Arc<[u8]> = Arc::from(&b"\x89PNG one"[..]);
        let two: Arc<[u8]> = Arc::from(&b"two"[..]);
        let blobs = vec![(sha256_hex(&one), one.clone()), (sha256_hex(&two), two.clone())];
        let bundle = encode_bundle(&blobs);
        let back = decode_bundle(&bundle).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].1, one.to_vec());
        assert!(decode_bundle(&[]).unwrap().is_empty());
        assert!(decode_bundle(&bundle[..bundle.len() - 1]).is_err(), "truncated");
        let mut wrong = bundle.clone();
        let last = wrong.len() - 1;
        wrong[last] ^= 1;
        assert!(decode_bundle(&wrong).is_err(), "bytes that do not hash to their name");
        let mut huge = sha256_hex(b"x").into_bytes();
        huge.extend_from_slice(&u64::MAX.to_le_bytes());
        assert!(decode_bundle(&huge).is_err(), "a length past the end never panics");
    }

    #[test]
    fn the_store_is_shared_by_clones_and_checks_hashes() {
        let store = SharedBlobs::default();
        let mut sink = store.clone();
        let blob = sink.put(b"bytes", "image/png");
        assert!(store.contains(&blob.hash));
        assert_eq!(store.get(&blob.hash).unwrap().as_ref(), b"bytes");
        assert!(!store.insert(&blob.hash, b"bytes").unwrap(), "already there");
        assert!(store.insert(&blob.hash, b"other").is_err());
    }
}
