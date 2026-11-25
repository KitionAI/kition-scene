//! The `.kscene` file (`docs/design.md` section 7.5): a ZIP holding `mimetype` (stored first, so
//! the file identifies itself), `meta.json` (what identifies the content: the format, its version,
//! the kit, the KDM version, the schema version and the document id), `content.json` (the scene's
//! content, DEFLATE at a fixed level: the KDM content encoding, its state and nothing else;
//! kition-sdk data model 7.3 and 7.4, rule M16b), `blobs/<sha256>` (the meshes and images the state
//! references). Nothing else: no editor state (a person's view, selection and current frame are
//! the host's view state, Kit UI protocol section 7n), no preview image. Writing is deterministic
//! for the same parts, so two sessions that save the same content write the same bytes.
#![forbid(unsafe_code)]

use std::io::{Cursor, Read, Write};

use zip::write::SimpleFileOptions;

/// The file's media type, also its `mimetype` entry.
pub const MIME: &str = "application/vnd.kition.scene+zip";
/// The format this crate writes and reads (`format` in `meta.json`).
pub const FORMAT: &str = "kition-scene";
/// The kit whose documents the file holds (`kit` in `meta.json`).
pub const KIT: &str = "kit.scene";
/// The format version this crate writes and reads (`formatVersion` in `meta.json`).
pub const VERSION: u32 = 1;
/// The largest file part read (512 MiB).
const MAX_ENTRY: u64 = 512 * 1024 * 1024;
/// The most blobs a file may hold.
const MAX_BLOBS: usize = 100_000;
/// The entry that holds the content.
const CONTENT: &str = "content.json";
/// The DEFLATE level of `content.json`: fixed, so the same content is the same bytes for one build.
const CONTENT_LEVEL: i64 = 6;

/// What identifies a file's content (`meta.json`, besides `format`, `formatVersion` and `kit`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Meta {
    /// The KDM version the content is written in.
    pub kdm: u32,
    /// The scene schema's version.
    pub schema_version: u32,
    /// The document id of the content.
    pub doc_id: String,
}

/// A `.kscene` file's parts.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct File {
    /// What identifies the content.
    pub meta: Meta,
    /// The content: the replica's content (`Replica::content_snapshot`, `content.json`).
    pub content: Vec<u8>,
    /// Blobs by their SHA-256 (lowercase hex), sorted by hash.
    pub blobs: Vec<(String, Vec<u8>)>,
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
    let meta = serde_json::to_vec_pretty(&serde_json::json!({
        "format": FORMAT,
        "formatVersion": VERSION,
        "kit": KIT,
        "kdm": file.meta.kdm,
        "schemaVersion": file.meta.schema_version,
        "docId": file.meta.doc_id,
    }))
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
    put("meta.json", deflated, &meta)?;
    put(CONTENT, deflated.compression_level(Some(CONTENT_LEVEL)), &file.content)?;
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
/// bytes checks they hash to their name. A file without `meta.json` or `content.json` is damaged.
pub fn read(bytes: &[u8]) -> Result<File, Error> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| invalid("not a .kscene file (not a ZIP)"))?;
    if entry(&mut archive, "mimetype")? != MIME.as_bytes() {
        return Err(invalid("not a .kscene file (wrong mimetype)"));
    }
    let meta: serde_json::Value =
        serde_json::from_slice(&entry(&mut archive, "meta.json")?).map_err(|error| invalid(format!("meta.json: {error}")))?;
    if meta.get("format").and_then(serde_json::Value::as_str) != Some(FORMAT) {
        return Err(invalid("meta.json is not a Kition Scene file's"));
    }
    let version = meta.get("formatVersion").and_then(serde_json::Value::as_u64).unwrap_or(0);
    if version != u64::from(VERSION) {
        return Err(invalid(format!("the file is version {version}; this kit reads version {VERSION}")));
    }
    if meta.get("kit").and_then(serde_json::Value::as_str) != Some(KIT) {
        return Err(invalid("meta.json names another kit"));
    }
    let number = |key: &str| meta.get(key).and_then(serde_json::Value::as_u64).and_then(|value| u32::try_from(value).ok());
    let meta = Meta {
        kdm: number("kdm").ok_or_else(|| invalid("meta.json has no kdm version"))?,
        schema_version: number("schemaVersion").ok_or_else(|| invalid("meta.json has no schema version"))?,
        doc_id: meta
            .get("docId")
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| invalid("meta.json names no document"))?
            .to_owned(),
    };
    let content = entry(&mut archive, CONTENT)?;
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
    Ok(File { meta, content, blobs })
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
            meta: Meta { kdm: 1, schema_version: 1, doc_id: "doc".into() },
            content: b"content bytes".to_vec(),
            blobs: vec![("a".repeat(64), b"mesh".to_vec()), ("b".repeat(64), b"image".to_vec())],
        }
    }

    #[test]
    fn a_file_round_trips_byte_for_byte() {
        for file in [sample(), File { blobs: Vec::new(), ..sample() }] {
            let bytes = write(&file).unwrap();
            assert!(sniff(&bytes));
            let back = read(&bytes).unwrap();
            assert_eq!(back, file);
            assert_eq!(write(&back).unwrap(), bytes);
        }
    }

    /// The file holds the content and what identifies it, nothing of a person's session (no
    /// `editor.json`: view state is the host's, Kit UI protocol section 7n) and no preview image.
    #[test]
    fn the_file_holds_only_the_content_its_identity_and_its_blobs() {
        let bytes = write(&sample()).unwrap();
        let archive = zip::ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();
        let names: Vec<&str> = archive.file_names().collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        let blob_a = format!("blobs/{}", "a".repeat(64));
        let blob_b = format!("blobs/{}", "b".repeat(64));
        let mut wanted = vec!["content.json", "meta.json", "mimetype", blob_a.as_str(), blob_b.as_str()];
        wanted.sort_unstable();
        assert_eq!(sorted, wanted, "{names:?}");
        assert_eq!(names.first(), Some(&"mimetype"), "mimetype comes first");
    }

    #[test]
    fn meta_json_holds_only_what_identifies_the_content() {
        let bytes = write(&sample()).unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();
        let meta: serde_json::Value = serde_json::from_slice(&entry(&mut archive, "meta.json").unwrap()).unwrap();
        let keys: Vec<&str> = meta.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys.len(), 6, "{meta}");
        for key in ["format", "formatVersion", "kit", "kdm", "schemaVersion", "docId"] {
            assert!(keys.contains(&key), "{key} in {meta}");
        }
        assert_eq!(meta["formatVersion"], 1);
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
        let mut nameless = sample();
        nameless.meta.doc_id.clear();
        assert!(read(&write(&nameless).unwrap()).is_err(), "a file that names no document");
    }

    #[test]
    fn a_file_without_its_content_is_damaged() {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(MIME.as_bytes()).unwrap();
        zip.start_file("meta.json", stored).unwrap();
        zip.write_all(br#"{"format":"kition-scene","formatVersion":1,"kit":"kit.scene","kdm":1,"schemaVersion":1,"docId":"doc"}"#).unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        let error = read(&bytes).unwrap_err();
        assert!(error.0.contains("content.json"), "{error}");
    }
}
