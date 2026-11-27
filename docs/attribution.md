# Attribution

The Scene kit is Kition's own code (Apache-2.0, [LICENSE](../LICENSE)).
The engine links these libraries; every one is shipped inside the kit's WebAssembly module and the
headless `kition-scene` executable.

| Library | Licence | Through |
|---|---|---|
| Kition SDK (`kition-sdk`, `kition-headless`, `kition-kit-server`) | Apache-2.0 | direct, pinned by commit in `Cargo.toml` |
| glam | MIT OR Apache-2.0 | direct (vectors and matrices) |
| serde, serde_json | MIT OR Apache-2.0 | direct |
| zip | MIT | direct (the `.kscene` container) |
| png | MIT OR Apache-2.0 | direct (rendered images) |
| thiserror | MIT OR Apache-2.0 | direct |

Blender (GPL) is used only as a reference application to check behaviour and output; none of its
code is in this kit.

## Bundled font

| Font | Licence | Used for |
|---|---|---|
| Inter (`crates/engine/model/fonts/inter/Inter-Variable.ttf`), copyright 2020 The Inter Project Authors (https://github.com/rsms/inter), from kition-fonts | OFL-1.1 (`crates/engine/model/fonts/inter/OFL.txt`) | text objects' letters, compiled into the engine |

| Library | Licence | Through |
|---|---|---|
| ttf-parser | MIT OR Apache-2.0 | direct (glyph outlines for text objects) |

## Drawn in the editor

| What | Made by | Notes |
|---|---|---|
| The navigation gizmo (`web/src/NavGizmo.tsx`: axis balls, rings, arms and the hover disc) | Kition, drawn as SVG in the editor | Its layout and axis colours (#ff3352, #8bdc00, #2890ff) follow what Blender's default theme shows, recorded by running the application; its buttons use Tessera's icons |
| The transform gizmos, the 3D cursor and the snap mark (`web/src/TransformGizmo.ts`: Move's arrows, squares and middle ring, Rotate's axis rings and view ring, Scale's box-ended axes; the red and white cursor ring with its cross) | Kition, drawn on a 2D canvas over the viewport | Their layout, the 75 px size and the axis colours follow what Blender's default theme and preferences show, recorded by running the application; the snapping toggle uses Tessera's magnet icon |
| The viewport header's select-mode icons and the shading pie (`web/src/ViewportHeader.tsx`: a square with its corners, one bold side, or filled; `web/src/PieMenu.tsx`: four items round a ring with a notch) | Kition, drawn as SVG and CSS in the editor | Their layout follows what Blender's header and Z pie show, recorded by running the application; the shading and overlays buttons and the toolbar's operation buttons use Tessera's icons |
