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
