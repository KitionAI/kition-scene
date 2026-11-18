"""Show seed-dependent turn planes after matching all public particle motion state.

Blender --background --python boids9_state.py -- OUTPUT.json
No implementation source is inspected. A velocity-override control proves the public
writes are used by the next integration step.
"""
import bpy
import json
import sys
from pathlib import Path

FIELDS = ["location", "velocity", "rotation", "angular_velocity", "prev_location", "prev_velocity", "prev_rotation", "prev_angular_velocity"]


def record(seed, replace=None):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    mesh = bpy.data.meshes.new("Emitter")
    mesh.from_pydata([[-0.02, 0, 0], [0.02, 0, 0]], [], [])
    obj = bpy.data.objects.new("Emitter", mesh)
    bpy.context.collection.objects.link(obj)
    bpy.context.view_layer.objects.active = obj
    obj.select_set(True)
    bpy.ops.object.particle_system_add()
    system = obj.particle_systems[-1]
    system.seed = seed
    settings = system.settings
    settings.physics_type = "BOIDS"
    settings.count = 2
    settings.frame_start = settings.frame_end = 1
    settings.lifetime = 300
    settings.emit_from = "VERT"
    settings.use_emit_random = False
    settings.normal_factor = 0
    settings.object_align_factor = (0, 0, 0)
    settings.effector_weights.gravity = 0
    settings.boids.states[0].ruleset_type = "AVERAGE"
    snapshots = []
    for frame in range(1, 4):
        bpy.context.scene.frame_set(frame)
        evaluated = obj.evaluated_get(bpy.context.evaluated_depsgraph_get()).particle_systems[0]
        if frame == 2 and replace:
            for particle, values in zip(evaluated.particles, replace):
                for field, value in values.items():
                    setattr(particle, field, value)
        snapshots.append({"frame": frame, "particles": [{field: list(getattr(particle, field)) for field in FIELDS} for particle in evaluated.particles]})
    return snapshots


first = record(0)
state = first[1]["particles"]
second = record(1, state)
control = [{**state[0], "velocity": [0, 0, 100]}, state[1]]
assert first[1] == second[1]
assert first[2] != second[2]
result = {"blender": bpy.app.version_string, "seed0": first, "seed1MatchingPublicState": second, "velocityOverrideControl": record(0, control)}
Path(sys.argv[sys.argv.index("--") + 1]).write_text(json.dumps(result, indent=2) + "\n")
