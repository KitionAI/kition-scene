# Development

## Repository layout

| Path | Contents |
|---|---|
| `crates/` | The engine (Rust), in groups: `engine/` (`model`, the document model; `nodes`, `shader`, `compositor`, `particles`, `physics`, `sequencer`), `formats/` (`kscene`, `exchange` for glTF/OBJ/STL, `usd`, `fbx`, `blend`), `render/` (`raster`, the software renderer; `trace`, the path tracer) and `kdm/` (`kdm-model`, the scene as a KDM replica) |
| `bindings/` | `wasm`: `kition-scene-kit`, the engine behind Kition's `KitDocument` contract, compiled to WebAssembly for the editor; `headless`: `kition-scene`, the headless kit executable for Kition's agent |
| `web/` | The kit's editor (React, on kition-sdk's `@kition/kit-editor`); `web/dev.html` is a host for development |
| `kit.json` | The kit manifest (KHP) |
| `agent/` | The agent guide (`guide.md`) and checks |
| `conformance/` | The agent conformance fixtures (`.kscene` scenes) that cases and tests refer to |
| `features.yaml`, `cases/` | The feature catalogue and its cases ([quality.md](quality.md)) |
| `docs/` | These documents: the entry documents, `features/` (one document per feature) and `parity/` (the comparison with the target application) |
| `tools/` | `tools/xtask` (`cargo xtask ci`, `wasm`, `manifest` and `ai`) and `tools/blender-reference` (records reference facts from a local copy of the reference application) |

## Build and check

```sh
cargo xtask ci                                            # the check every change passes
cargo build --release -p kition-scene                     # the headless kit executable
cd web && pnpm install && pnpm engine && pnpm test:flow   # the editor
```

`cargo xtask` is an alias in `.cargo/config.toml` for `cargo run --package xtask --`. The design
of the engine is in [design.md](design.md); the rules for every change are in
[AGENTS.md](../AGENTS.md) and [quality.md](quality.md).

By default `pnpm install` in `web/` installs the published `@kition/*` packages from npm, at the
versions the committed `web/pnpm-lock.yaml` pins, so the editor needs no other checkout. To work
against local checkouts instead, run `KITION_LINK=1 pnpm install` in `web/`: `web/.pnpmfile.cjs`
then links `@kition/kit-editor`, `@kition/kit-host`, `@kition/kit-ui-protocol` and
`@kition/kit-dev-host` from the kition-sdk checkout and `@kition/tessera-icons` from the
kition-tessera checkout next to this repository (or the ones `KITION_SDK` and `KITION_TESSERA`
name), each one whose package is there. The linked install rewrites `web/pnpm-lock.yaml`; do not
commit that lockfile.

`KITION_BLENDER=<path to the Blender executable> cargo test -p kition-scene-exchange` also runs
the cross-checks against Blender's importers and exporters.

`pnpm engine` (`web/scripts/build-engine.sh`) builds the engine to WebAssembly into
`web/public/engine/`; `pnpm dev` serves the editor in the development host (`web/dev.html`).

The headless executable serves the kit's tools to any MCP client through kition-sdk's
`kition-kits-mcp`:

```sh
cargo build --release -p kition-scene                  # target/release/kition-scene
# in a kition-sdk checkout:
cargo build -p kition-kits --bin kition-kits-mcp
target/debug/kition-kits-mcp --workspace <folder> --kit <kition-scene>/target/release/kition-scene
```

[agent/guide.md](../agent/guide.md) explains the tools, and [writing-format.md](writing-format.md)
lists every command.

### The viewport and the file

The editor draws solid, wireframe and material views on the GPU (WebGL2), with the engine's
software viewport as reference and fallback (`localStorage['kition.scene.gl'] = 'off'` keeps the
software one). Rendered shading progressively traces scene lighting in the worker, reusing the
prepared scene between samples while keeping picking and overlays at full resolution.

A `.kscene` file is a ZIP containing `mimetype`, `replica.kdm` (the scene and its editing history),
content-addressed mesh and image data under `blobs/`, and local view settings in `editor.json`.
The engine contract is 2: the scene is a KDM replica.

## Take the README screenshots

The pictures under `assets/readme/` are taken by `web/scripts/readme-shots.mjs` on kition-sdk's
shared runner (`tools/readme-shots`): it opens the showcase sample `glazed-donut-sprinkles` from a
[kition-showcase](https://github.com/KitionAI/kition-showcase) checkout next to this repository
(or the one `KITION_SHOWCASE` names) in the development host, in Classic display mode at
1440 × 900 and device scale 2, and changes it only through the editor. Headless Chromium has no
GPU, so the viewport would step aside for the software one; `CHROME` names a browser to use
instead. A wrapper that starts Playwright's headless Chromium on the machine's GPU, as
`web/tests/flow.mjs` does (macOS):

```sh
cat > /tmp/chrome-gl.sh <<'SH'
#!/bin/sh
exec "$HOME/Library/Caches/ms-playwright/chromium_headless_shell-1217/chrome-headless-shell-mac-arm64/chrome-headless-shell" \
  --use-angle=metal --enable-gpu --ignore-gpu-blocklist "$@"
SH
chmod +x /tmp/chrome-gl.sh
cd web
CHROME=/tmp/chrome-gl.sh node scripts/readme-shots.mjs           # hero, see-1 … see-4
CHROME=/tmp/chrome-gl.sh node scripts/readme-shots.mjs see-3     # only the named ones
```

Run it with the engine built in `web/public/engine/` and no HTTP proxy set. see-1 waits until the
rendered viewport has all its samples. When `pngquant` is installed the script shrinks each
picture with `--quality 70-90 --nofs` (a README picture is at most 400 KB; without dithering, the
path tracer's noise stays small). `made-1.png` … `made-4.png` are copies of showcase samples'
`thumb.png`.

## Publish

Kition ships the kit's package, `kition-scene-<version>.tgz` (the `scene-ui/` folder), a release
asset of this repository tagged `package-v<version>`. It is built on GitHub, never on a developer
machine: the Package workflow (`.github/workflows/package.yml`, which calls Kition's reusable
`kit-package.yml`) builds the commit it is started on, with `gh workflow run package.yml -R
KitionAI/kition-scene`, and publishes the Release with the package's lock entry,
`kition-scene-<version>.json`. Kition pins it with `node scripts/pin-kits.mjs scene` (see
`docs/kits.md` there). A version is published once: bump `version` in `kit.json` to publish a
change.
