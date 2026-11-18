//! Regression: a new document's viewport is the width and height it
//! was created with (the editor resizes it later with view.resize); zero or out of range keeps
//! the default.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use kition_kit::KitDocument;
use kition_scene_kit::SceneDocument;

#[test]
fn a_new_document_has_the_size_it_was_made_with() {
    let doc = SceneDocument::create("Wide.kscene", 1920, 640).unwrap();
    assert_eq!((doc.width(), doc.height()), (1920, 640));
    assert_eq!(doc.render(0.0, 0.0, 1.0, 1920, 640).unwrap().len(), 1920 * 640 * 4);
    // Reopened, it is the size it was saved at.
    let reopened = SceneDocument::open("Wide.kscene", &doc.save(None).unwrap()).unwrap();
    assert_eq!((reopened.width(), reopened.height()), (1920, 640));
    let doc = SceneDocument::create("Default.kscene", 0, 0).unwrap();
    assert_eq!((doc.width(), doc.height()), (1280, 720), "no size: the default viewport");
}
