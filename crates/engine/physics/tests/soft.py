"""Soft bodies in the reference application, for tests/soft.rs. Run: blender -b --factory-startup
--python soft.py -- out.json; writes every number the test compares (and
tests/fixtures/soft-reference.json holds what Blender 5.2.3 wrote)."""

import bpy, json, sys, math


def reset(end):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.frame_start, scene.frame_end, scene.render.fps = 1, end, 24
    return scene


def soft(obj, end, **settings):
    bpy.context.view_layer.objects.active = obj
    bpy.ops.object.modifier_add(type='SOFT_BODY')
    obj.modifiers['Softbody'].point_cache.frame_end = end
    for key, value in settings.items():
        setattr(obj.soft_body, key, value)


def frames(obj, end):
    """The evaluated vertices in world space at every frame."""
    scene, out = bpy.context.scene, []
    for frame in range(1, end + 1):
        scene.frame_set(frame)
        evaluated = obj.evaluated_get(bpy.context.evaluated_depsgraph_get())
        out.append([list(evaluated.matrix_world @ v.co) for v in evaluated.data.vertices])
    return out


def r(value):
    return round(value, 5)


def summary(track, full=()):
    """Per frame the centroid and the lowest height; every vertex at the frames in `full`."""
    out = {
        'centroid': [[r(sum(p[axis] for p in frame) / len(frame)) for axis in range(3)] for frame in track],
        'lowest': [r(min(p[2] for p in frame)) for frame in track],
    }
    if full:
        out['start'] = [[r(c) for c in p] for p in track[0]]
        out['frames'] = {str(f): [[r(c) for c in p] for p in track[f - 1]] for f in full}
    return out


def edge_mesh(name, verts, edges, pinned):
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(verts, edges, [])
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.scene.collection.objects.link(obj)
    group = obj.vertex_groups.new(name='Pin')
    group.add(pinned, 1.0, 'REPLACE')
    return obj


def jelly():
    reset(60)
    bpy.ops.mesh.primitive_cube_add(size=2)
    obj = bpy.context.object
    soft(obj, 60)
    return summary(frames(obj, 60))


def hang(bend):
    reset(120)
    bpy.ops.mesh.primitive_grid_add(x_subdivisions=10, y_subdivisions=10, size=2)
    obj = bpy.context.object
    group = obj.vertex_groups.new(name='Pin')
    group.add([v.index for v in obj.data.vertices if v.co.y < -0.99], 1.0, 'REPLACE')
    soft(obj, 120, vertex_group_goal='Pin', goal_default=1.0, bend=bend)
    return summary(frames(obj, 120), full=(11, 31, 61, 120))


def chain(shape, **settings):
    reset(240)
    if shape == 'pair':
        obj = edge_mesh('Pair', [(0, 0, 0), (0, 0, -1), (0, 0, -2)], [(0, 1), (1, 2)], [0])
    else:
        obj = edge_mesh('Bend', [(0, 0, 0), (1, 0, 0), (2, 0, 0)], [(0, 1), (1, 2)], [0, 1])
    soft(obj, 240, vertex_group_goal='Pin', friction=5.0, **settings)
    return summary(frames(obj, 240), full=(240,))


def drop():
    reset(72)
    bpy.ops.mesh.primitive_plane_add(size=10)
    bpy.ops.object.modifier_add(type='COLLISION')
    bpy.ops.mesh.primitive_cube_add(size=1, location=(0, 0, 2))
    obj = bpy.context.object
    soft(obj, 72, use_goal=False)
    return summary(frames(obj, 72))


def bounce(damping):
    reset(96)
    bpy.ops.mesh.primitive_plane_add(size=10)
    bpy.ops.object.modifier_add(type='COLLISION')
    bpy.context.object.collision.damping = damping
    bpy.ops.mesh.primitive_plane_add(size=0.2, location=(0, 0, 1))
    obj = bpy.context.object
    soft(obj, 96, use_goal=False, use_edges=False)
    return summary(frames(obj, 96))


def field(kind, mass):
    reset(48)
    bpy.context.scene.gravity = (0, 0, 0)
    bpy.ops.mesh.primitive_grid_add(x_subdivisions=4, y_subdivisions=4, size=1)
    obj = bpy.context.object
    soft(obj, 48, use_goal=False, mass=mass)
    if kind == 'FORCE':
        bpy.ops.object.effector_add(type='FORCE', location=(3, 0, 0))
    else:
        bpy.ops.object.effector_add(type='WIND', location=(-3, 0, 0), rotation=(0, math.pi / 2, 0))
    return summary(frames(obj, 48))


def frame_range():
    reset(48)
    bpy.ops.mesh.primitive_cube_add(size=2)
    obj = bpy.context.object
    soft(obj, 48)
    obj.modifiers['Softbody'].point_cache.frame_start = 10
    obj.modifiers['Softbody'].point_cache.frame_end = 30
    return summary(frames(obj, 48))


result = {
    'jelly': jelly(),
    'hang': hang(0.0),
    'hangBend': hang(1.0),
    'pair': chain('pair'),
    'bend': chain('bend', bend=2.0),
    'drop': drop(),
    'bounce': bounce(0.0),
    'bounceDefault': bounce(0.1),
    'bounceDead': bounce(0.5),
    'force': field('FORCE', 1.0),
    'forceHeavy': field('FORCE', 2.0),
    'wind': field('WIND', 1.0),
    'range': frame_range(),
}
with open(sys.argv[sys.argv.index('--') + 1], 'w') as handle:
    json.dump(result, handle, separators=(',', ':'))
