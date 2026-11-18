# Roadmap

> **Last updated:** 2026-11-17 · **Source:** `features.yaml` (updated 2026-11-17) and the engine's command list

Open work on Kition Scene, in order. Each item names where its detail lives.

1. **Prove the P1 features end to end:** `scene.animation.timeline`, `scene.collab.presence`, `scene.compositor.nodes`, `scene.curve.text-lattice`, `scene.edit.history`, `scene.file.import-export`, `scene.material.manage`, `scene.mesh.edit`, `scene.mode.switch`, `scene.modifier.stack`, `scene.nodes.geometry`, `scene.object.add`, `scene.object.transform`, `scene.particles.hair`, `scene.physics.simulation`, `scene.render.lights-world`, `scene.rigging.armature`, `scene.sculpt.paint`, `scene.uv.editing`, `scene.view.viewport`: each needs a case that shows it working in the editor and surviving save and reopen before it is `has`. Detail in [gaps.md](gaps.md).
2. **Keep the catalogue complete:** every new engine command joins a feature in `features.yaml` with a case.
