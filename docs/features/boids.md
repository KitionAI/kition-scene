# Airborne boid particles

The Particles panel's Physics control selects Newtonian or Boids for an emitter. `particle.add` and `particle.set` expose the same `physics` and `boids` settings. Boids use Average evaluation across up to eight Goal, Avoid, Separate and Flock rules. Goal and Avoid target an object's current world-space origin, including parenting; an absent target contributes no response. Deleting a target clears its reference, including after a collaborative merge.

Maximum air speed defaults to 10 m/s, acceleration fraction to 0.5, turning-angle fraction to 0.5, personal space to 1, and the rule list to Separate and Flock. Rules are averaged, so list order does not express priority. Kition Scene does not implement Blender's default Fuzzy evaluation.

Boids emit from faces or vertices and start at rest. Newtonian launch, gravity, drag and bounce settings are retained when switching modes and hidden while boids are active. Scene force fields and collision objects do not affect airborne boids. Explicit collider lists, die-on-collision, hair systems and volume emission are rejected atomically in boid mode. Halo and object rendering, size variation and seeded instance rotation remain available; instance rotation is independent of flight direction.

Every query replays from emission using simultaneous particle updates. Births and deaths split frame intervals at their actual fractional times, so a newborn moves only during its age and a dead neighbor stops contributing immediately. Seeking or rewinding does not depend on playback history. Targets and emitter geometry use the scene's current evaluated state throughout that replay; animated target/emitter history is not reconstructed.

The settings limits are 2048 particles, 1000 emission-plus-lifetime frames, 500000 particle-frames, frame-start/end magnitudes at most 100000, timestep at most 0.1 s, particle size at most 10 m, and maximum speed at most 100 m/s. A separate 500000-update budget counts active particles across unique birth/death events and frame ticks, reserving one extra update per particle for a fractional query. The default 1000-particle emitter fits at 490496 updates. A rejected edit reports which budget must be reduced.

Neighbor search visits at most 128 tree nodes per particle and keeps at most 32 neighbors. Dense flocks can therefore differ from an unrestricted Blender neighborhood. Goal/Avoid-only systems do not build or query the neighbor tree. Surface preparation is linear in emitter mesh size, followed by logarithmic area sampling for faces; the replay budget does not limit the cost of evaluating the scene's modifiers.

The implementation uses wide intermediates for large finite coordinates, without a scene-wide coordinate validity condition that concurrent transforms could violate. Derived nonfinite world targets contribute no response and nonfinite emitter start positions emit no particle. Per-system settings remain one existing KDM JSON register; this extension retains schema version 6.

Ground behavior, obstacle avoidance, cross-system flocking, Fuzzy/Random evaluation, extra rule kinds and Blender's hidden seed-dependent turn state remain unsupported. See [the public-app observations](../../crates/engine/particles/tests/blender/boids9.md) for measured steering and the precise trajectory limits.
