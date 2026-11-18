//! The `.kscene` file (`docs/design.md` section 7.4): a ZIP holding `mimetype` (stored first, so
//! the file identifies itself), `editor.json` (the format, its version, and the editor's state:
//! view, selection, current frame), `replica.kdm` (the scene's KDM replica snapshot, with its
//! history), and `blobs/<sha256>` (every mesh and image the replica references). Writing is
//! deterministic for the same parts.
#![forbid(unsafe_code)]

use std::io::{Cursor, Read, Write};

use zip::write::SimpleFileOptions;

/// The file's media type, also its `mimetype` entry.
pub const MIME: &str = "application/vnd.kition.scene+zip";
/// The version this crate writes and reads (`version` in `editor.json`).
pub const VERSION: u32 = 4;
/// The largest file part read (512 MiB).
const MAX_ENTRY: u64 = 512 * 1024 * 1024;
/// The most blobs a file may hold.
const MAX_BLOBS: usize = 100_000;

/// A `.kscene` file's parts.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct File {
    /// The replica snapshot.
    pub snapshot: Vec<u8>,
    /// Blobs by their SHA-256 (lowercase hex), sorted by hash.
    pub blobs: Vec<(String, Vec<u8>)>,
    /// The editor's state.
    pub editor: Option<serde_json::Value>,
}

/// Why a file did not read or write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid-file: {}", self.0)
    }
}

impl std::error::Error for Error {}

fn invalid(message: impl Into<String>) -> Error {
    Error(message.into())
}

/// Writes a file.
pub fn write(file: &File) -> Result<Vec<u8>, Error> {
    let editor = serde_json::to_vec_pretty(&serde_json::json!({ "format": "kition-scene", "version": VERSION, "editor": file.editor }))
        .map_err(|error| invalid(error.to_string()))?;
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let fixed = zip::DateTime::default();
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored).last_modified_time(fixed);
    let deflated = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).last_modified_time(fixed);
    let mut put = |name: &str, options: SimpleFileOptions, bytes: &[u8]| -> Result<(), Error> {
        zip.start_file(name, options).map_err(|error| invalid(error.to_string()))?;
        zip.write_all(bytes).map_err(|error| invalid(error.to_string()))
    };
    put("mimetype", stored, MIME.as_bytes())?;
    put("editor.json", deflated, &editor)?;
    put("replica.kdm", deflated, &file.snapshot)?;
    let mut blobs: Vec<&(String, Vec<u8>)> = file.blobs.iter().collect();
    blobs.sort_by(|a, b| a.0.cmp(&b.0));
    blobs.dedup_by(|a, b| a.0 == b.0);
    for (hash, bytes) in blobs {
        put(&format!("blobs/{hash}"), deflated, bytes)?;
    }
    let cursor = zip.finish().map_err(|error| invalid(error.to_string()))?;
    Ok(cursor.into_inner())
}

/// Whether the bytes start like a `.kscene` file.
pub fn sniff(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04")
        && bytes.get(30..38) == Some(b"mimetype")
        && bytes.windows(MIME.len()).take(128).any(|window| window == MIME.as_bytes())
}

/// Reads a file's parts. Blob names must be lowercase SHA-256 hex; the store that takes the
/// bytes checks they hash to their name.
pub fn read(bytes: &[u8]) -> Result<File, Error> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| invalid("not a .kscene file (not a ZIP)"))?;
    if entry(&mut archive, "mimetype")? != MIME.as_bytes() {
        return Err(invalid("not a .kscene file (wrong mimetype)"));
    }
    let envelope: serde_json::Value =
        serde_json::from_slice(&entry(&mut archive, "editor.json")?).map_err(|error| invalid(format!("editor.json: {error}")))?;
    if envelope.get("format").and_then(serde_json::Value::as_str) != Some("kition-scene") {
        return Err(invalid("editor.json is not a Kition Scene file's"));
    }
    let version = envelope.get("version").and_then(serde_json::Value::as_u64).unwrap_or(0);
    if version != u64::from(VERSION) {
        return Err(invalid(format!("the file is version {version}; this kit reads version {VERSION}")));
    }
    let editor = envelope.get("editor").filter(|value| !value.is_null()).cloned();
    let snapshot = entry(&mut archive, "replica.kdm")?;
    let names: Vec<String> = archive.file_names().filter(|name| name.starts_with("blobs/")).map(str::to_owned).collect();
    if names.len() > MAX_BLOBS {
        return Err(invalid("the file holds too many blobs"));
    }
    let mut blobs = Vec::with_capacity(names.len());
    for name in names {
        let hash = name.trim_start_matches("blobs/").to_owned();
        if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
            return Err(invalid(format!("{name} is not a blob name")));
        }
        let bytes = entry(&mut archive, &name)?;
        blobs.push((hash, bytes));
    }
    blobs.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(File { snapshot, blobs, editor })
}

fn entry(archive: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<Vec<u8>, Error> {
    let file = archive.by_name(name).map_err(|_| invalid(format!("the file has no {name}")))?;
    if file.size() > MAX_ENTRY {
        return Err(invalid(format!("{name} is too large")));
    }
    let mut out = Vec::with_capacity(usize::try_from(file.size()).unwrap_or(0));
    file.take(MAX_ENTRY + 1).read_to_end(&mut out).map_err(|error| invalid(format!("{name}: {error}")))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> File {
        File {
            snapshot: b"snapshot bytes".to_vec(),
            blobs: vec![("a".repeat(64), b"mesh".to_vec()), ("b".repeat(64), b"image".to_vec())],
            editor: Some(serde_json::json!({ "frame": 12 })),
        }
    }

    #[test]
    fn a_file_round_trips_byte_for_byte() {
        let bytes = write(&sample()).unwrap();
        assert!(sniff(&bytes));
        let back = read(&bytes).unwrap();
        assert_eq!(back, sample());
        assert_eq!(write(&back).unwrap(), bytes);
    }

    #[test]
    fn damaged_files_are_refused() {
        assert!(read(b"not a scene").is_err());
        let bytes = write(&sample()).unwrap();
        for cut in [10, bytes.len() / 2, bytes.len() - 1] {
            assert!(read(&bytes[..cut]).is_err(), "cut at {cut}");
        }
        let mut wrong = sample();
        wrong.blobs.push(("not-a-hash".into(), Vec::new()));
        assert!(read(&write(&wrong).unwrap()).is_err());
    }
}
