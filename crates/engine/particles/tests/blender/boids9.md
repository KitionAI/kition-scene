# Airborne boid observations

This implementation was derived only from Blender 5.2.3 LTS running public Python APIs and the [Blender boids manual](https://docs.blender.org/manual/en/latest/physics/particles/emitter/physics/boids.html). No Blender implementation source was read.

`boids9.py` creates mesh emitters, configures boid properties and rules through RNA and `bpy.ops.boid`, advances frames, and records public particle positions and velocities. `../fixtures/boids9-reference.json` contains the unmodified observations. Ordinary Rust tests read it without Blender. `KITION_BLENDER=<executable> cargo test -p kition-scene-particles --test boids9` repeats the recording live.

The 24 observations vary target distance and direction, maximum speed, acceleration, turning angle, timestep, personal space, missing targets, empty rules, separation, flocking and averaged combinations. All states use airborne Average evaluation, minimum air speed zero, particle size 0.05 m, no launch velocity and no gravity. Blender defaults to Fuzzy evaluation; Kition Scene deliberately exposes only Average.

## Measured steering

For a Goal, the requested speed is maximum speed multiplied by the cube of `min(1, distance / (2 * speed))`; a stationary particle requests maximum speed. Avoid requests maximum speed away from the target. Acceleration caps the requested speed at current speed plus the configured acceleration fraction of maximum speed. The target direction is limited by the configured fraction of 180 degrees. The next velocity approaches that target by the timestep, and position uses the trapezoid of the previous and next velocities.

Separation responds inside the sum of the two particle sizes multiplied by personal space. Flocking combines the displacement to neighbors with their velocity difference. Average combines applicable response vectors and requested speeds. The implementation uses simultaneous old-state reads for coupled rules.

The 21 deterministic isolated observations are compared within 0.0005 m and 0.001 m/s through every recorded sample. The same numerical comparisons remain for every recorded mixed-case sample before the demonstrated seed-dependent event: frame 3 for Separate + Flock, frame 35 for Separate + Goal, and frame 56 for Flock + Goal. At and after those events, the tests retain sample counts and full error reporting while checking deterministic replay, finite positions and velocities, bounded speed and displacement, nonstationary motion, Goal approach and pair separation or cohesion. The Rust test starts coupled cases from the recorded frame-2 state, deliberately separating steering from emission and birth-heading randomness. Full scene tests separately cover the kit's own emitter, rendering, seeking, editing, save/reopen and collaborative state.

## Hidden turn state

`boids9_state.py` and `boids9_mixed_state.py` expose a limit of reconstructing trajectories from public particle state. Each runs seed0, then runs seed1 and overwrites all eight writable public motion fields immediately before a turn: location, velocity, rotation, angular velocity, and each previous value. The overwritten snapshots are exactly equal, but their next velocities differ. A separate velocity-override control changes the next trajectory, confirming the writes take effect.

| Case | Equal public state at frame | Next frame | Seed0 first velocity | Seed1 first velocity |
| --- | --- | --- | --- | --- |
| Separate + Flock | 2 | 3 | (-0.192000, 0.203426, 0.043382) | (-0.192000, -0.187244, 0.090575) |
| Separate + Goal | 34 | 35 | (-0.164388, -0.077940, 5.250683) | (-0.016695, 0.233801, 5.091264) |
| Flock + Goal | 55 | 56 | (0.173305, 0.001266, 2.879959) | (0.203939, 0.007855, 2.899850) |

The committed public-state fixtures preserve these observations. This is evidence of additional seed-dependent state, not proof of its internal algorithm. Kition Scene uses deterministic steering without reproducing that hidden turn state. Whole flock paths, especially after an opposing or near-arrival turn, are not claimed to match Blender sample by sample.
