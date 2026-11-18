# Cloth and soft-body field weights

Each cloth and soft body has its own `effectorWeights` in its existing object physics
settings. The Physics panel and `physics.cloth.add/set` / `physics.softBody.add/set` expose
All, Gravity, Force, Wind, Vortex, Turbulence and Drag. Each defaults to 1 and accepts finite
values from -200 to 200, matching the public Blender 5.2.3 LTS controls. Commands merge a
partial weight object; invalid values and unknown keys fail atomically.

Gravity uses its signed weight independently of All. For each field kind, the influence is
`max(all * kindWeight, 0)`. Zero or a negative product disables that field; two negative
weights produce a positive influence. Force, Wind, Vortex and Turbulence scale their strength
and flow before field evaluation. Drag scales both linear and quadratic drag coefficients.
In particular, Vortex weights precede its nonlinear centripetal term, so multiplying the
final force would give a different result. All-one weights leave the previous field
coefficients and gravity unchanged, preserving existing finite simulation results.

Settings are undoable, saved and collaborative. The existing physics JSON register contains
them; no KDM schema field is added (schema 6). Concurrent edits to different objects retain
both; conflicting edits to one object's physics choose one whole valid register according
to the replica's existing conflict policy. Changes to weights, field settings or geometry
invalidate the derived bake. A bake is never saved and each peer computes its own.

Large valid weights can make a simulation diverge. Nonfinite coefficients, positions or
velocities cause an explicit `failed:` bake error, discard every partial cache frame, and
leave the scene intact. Inspect reports `physics.state: "failed"` and an `error` string.
Automatic baking stops after failure. Correct the settings and request Bake explicitly,
or choose Free Bake. This avoids automatic retries on every subsequent edit. A failed
simulation does not make a concurrently merged scene invalid.

This feature covers cloth and soft bodies only, with the five field kinds already supported
by Kition Scene. Hair dynamics, Newtonian particles, boids, rigid-body-world weights, fluids, field
collection filters and other Blender field kinds remain outside this interface. Weights
do not add collision or aerodynamics features to the existing solvers. In particular,
Kition Scene's existing cloth field evaluation ignores flow, uses its own surface-area wind law,
and the existing Vortex field ignores flow. Existing solver differences in absolute
trajectories remain. The new reference tests establish the weight-transfer identities
separately in Blender and Kition Scene; they do not claim identical simulated paths.

The [recording notes](../../crates/engine/physics/tests/blender/weights10.md) explain the
public-app observations and tolerances. SDK tests cover typed edits, independent systems,
cache invalidation, deterministic replay, failure recovery and saved files. Fixed actor
and import orders run twenty times, including a concurrent combination whose valid
settings produce an explicit failed bake on both peers.
