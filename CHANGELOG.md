# Changelog

Changes to Kition Scene, newest first. A version is the kit package version in `kit.json`;
each package is published with a `package-v<version>` tag.

## Unreleased

- A `.kscene` file holds the scene's content, not its history (kition-sdk rule M16b): `content.kdm`, a sealed content snapshot (the state, the same document id and object ids, of the history only a `system` seal), the meshes and images the scene uses, and `meta.json` (format version 1, kit, KDM and schema versions, document id). No deleted objects or text, no overwritten values, no names of the people or agents who edited; the history stays in the host's history store. Files of the earlier layout (`replica.kdm` with its history) are not read.
- The view, selection, current frame and UV options are no longer saved in the `.kscene` (no `editor.json`): they are the person's view state, which the editor keeps with a host that serves `view-state` (Kit UI protocol section 7n). Two sessions that open the same file and save it without an edit write the same bytes.
- A save the host did not prepare (`prepareSave`) after an edit is refused; the temporary fallback that sealed a copy is gone.
- A scene opens on the host's stored replica with its file taken in: as it is when the store has the file's version, with the file's changes when the store missed only them, and otherwise with the file's state written by object id as one import step. A file another program saved is taken in the same way.

## 0.1.4

- The same kit as 0.1.3, built on kition-sdk 6672663 (the replica's undo passes over an undo group whose changes came to nothing): the 0.1.3 package was built on kition-sdk 6b4d471.

## 0.1.3

- A host tells a scene file's document id (`document.identity`) and makes a copied scene a new document (`document.fork`): the fork keeps the objects and their ids, the editor's view and selection, and carries a new id, none of the source's history and only the blobs the scene uses (Kit Process Protocol section 5.6, kition-sdk 6b4d471). Two copies of the same `.kscene` were one document until now.
- The headless program speaks Kit Process Protocol version 1 (`headless.protocol` is `kition-kit/1`), as hosts on kition-sdk from bb1b07b require.
- kition-sdk 6672663: KDM undo and redo pass over a group whose changes came to nothing, so an undo step that changed nothing no longer stops undo.

## 0.1.2

- A scene opened in a dark host shows no light frame first: the viewport is ready only once the engine has the host's scheme and has drawn with it.
- `kit.json` declares the engine's editor state as `engine.editorState`, so a host that does not run the engine's Rust code reads `inspect()` without it.

## 0.1.1

- The viewport takes no clicks before the document is its size: a click right after opening no longer lands in a picture framed for the old size (selecting nothing and clearing the selection).
- Cmd+S after deleting an object from the outliner saves the document.
- The viewport follows the host's dark scheme: its background, grid floor and the shade outside a camera's frame are drawn dark in a dark host, live both ways.

## 0.1.0

First public release of the source, under the Apache License 2.0. It contains:

- The Rust engine: objects, meshes and their edit operations, modifiers, Geometry Nodes, materials
  and shader nodes, lights and cameras, keyframes, drivers, the NLA, armatures, sculpting, UVs,
  particles and hair, rigid bodies and cloth, a software renderer and a path tracer, and a
  compositor; glTF, OBJ, STL, FBX and USD import and export, and `.blend` import.
- The kit engine (`kition-scene-kit`), native and WebAssembly: the scene is a KDM replica,
  replicas merge concurrent edits and repair what a merge leaves dangling, and undo is per person
  and per agent. A check catalogue reports what a scene cannot render as meant (zero scale,
  inside-out meshes, a missing camera or light, objects out of the shot), with the commands that
  fix it.
- The headless kit (`kition-scene`) for agents over the Kit Protocol: it reads every object, runs
  the editor's own commands on the objects, time and mesh elements a call names, and previews the
  scene from the camera or any view. A write names the revision it read and is refused when
  someone else changed what it rests on.
- The kit's editor (`web/`): the viewport with solid, material preview and rendered shading, the
  outliner and properties, the timeline and graph editor, and the modifier, material, physics and
  particle panels, in kit-editor's frame.
- NOTICE and docs/attribution.md list the Inter font, the colour-management sources, and Loro
  (MIT), bundled by the editor and linked by the engine.
