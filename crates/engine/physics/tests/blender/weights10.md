# Field-weight observations

`weights10.py` uses only the public `bpy` API in the Blender application, version 5.2.3 LTS.
No implementation source was read. The committed `../fixtures/weights10-reference.json`
contains public RNA defaults/ranges, 120 parameter observations and 28 coefficient controls.
It records a four-vertex quad at frames 1, 2, 4 and 12 with cloth or a soft-body modifier.
Air damping/friction, goals and soft-body edges are disabled to isolate field influence.
The field is offset from the quad so Vortex and Force exercise nontrivial directions.

The observations establish seven defaults of 1 and hard ranges -200 to 200. All does not
scale Gravity. Negative Gravity reverses acceleration. All and a field's own weight are
multiplicative; a nonpositive product disables the field and a double negative enables it.
Independent product settings (0.3 times 0.7 and 0.21 alone) produce the same motion. Controls
separate strength from flow: a half-weight field agrees with both coefficients halved,
while halving strength alone need not agree. Drag's corresponding control halves both
drag coefficients. Vortex's nonlinear term requires weighting its strength before evaluation.

The fixture preserves absolute positions to make the observations auditable. For example,
at frame 12, default cloth gravity has centroid Z -0.60429549 m and soft-body gravity
-0.59890050 m; default Wind produces Z +0.08555511 m for cloth and -0.05087499 m for the
soft body. Default Vortex produces XY (0.47364877, -0.22849325) m for cloth and
(-0.32095131, 0.16396826) m for the soft body. These are observations, not new absolute
trajectory expectations for Kition Scene's existing solvers.

`weights10.rs` checks the same identities separately on the recorded application results
and on Kition Scene's own simulated results. Its 0.00002 m centroid tolerance accounts for the
recorded single-precision accumulation and does not mask the known baseline solver gaps.
Every simulated frame also has four finite vertices. The test includes nonzero motion
checks, so disabled fields cannot satisfy the transfer cases accidentally. Unit tests
add exact coefficient and default-output equality, full-range validation, finite boundaries
and explicit overflow handling. Existing physics expectations and tolerances are unchanged.

To repeat the observations without altering the committed reference:

```sh
KITION_BLENDER=<blender-executable> cargo +1.95.0 test -p kition-scene-physics --test weights10
```

The optional live test writes a temporary recording and compares it with the committed
JSON. The offline reference and simulation tests always run without Blender.
