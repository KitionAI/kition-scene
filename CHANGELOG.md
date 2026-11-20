# Changelog

Changes to Kition Scene, newest first. A version is the kit package version in `kit.json`;
each package is published with a `package-v<version>` tag.

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
