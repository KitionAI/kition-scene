# Binary USD export

`file.export` with `format: "usdc"` writes an OpenUSD crate layer (0.8.0, `PXR-USDC`).
The editor offers **USD binary (.usdc)** in Export. It has the same scene scope and selected-only
behavior as USDA: hierarchy, evaluated meshes, UVs and normals, preview materials, all four
light kinds, cameras, skeletons and skin weights, blend shapes and sampled animation. Export
neither changes the scene nor adds an undo step. No KDM field or schema version changes.

USDC uses the same scene authoring as USDA, then encodes typed values and the crate's token,
string, field, fieldset, path and spec tables. Numeric arrays retain their declared precision,
including binary16 joint scales and binary64 transforms. Quaternions store their real component
last in the binary representation. Uniform/custom flags, Kition Scene's nested boolean custom data,
schema lists, relationship targets, connections and time samples remain authored. Output is
deterministic. Tables use safe LZ4 block compression and USD's two-bit delta widths: the common
delta occupies no payload, and other differences use signed 8-, 16- or 32-bit values. The common
single-byte delta is selected by frequency, with lower values winning ties. Tables over
32 MiB use literal LZ4 blocks to keep the compressor's buffer-size arithmetic within 32-bit
limits in WebAssembly. Numeric value arrays remain uncompressed.

As with raw USDA, raw USDC exports material factors without image files. Choose USDZ when an
export needs its images packaged. USDZ continues to use its USDA root layer and 64-byte-aligned
assets. Variants, references and payload composition are not added by binary export.

The encoder refuses unsupported types or metadata, non-finite and out-of-range typed numbers,
invalid or duplicate spec paths, deeply nested prims or values, and deeply nested target paths.
It caps tables at one million entries, token bytes at 64 MiB, and output at 512 MiB. Unknown
values cannot silently disappear from a binary export.

## Size and timing measurements

Measured with Rust 1.95.0 release builds on macOS arm64. The baseline is the same writer
without table compression: literal-only LZ4 blocks and full-width integer differences.
The scene, declared value precision, uncompressed numeric arrays and benchmark workload are
identical. The compressing writer also consumes table iterators directly instead of copying each
integer column into a temporary vector.

| Workload | Baseline bytes | Compressed bytes | Reduction | Baseline wall ms/export, median (range) | Compressed wall ms/export, median (range) |
| --- | ---: | ---: | ---: | ---: | ---: |
| Starter scene, including scene authoring and USDA parsing | 7,696 | 3,882 | 49.6% | 1.300 (0.150–4.718) | 0.309 (0.152–5.324) |
| Synthetic layer: 2,000 animated Xforms, triangle meshes and material bindings | 2,531,270 | 935,997 | 63.0% | 315.498 (169.810–666.823) | 353.429 (93.370–551.091) |

Three alternating runs of each executable give 21 timed batches per workload. Each batch
wrote the starter scene 100 times or the larger layer three times, after four untimed exports.
Layer construction and file I/O are outside timed batches. Process CPU time (user plus system,
including construction and warmup) is 1.05 seconds median, range 1.05–1.06, for the baseline
and 1.07 seconds median, range 1.06–1.12, with table compression. The machine's one-minute load
average ranged from 476 to 491 during the runs; the broad wall-time ranges do not establish a CPU or latency improvement.
The measured gain is smaller files at a small compression cost. Geometry-heavy files may gain
less because numeric value arrays remain uncompressed.

The reproducible current workload lives in `crates/formats/usd/examples/usdc_bench.rs`:

```sh
cargo +1.95.0 build --release -p kition-scene-usd --example usdc_bench
/usr/bin/time -p "${CARGO_TARGET_DIR:-target}/release/examples/usdc_bench"
```

## Evidence and clean-room sources

The implementation in `crates/formats/usd/src/usdc/` is the inverse of Kition Scene's existing
clean-room crate reader. Format terminology and the conversion model follow the published
[OpenUSD crate description](https://openusd.org/release/glossary.html#crate-file-format).
IEEE 754 binary16 rounding and the published
[LZ4 block format](https://github.com/lz4/lz4/blob/dev/doc/lz4_Block_format.md) supply the scalar
and compression rules. No Blender implementation or source is used.

`crates/formats/usd/tests/blender/usdc.py` records three Blender 5.2.3 LTS scenes through its
public Python API: the default scene; animated parented meshes, a Principled material, four
light kinds and a camera; and a skinned column with two keyed bones and keyed shape deformation,
plus a shape-keyed grid. Committed `.usdc` inputs and JSON reports under
`crates/formats/usd/tests/fixtures/usdc6/` make checks independent of a Blender installation.
The tests check frames 1, 11 and 21: matrices and deformed vertices within 0.002, normalized
weights within 0.002, and material/light/camera values within 0.1%. USDA and USDC imports also
compare every scene field, allowing one part per million for float rounding before transform
decomposition while preserving structure and integer fields exactly.

`tests/blender/usdc_stage.py` uses Blender's bundled OpenUSD 0.26.3 public `Usd.Stage` API to
record `typed.stage.json` from the synthetic `typed.usda`. Its typed defaults, half rounding
(including subnormals and ties), matrix and quaternion arrays, flags, paths, and times are
checked exactly, after treating JSON integer and float representations of the same number alike.

The Chromium and WebKit editor flow opens File → Export, selects USDC, and checks the
`PXR-USDC` header in the bytes written through the development host. Export uses the kit's
format dialog, which calls the host destination picker.

Run offline:

```sh
cargo +1.95.0 test -p kition-scene-usd --test usdc_write
```

Set `KITION_BLENDER` to the Blender executable for live checks. Each Kition Scene-emitted binary scene
is opened by Blender's USD importer and compared to its geometry report; OpenUSD separately
compares every authored property of Kition Scene's text and binary layers, including all samples and
connection/relationship targets, authored custom data and schema lists. The typed fixture is
also read directly by OpenUSD. To record new observations, run the two scripts into a temporary directory and review the resulting
fixtures before proposing any expected-result change.
