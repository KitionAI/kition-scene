//! The parameters of this kit's commands that name files or folders on disk (KHP A28), declared
//! as typed files in the commands' schemas (`kition_kit::files`): a host lends them, and a
//! headless kit keeps those commands from agents (the host opens, saves and exports documents).

use kition_kit::files::FileParam;

/// Every file or folder parameter of the kit's commands.
/// The scene formats `file.import` reads, and the pictures images take (`ImageFormat`; the world's
/// environment also takes Radiance `.hdr`).
const SCENES: &[&str] = &["glb", "gltf", "obj", "stl", "fbx", "usd", "usda", "usdc", "usdz", "blend"];
const PICTURES: &[&str] = &["png", "jpg", "jpeg"];
const ENVIRONMENTS: &[&str] = &["png", "jpg", "jpeg", "hdr"];

pub const FILES: &[FileParam] = &[
    FileParam::read("file.import", "path", SCENES),
    FileParam::read("material.setTexture", "path", PICTURES),
    FileParam::read("modifier.setImage", "path", PICTURES),
    FileParam::read("world.setImage", "path", ENVIRONMENTS),
    FileParam::read("compositor.setImage", "path", PICTURES),
];

#[cfg(test)]
mod tests {
    use kition_kit::KitDocument;

    use super::*;

    /// Parameters whose names read like files but are not: ids, property paths, directions,
    /// folders of the kit's own panels, in-memory data.
    const NOT_FILES: &[(&str, &str)] = &[
        ("graph.moveKey", "path"),
        ("graph.deleteKey", "path"),
        ("graph.setInterpolation", "path"),
        ("graph.addModifier", "path"),
        ("graph.setModifier", "path"),
        ("graph.removeModifier", "path"),
    ];

    #[test]
    fn every_file_parameter_is_declared() {
        let doc = crate::SceneDocument::create("Untitled.kscene", 64, 64).unwrap_or_else(|error| panic!("{error}"));
        let problems = kition_kit::files::check(&doc.commands(), FILES, NOT_FILES);
        assert!(problems.is_empty(), "{problems:#?}");
    }
}
