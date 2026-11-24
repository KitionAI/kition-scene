//! Regression: a new document's viewport is the width and height it
//! was created with (the editor resizes it later with view.resize); zero or out of range keeps
//! the default.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;

#[test]
fn a_new_document_has_the_size_it_was_made_with() {
    let mut doc = SceneDocument::create("Wide.kscene", 1920, 640).unwrap();
    assert_eq!((doc.width(), doc.height()), (1920, 640));
    assert_eq!(doc.render(0.0, 0.0, 1.0, 1920, 640).unwrap().len(), 1920 * 640 * 4);
    // The viewport's size is the editor's, not the file's: reopened, the scene has the default
    // size until the editor resizes it, and the file is the same whatever the size.
    let bytes = doc.save_file(None).unwrap();
    let mut reopened = SceneDocument::open("Wide.kscene", &bytes).unwrap();
    assert_eq!((reopened.width(), reopened.height()), (1280, 720));
    assert_eq!(reopened.save_file(None).unwrap(), bytes, "the viewport's size is not in the file");
    let doc = SceneDocument::create("Default.kscene", 0, 0).unwrap();
    assert_eq!((doc.width(), doc.height()), (1280, 720), "no size: the default viewport");
}
