"""Record seed-dependent mixed-rule turns after matching public motion state.

Usage: Blender --background --factory-startup --python boids9_mixed_state.py -- OUTPUT.json
The output is checked in; ordinary tests do not require Blender.
"""
import bpy
import json
import sys
from pathlib import Path


def rule(kind, target=None):
    value = {"kind": kind}
    if target is not None:
        value["point"] = target
    return value


CASES = [
    {"name": "goal_up", "rules": [rule("goal", [0, 0, 10])]},
    {"name": "goal_far", "rules": [rule("goal", [0, 0, 100])]},
    {"name": "goal_close", "rules": [rule("goal", [0, 0, 1])], "last": 31},
    {"name": "goal_oblique", "rules": [rule("goal", [5, 2, 8])], "timestep": 0.08, "last": 31},
    {"name": "goal_side", "rules": [rule("goal", [10, 0, 0])]},
    {"name": "goal_negative", "rules": [rule("goal", [-6, 2, 5])]},
    {"name": "goal_speed_four", "rules": [rule("goal", [0, 0, 3])], "speed": 4},
    {"name": "goal_slow_acceleration", "rules": [rule("goal", [0, 0, 10])], "acceleration": 0.1},
    {"name": "goal_small_timestep", "rules": [rule("goal", [3, -4, 5])], "timestep": 0.02},
    {"name": "goal_low_turn", "rules": [rule("goal", [10, 0, 0])], "angular": 0.1},
    {"name": "goal_zero_turn", "rules": [rule("goal", [10, 0, 0])], "angular": 0},
    {"name": "goal_zero_speed", "rules": [rule("goal", [10, 0, 0])], "speed": 0},
    {"name": "avoid_side", "rules": [rule("avoid", [1, 0, 0])]},
    {"name": "avoid_oblique", "rules": [rule("avoid", [1, -2, 0])]},
    {"name": "no_rules", "rules": []},
    {"name": "missing_goal", "rules": [rule("goal")]},
    {"name": "separate_close", "rules": [rule("separate")], "vertices": [[-.02, 0, 0], [.02, 0, 0]]},
    {"name": "separate_far", "rules": [rule("separate")], "vertices": [[-1, 0, 0], [1, 0, 0]]},
    {"name": "separate_large_space", "rules": [rule("separate")], "space": 3, "vertices": [[-.08, 0, 0], [.08, 0, 0]]},
    {"name": "flock_pair", "rules": [rule("flock")], "vertices": [[-1, 0, 0], [1, 0, 0]], "initialize": 2},
    {"name": "averaged_goals", "rules": [rule("goal", [10, 0, 5]), rule("goal", [-10, 0, 5])]},
    {"name": "separate_goal", "rules": [rule("separate"), rule("goal", [0, 0, 10])], "vertices": [[-.02, 0, 0], [.02, 0, 0]], "initialize": 2},
    {"name": "flock_goal", "rules": [rule("flock"), rule("goal", [0, 0, 10])], "vertices": [[-1, 0, 0], [1, 0, 0]], "initialize": 2},
    {"name": "default_brain", "rules": [rule("separate"), rule("flock")], "vertices": [[-.02, 0, 0], [.02, 0, 0]], "initialize": 2},
]


def record(case, seed, before, replace=None):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    mesh = bpy.data.meshes.new("Emitter")
    mesh.from_pydata(case.get("vertices", [[0, 0, 0]]), [], [])
    emitter = bpy.data.objects.new("Emitter", mesh)
    bpy.context.collection.objects.link(emitter)
    bpy.context.view_layer.objects.active = emitter
    emitter.select_set(True)
    bpy.ops.object.particle_system_add()
    emitter.particle_systems[-1].seed = seed
    settings = emitter.particle_systems[-1].settings
    settings.physics_type = "BOIDS"
    settings.count = len(mesh.vertices)
    settings.frame_start = 1
    settings.frame_end = 1
    settings.lifetime = 300
    settings.emit_from = "VERT"
    settings.use_emit_random = False
    settings.normal_factor = 0
    settings.object_align_factor = (0, 0, 0)
    settings.effector_weights.gravity = 0
    settings.timestep = case.get("timestep", 0.04)
    settings.particle_size = 0.05
    boids = settings.boids
    boids.air_speed_max = case.get("speed", 10)
    boids.air_acc_max = case.get("acceleration", 0.5)
    boids.air_ave_max = case.get("angular", 0.5)
    boids.air_personal_space = case.get("space", 1)
    state = boids.states[0]
    state.ruleset_type = "AVERAGE"
    with bpy.context.temp_override(particle_settings=settings):
        while state.rules:
            state.active_boid_rule_index = 0
            bpy.ops.boid.rule_del()
        for data in case["rules"]:
            bpy.ops.boid.rule_add(type=data["kind"].upper())
            if "point" in data:
                target = bpy.data.objects.new("Target", None)
                bpy.context.collection.objects.link(target)
                target.location = data["point"]
                state.rules[-1].object = target
    samples = []
    for frame in range(1, case.get("last", 61) + 1):
        bpy.context.scene.frame_set(frame)
        system = emitter.evaluated_get(bpy.context.evaluated_depsgraph_get()).particle_systems[0]
        if frame == before and replace:
            for p, values in zip(system.particles, replace):
                for field, value in values.items():
                    setattr(p, field, value)
        if frame in [before, before + 1]:
            samples.append({"frame": frame, "particles": [{key:list(getattr(p,key)) for key in ["location","velocity","rotation","angular_velocity","prev_location","prev_velocity","prev_rotation","prev_angular_velocity"]} for p in system.particles]})
    return {**case, "samples": samples}



observations = []
for case, before in [(CASES[-3], 34), (CASES[-2], 55)]:
    case = {**case, "last": before + 1}
    first = record(case, 0, before)
    second = record(case, 1, before, first["samples"][0]["particles"])
    assert first["samples"][0] == second["samples"][0]
    assert first["samples"][1] != second["samples"][1]
    observations.append({"case": case["name"], "seed0": first, "seed1MatchingPublicState": second})
Path(sys.argv[sys.argv.index("--") + 1]).write_text(json.dumps({"blender": bpy.app.version_string, "observations": observations}, indent=2) + "\n")
