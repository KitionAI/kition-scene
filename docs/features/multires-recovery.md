# Multiresolution recovery

The Multires modifier panel and agent commands share two operators:

- `multires.unsubdivide` recovers one complete quad subdivision below the base.
  The previous base becomes level 1, and each previous sculpted level moves up
  one level without changing its visible positions. The viewport and sculpt
  level numbers stay unchanged, as in Blender; select the highest level to see
  all retained detail.
- `multires.rebuildSubdivisions` requires no existing multiresolution levels.
  It recovers as many complete levels as possible, up to six, and selects the
  highest viewport and sculpt level. Disconnected components stop at their
  common recoverable depth. One command is one undo step.

Both accept `object` (id or name), defaulting to the active or sculpted mesh,
and require a Multires modifier. They return `{object, levels, modifier}`.
They run in object or sculpt mode, with dynamic topology disabled.

## Supported topology and data

Recovery recognizes topology, independently of vertex/face ordering and world
orientation. Each input component must be an oriented manifold of quads that
is a complete subdivision of another quad mesh. A boundary corner or an
interior vertex of valence other than four identifies its subdivision phase.
Components without such an anchor, including fully regular closed tori, are
refused rather than assigned an arbitrary phase. Triangles, n-gons, loose
edges/vertices, inconsistent winding, bow ties and nonmanifold edges are refused.

The limits are six stored levels, 100,000 base faces and 250,000 faces at the
highest existing level. The algorithm propagates roles once per component;
it performs no backtracking. It validates the result with an oriented,
bijective correspondence to the input subdivision before storing anything.

Positions, smooth shading, and all existing multiresolution detail are retained.
UVs are retained when linear subdivision of the recovered corner UVs reproduces
every input corner within 1e-6 UV units. Other UV detail cannot be represented
by the existing format and is refused. Meshes with any vertex groups, shape
keys, sculpt mask, face sets, hidden-face data, seams or UV pins are refused,
including zero-valued arrays. Those attributes must be removed explicitly
before recovery; they are never silently discarded.

An initial unsupported mesh leaves the scene, editor session, history and file
bytes unchanged. Rebuild stops normally when its recovered base admits no
further complete level. The whole reconstructed hierarchy lives in the mesh
blob's existing multires offsets, so schema version 6 stays unchanged. Undo,
redo, save/reopen and collaboration use the existing atomic mesh register.

## Clean-room evidence

Only the public Blender application and its
[Multiresolution manual](https://docs.blender.org/manual/en/3.0/modeling/modifiers/generate/multiresolution.html)
were consulted. No Blender implementation source was accessed.
`crates/engine/model/tests/blender/multires8.py` records Blender 5.2.3 LTS
through public Python operators. `tests/fixtures/multires8-reference.json`
in that crate contains the input, recovered base and every output level of
plain/deformed cubes, an open plane, a mesh with an existing level, and
unsupported cube/triangle cases. Offline tests compare counts and oriented
topology, retained base positions within 2e-6 m, and highest-level positions
within 2e-6 m for the initially level-free reference cases. Existing Kition
sculpted levels are compared before/after within 3e-6 m. Intermediate newly
recovered surfaces are not claimed to match Blender's displacement interpolation.

Run the reference script again by setting `KITION_BLENDER` to a local Blender
executable and running `cargo test -p kition-scene-model --test multires8`.
The same tests always use the committed observations offline. Additional
tests cover reordered indices/faces, rotated geometry, open grids, extraordinary
vertices, disconnected components of different depths, and atomic rejection.
The KDM test repeats both fixed actor orders and both merge orders twenty times.
`web/tests/multires8.mjs` drives the actual panel, person undo/redo, and host
save/reopen in Chromium or WebKit.
