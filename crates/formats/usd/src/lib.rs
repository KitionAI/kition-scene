//! USD for Kition scenes: USDA and binary USDC layers read, USDA and USDC written, and USDZ packages
//! (an uncompressed zip whose first file is the root layer, beside the images its materials
//! use) read and written.
#![forbid(unsafe_code)]

mod export;
mod import;
mod lz4;
mod skel;
pub mod usda;
pub mod usdc;
mod zip;

use kition_scene_exchange::Imported;
use kition_scene_model::SceneError;

/// Reads a USD file: USDA text, a binary USDC layer, or a USDZ package whose first layer is
/// either (its materials' images read from the package).
pub fn read(bytes: &[u8]) -> Result<Imported, SceneError> {
    if bytes.starts_with(b"PK\x03\x04") {
        let files = zip::read(bytes)?;
        let (name, layer) = files.first().ok_or_else(|| usda::invalid("a damaged USDZ package"))?;
        let name = name.to_ascii_lowercase();
        if !(name.ends_with(".usda") || name.ends_with(".usd") || name.ends_with(".usdc")) {
            return Err(usda::invalid("a USDZ package whose first file is not a USD layer"));
        }
        if layer.starts_with(b"PK\x03\x04") {
            return Err(usda::invalid("a USDZ package inside a USDZ package"));
        }
        return read_layer(layer, &files);
    }
    read_layer(bytes, &[])
}

fn read_layer(bytes: &[u8], assets: &[(String, &[u8])]) -> Result<Imported, SceneError> {
    if bytes.starts_with(usdc::MAGIC) {
        return import::import(&usdc::parse(bytes)?, assets);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| usda::invalid("the file is neither USDA text nor a USDC or USDZ file"))?;
    import::import(&usda::parse(text)?, assets)
}

/// Writes the scene's visible objects (or `only` and their ancestors) as USDA.
pub fn write(scene: &kition_scene_model::Scene, only: Option<&[kition_scene_model::Id]>) -> Result<Vec<u8>, SceneError> {
    export::export(scene, only, false).map(|(text, _)| text.into_bytes())
}

/// Writes the same authored scene as [`write`] in binary USDC (crate 0.8.0).
pub fn write_binary(scene: &kition_scene_model::Scene, only: Option<&[kition_scene_model::Id]>) -> Result<Vec<u8>, SceneError> {
    let (text, _) = export::export(scene, only, false)?;
    usdc::write(&usda::parse(&text)?)
}

/// Writes the scene's visible objects (or `only` and their ancestors) as a USDZ package: the
/// USDA layer first, then the images its materials use.
pub fn write_package(scene: &kition_scene_model::Scene, only: Option<&[kition_scene_model::Id]>) -> Result<Vec<u8>, SceneError> {
    let (text, images) = export::export(scene, only, true)?;
    let mut files: Vec<(String, &[u8])> = vec![("scene.usda".to_string(), text.as_bytes())];
    for (path, hash) in &images {
        if let Some(image) = scene.image(hash) {
            files.push((path.clone(), image.bytes.as_slice()));
        }
    }
    zip::write(&files)
}
