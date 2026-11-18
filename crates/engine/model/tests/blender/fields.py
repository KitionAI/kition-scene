# Force fields on particles, and particle density groups, in the reference application, for
# tests/fields_blender.rs. One particle released at rest (no gravity) on a 1 mm plane next to
# each kind of field, its position at frames 10 and 30; 400 particles released into turbulence,
# their mean speed; and 1000 particles on a grid weighted by a density group, their mean x.
# Run: blender -b --factory-startup --python fields.py -- out.json
import json
import math
import sys

import bpy

out = sys.argv[sys.argv.index("--") + 1]


def reset():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.frame_start, scene.frame_end = 1, 250


def emitter(normal=0.0, count=1, size=0.001):
    bpy.ops.mesh.primitive_plane_add(size=size)
    plane = bpy.context.active_object
    plane.modifiers.new("P", "PARTICLE_SYSTEM")
    settings = plane.particle_systems[0].settings
    settings.count, settings.frame_start, settings.frame_end, settings.lifetime = count, 1, 1, 200
    settings.normal_factor = normal
    settings.effector_weights.gravity = 0
    return plane


def field(kind, location, rotation=(0, 0, 0), **settings):
    bpy.ops.object.effector_add(type=kind, location=location, rotation=rotation)
    for key, value in settings.items():
        setattr(bpy.context.active_object.field, key, value)


def alive(plane, frames):
    scene, found = bpy.context.scene, {}
    for frame in range(1, max(frames) + 1):
        scene.frame_set(frame)
        if frame in frames:
            system = plane.evaluated_get(bpy.context.evaluated_depsgraph_get()).particle_systems[0]
            # Copies: the particles read live, so later frames would overwrite them.
            found[frame] = [(list(p.location), p.velocity.length) for p in system.particles if p.alive_state == "ALIVE"]
    return found


def single(kind, location, rotation=(0, 0, 0), normal=0.0, **settings):
    reset()
    plane = emitter(normal)
    field(kind, location, rotation, **settings)
    return {str(frame): particles[0][0] for frame, particles in alive(plane, [10, 30]).items()}


def turbulence(strength, size):
    reset()
    plane = emitter(count=400, size=2.0)
    field("TURBULENCE", (0, 0, 0), strength=strength, size=size)
    return {str(frame): sum(speed for _, speed in particles) / len(particles) for frame, particles in alive(plane, [10, 30]).items()}


def density(weight, emit_from):
    reset()
    bpy.ops.mesh.primitive_grid_add(x_subdivisions=11, y_subdivisions=11, size=2)
    plane = bpy.context.active_object
    group = plane.vertex_groups.new(name="Density")
    for vertex in plane.data.vertices:
        group.add([vertex.index], weight(vertex.co.x), "REPLACE")
    plane.modifiers.new("P", "PARTICLE_SYSTEM")
    settings = plane.particle_systems[0].settings
    settings.count, settings.frame_start, settings.frame_end, settings.lifetime = 1000, 1, 1, 100
    settings.normal_factor, settings.emit_from = 0, emit_from
    settings.effector_weights.gravity = 0
    plane.particle_systems[0].vertex_group_density = "Density"
    xs = [location[0] for location, _ in alive(plane, [2])[2]]
    return {"alive": len(xs), "meanX": sum(xs) / len(xs), "negative": sum(1 for x in xs if x < 0)}


result = {
    "force": single("FORCE", (2, 0, 0)),
    "forceFalloff": single("FORCE", (2, 0, 0), falloff_power=2.0),
    "wind": single("WIND", (0, 0, -3)),
    "windStill": single("WIND", (-3, 0, 0), (0, math.pi / 2, 0), flow=0.0, strength=5.0),
    "vortex": single("VORTEX", (1, 0, 0)),
    "vortexStrong": single("VORTEX", (1, 0, 0), strength=2.0),
    "drag": single("DRAG", (0, 0, 0), normal=2.0),
    # The reference's `linear_drag` slows in proportion to the square of the speed.
    "dragSquare": single("DRAG", (0, 0, 0), normal=4.0, linear_drag=1.0, quadratic_drag=0.0),
    "turbulence": turbulence(1.0, 1.0),
    "turbulenceStrong": turbulence(5.0, 1.0),
    "turbulenceLarge": turbulence(5.0, 2.0),
    "densityRamp": density(lambda x: (x + 1) / 2, "FACE"),
    "densityStep": density(lambda x: 1.0 if x > 0 else 0.0, "FACE"),
    "densityVertices": density(lambda x: (x + 1) / 2, "VERT"),
}
with open(out, "w") as handle:
    json.dump(result, handle)
