"""Hair dynamics in the reference application, for tests/hair.rs. Run: blender -b --factory-startup
--python hair.py -- out.json; writes every number the test compares (and
tests/fixtures/hair-reference.json holds what Blender 5.2.3 wrote).

Each case grows four strands of four segments from a 10 cm plane and reads the simulated
strands (`co_hair` with as many display steps as segments, so the points are the keys)."""

import bpy, json, sys, math


def strands(level=True, length=1.0, end=200, gravity=True, floor=None, field=None, move=False, **settings):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.frame_start, scene.frame_end, scene.render.fps = 1, end, 24
    if not gravity:
        scene.gravity = (0, 0, 0)
    if floor is not None:
        bpy.ops.mesh.primitive_plane_add(size=10, location=(0, 0, floor))
        bpy.ops.object.modifier_add(type='COLLISION')
    if field == 'WIND':
        bpy.ops.object.effector_add(type='WIND', location=(-3, 0, 0), rotation=(0, math.pi / 2, 0))
    elif field == 'FORCE':
        bpy.ops.object.effector_add(type='FORCE', location=(3, 0, 0))
    # Level: the plane turned to face -Y, so its hair grows level.
    bpy.ops.mesh.primitive_plane_add(size=0.1, rotation=(math.radians(90) if level else 0, 0, 0))
    emitter = bpy.context.object
    if move:
        emitter.location = (0, 0, 0)
        emitter.keyframe_insert('location', frame=1)
        emitter.location = (1, 0, 0)
        emitter.keyframe_insert('location', frame=25)
        for curve in emitter.animation_data.action.fcurves if hasattr(emitter.animation_data.action, 'fcurves') else []:
            for key in curve.keyframe_points:
                key.interpolation = 'LINEAR'
        for layer in getattr(emitter.animation_data.action, 'layers', []):
            for strip in layer.strips:
                for bag in strip.channelbags:
                    for curve in bag.fcurves:
                        for key in curve.keyframe_points:
                            key.interpolation = 'LINEAR'
    bpy.ops.object.particle_system_add()
    system = emitter.particle_systems[0]
    particles = system.settings
    particles.type = 'HAIR'
    particles.count = 4
    particles.hair_length = length
    particles.hair_step = 4
    particles.display_step = 2
    system.use_hair_dynamics = True
    system.point_cache.frame_end = end
    for key, value in settings.items():
        setattr(system.cloth.settings, key, value)
    tips, roots, lowest, lengths = [], [], [], []
    for frame in range(1, end + 1):
        scene.frame_set(frame)
        evaluated = emitter.evaluated_get(bpy.context.evaluated_depsgraph_get())
        hair = evaluated.particle_systems[0]
        points = [[list(hair.co_hair(evaluated, particle_no=i, step=j)) for j in range(5)] for i in range(len(hair.particles))]
        tips.append([round(sum(s[-1][a] - s[0][a] for s in points) / len(points), 5) for a in range(3)])
        roots.append([round(sum(s[0][a] for s in points) / len(points), 5) for a in range(3)])
        lowest.append(round(min(p[2] for s in points for p in s), 5))
        lengths.append(round(sum(sum(math.dist(s[k], s[k + 1]) for k in range(4)) for s in points) / len(points), 5))
    return {'tip': tips, 'root': roots, 'lowest': lowest, 'length': lengths}


result = {
    'level': strands(),
    'levelSoft': strands(bending_stiffness=0.05),
    'levelStiff': strands(bending_stiffness=5.0),
    'long': strands(length=2.0),
    'heavy': strands(mass=1.0),
    'damped': strands(bending_damping=5.0),
    'air': strands(air_damping=10.0),
    'pinned': strands(pin_stiffness=1.0),
    'upright': strands(level=False, end=48),
    'force': strands(level=False, end=48, gravity=False, field='FORCE'),
    'forceHeavy': strands(level=False, end=48, gravity=False, field='FORCE', mass=1.0),
    'wind': strands(level=False, end=48, gravity=False, field='WIND'),
    'floor': strands(end=96, floor=-0.3),
    'moving': strands(level=False, end=48, move=True),
}
with open(sys.argv[sys.argv.index('--') + 1], 'w') as handle:
    json.dump(result, handle, separators=(',', ':'))
