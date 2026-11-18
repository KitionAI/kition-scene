//! The exchange formats `file.import` and `file.export` take: glTF, OBJ, and STL (the exchange
//! crate), FBX, and USD.

use kition_scene_exchange::{Format, Imported};
use kition_scene_model::{Id, Scene};

/// Formats `file.export` writes (besides `png`).
pub const EXPORTS: &[&str] = &["glb", "gltf", "obj", "stl", "fbx", "usda", "usdc", "usdz"];
/// Formats `file.import` reads (besides `blend`).
pub const IMPORTS: &[&str] = &["glb", "gltf", "obj", "stl", "fbx", "usd", "usda", "usdc", "usdz"];

#[derive(Debug, Clone, Copy)]
pub enum Exchange {
    Basic(Format),
    Fbx,
    /// USDA text.
    Usd,
    /// A binary USD crate layer.
    Usdc,
    /// A USDZ package: its layer and the images its materials use.
    Usdz,
}

impl Exchange {
    /// The format of an extension (without the dot, any case).
    pub fn from_extension(extension: &str) -> Option<Self> {
        let extension = extension.trim_start_matches('.').to_ascii_lowercase();
        match extension.as_str() {
            "fbx" => Some(Self::Fbx),
            "usd" | "usda" => Some(Self::Usd),
            "usdc" => Some(Self::Usdc),
            "usdz" => Some(Self::Usdz),
            other => Format::from_extension(other).map(Self::Basic),
        }
    }

    pub fn import(self, bytes: &[u8]) -> Result<Imported, String> {
        match self {
            Self::Basic(format) => kition_scene_exchange::import(bytes, format),
            Self::Fbx => kition_scene_fbx::read(bytes),
            Self::Usd | Self::Usdc | Self::Usdz => kition_scene_usd::read(bytes),
        }
        .map_err(|error| error.to_string())
    }

    /// The file's bytes and media type.
    pub fn export(self, scene: &Scene, only: Option<&[Id]>) -> Result<(Vec<u8>, &'static str), String> {
        match self {
            Self::Basic(format) => kition_scene_exchange::export(scene, format, only).map(|bytes| (bytes, format.mime())),
            Self::Fbx => kition_scene_fbx::write(scene, only).map(|bytes| (bytes, "application/octet-stream")),
            Self::Usd => kition_scene_usd::write(scene, only).map(|bytes| (bytes, "model/vnd.usda")),
            Self::Usdc => kition_scene_usd::write_binary(scene, only).map(|bytes| (bytes, "application/octet-stream")),
            Self::Usdz => kition_scene_usd::write_package(scene, only).map(|bytes| (bytes, "model/vnd.usdz+zip")),
        }
        .map_err(|error| error.to_string())
    }
}

/// "a, b, or c".
pub fn listed(names: &[&str]) -> String {
    match names.split_last() {
        Some((last, [])) => (*last).to_string(),
        Some((last, rest)) => format!("{}, or {last}", rest.join(", ")),
        None => String::new(),
    }
}
