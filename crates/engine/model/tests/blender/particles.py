# Particle statistics from the reference application, for tests/particles_blender.rs: an
# emitter with default settings on the default plane (live particles and their mean height at
# frames 10 and 50) and a hair system with default settings on the default UV sphere (strand
# count and mean length). Run: blender -b --factory-startup --python particles.py -- out.json
import json
import sys

import bpy

out = sys.argv[sys.argv.index("--") + 1]
for obj in list(bpy.data.objects):
    bpy.data.objects.remove(obj)
scene = bpy.context.scene
scene.frame_start, scene.frame_end = 1, 250

bpy.ops.mesh.primitive_plane_add(size=2)
plane = bpy.context.active_object
plane.modifiers.new("Particles", "PARTICLE_SYSTEM")

bpy.ops.mesh.primitive_uv_sphere_add(radius=1, location=(10, 0, 0))
sphere = bpy.context.active_object
sphere.modifiers.new("Hair", "PARTICLE_SYSTEM")
sphere.particle_systems[0].settings.type = "HAIR"

result = {"emitter": {}}
for frame in range(1, 51):
    scene.frame_set(frame)
    if frame in (10, 50):
        graph = bpy.context.evaluated_depsgraph_get()
        system = plane.evaluated_get(graph).particle_systems[0]
        alive = [p for p in system.particles if p.alive_state == "ALIVE"]
        heights = [p.location.z for p in alive]
        result["emitter"][str(frame)] = {
            "alive": len(alive),
            "meanHeight": sum(heights) / max(len(heights), 1),
        }

graph = bpy.context.evaluated_depsgraph_get()
system = sphere.evaluated_get(graph).particle_systems[0]
lengths = []
for particle in system.particles:
    keys = [key.co_local for key in particle.hair_keys]
    lengths.append(sum((keys[i + 1] - keys[i]).length for i in range(len(keys) - 1)))
result["hair"] = {
    "strands": len(system.particles),
    "keys": len(system.particles[0].hair_keys) if len(system.particles) else 0,
    "meanLength": sum(lengths) / max(len(lengths), 1),
}
with open(out, "w") as handle:
    json.dump(result, handle)
