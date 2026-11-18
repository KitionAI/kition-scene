# The reference side of tests/anim_blender.rs: builds each case in Blender (run with
# `blender -b --factory-startup --python anim.py -- out.json`) and writes the evaluated results:
# world matrices (row-major), vertex positions, and driven values.
import json
import math
import sys

import bpy
from mathutils import Euler, Matrix, Vector

out = {}


def fresh():
    bpy.ops.wm.read_factory_settings(use_empty=True)


def update():
    bpy.context.view_layer.update()


def rows(matrix):
    return [list(row) for row in matrix]


def empty(name, loc, rot=(0, 0, 0), scale=(1, 1, 1), parent=None):
    o = bpy.data.objects.new(name, None)
    o.location = loc
    o.rotation_euler = rot
    o.scale = scale
    bpy.context.collection.objects.link(o)
    if parent is not None:
        o.parent = parent
    return o


def armature(name, bones, loc=(0, 0, 0), rot=(0, 0, 0)):
    """bones: (name, head, tail, roll, parent, connected)"""
    data = bpy.data.armatures.new(name)
    rig = bpy.data.objects.new(name, data)
    bpy.context.collection.objects.link(rig)
    rig.location = loc
    rig.rotation_euler = rot
    bpy.context.view_layer.objects.active = rig
    bpy.ops.object.mode_set(mode='EDIT')
    for bone_name, head, tail, roll, parent, connected in bones:
        b = data.edit_bones.new(bone_name)
        b.head = head
        b.tail = tail
        b.roll = roll
        if parent:
            b.parent = data.edit_bones[parent]
            b.use_connect = connected
    bpy.ops.object.mode_set(mode='OBJECT')
    for pb in rig.pose.bones:
        pb.rotation_mode = 'XYZ'
    return rig


def pose(rig, name, loc=(0, 0, 0), rot=(0, 0, 0), scale=(1, 1, 1)):
    pb = rig.pose.bones[name]
    pb.location = loc
    pb.rotation_euler = rot
    pb.scale = scale


def bone_world(rig, name):
    return rig.matrix_world @ rig.pose.bones[name].matrix


def linear(action):
    for layer in action.layers:
        for strip in layer.strips:
            for bag in strip.channelbags:
                for curve in bag.fcurves:
                    for key in curve.keyframe_points:
                        key.interpolation = 'LINEAR'


def object_action():
    """Keys location, rotation, scale from rest at frame 1 to a pose at frame 21."""
    d = empty('Dummy', (0, 0, 0))
    for data_path in ('location', 'rotation_euler', 'scale'):
        d.keyframe_insert(data_path, frame=1)
    d.location = (1, 0.5, -0.4)
    d.rotation_euler = (0.5, -0.3, 0.8)
    d.scale = (1.5, 0.8, 1.2)
    for data_path in ('location', 'rotation_euler', 'scale'):
        d.keyframe_insert(data_path, frame=21)
    action = d.animation_data.action
    d.animation_data.action = None
    bpy.data.objects.remove(d)
    linear(action)
    return action


# Stretch To.
for keep in ('SWING_Y', 'PLANE_X', 'PLANE_Z'):
    for volume in ('VOLUME_XZX', 'VOLUME_X', 'VOLUME_Z', 'NO_VOLUME'):
        for bulge in (1.0, 0.5):
            fresh()
            o = empty('O', (1, 2, 0.5), (0.1, 0.2, 0.3), (1, 1.5, 2))
            t = empty('T', (4, -1, 2))
            c = o.constraints.new('STRETCH_TO')
            c.target = t
            c.keep_axis = keep
            c.volume = volume
            c.rest_length = 2.0
            c.bulge = bulge
            update()
            out['stretch-%s-%s-%s' % (keep, volume, bulge)] = rows(o.matrix_world)

# Floor.
for axis in ('FLOOR_X', 'FLOOR_Y', 'FLOOR_Z', 'FLOOR_NEGATIVE_X', 'FLOOR_NEGATIVE_Y', 'FLOOR_NEGATIVE_Z'):
    for use_rotation in (False, True):
        for index, at in enumerate([(1, 2, 0.5), (5, -3, 4), (2.5, 0.2, 1.4)]):
            fresh()
            o = empty('O', at, (0.1, 0.2, 0.3))
            t = empty('T', (2, 0, 1.5), (0.3, -0.2, 0.5))
            c = o.constraints.new('FLOOR')
            c.target = t
            c.floor_location = axis
            c.use_rotation = use_rotation
            c.offset = 0.25
            update()
            out['floor-%s-%s-%d' % (axis, use_rotation, index)] = rows(o.matrix_world)

# Transformation: every channel pair and mix, clamped and extrapolated.
SUFFIX = {'LOCATION': '', 'ROTATION': '_rot', 'SCALE': '_scale'}
for source in ('LOCATION', 'ROTATION', 'SCALE'):
    for dest in ('LOCATION', 'ROTATION', 'SCALE'):
        mixes = {'LOCATION': ['ADD', 'REPLACE'], 'ROTATION': ['ADD', 'REPLACE', 'BEFORE', 'AFTER'], 'SCALE': ['REPLACE', 'MULTIPLY']}[dest]
        for mix in mixes:
            for extrapolate in (False, True):
                fresh()
                o = empty('O', (1, 2, 0.5), (0.1, 0.2, 0.3), (1, 1.2, 0.9))
                t = empty('T', (3, 0.3, 2.2), (0.4, -0.6, 1.1), (0.7, 2.5, 1.3))
                c = o.constraints.new('TRANSFORM')
                c.target = t
                c.map_from = source
                c.map_to = dest
                c.use_motion_extrapolate = extrapolate
                for i, a in enumerate('xyz'):
                    setattr(c, 'from_min_%s%s' % (a, SUFFIX[source]), [0.0, -1.0, 0.5][i] if source != 'SCALE' else [0.5, 1.0, 0.8][i])
                    setattr(c, 'from_max_%s%s' % (a, SUFFIX[source]), [2.0, 1.0, 1.5][i] if source != 'SCALE' else [1.5, 3.0, 2.0][i])
                    setattr(c, 'to_min_%s%s' % (a, SUFFIX[dest]), [0.0, 1.0, -1.0][i] if dest != 'SCALE' else [1.0, 0.5, 1.0][i])
                    setattr(c, 'to_max_%s%s' % (a, SUFFIX[dest]), [1.0, 3.0, 0.5][i] if dest != 'SCALE' else [2.0, 1.5, 3.0][i])
                c.map_to_x_from = 'Y'
                c.map_to_y_from = 'Z'
                c.map_to_z_from = 'X'
                if dest == 'LOCATION':
                    c.mix_mode = mix
                elif dest == 'ROTATION':
                    c.mix_mode_rot = mix
                else:
                    c.mix_mode_scale = mix
                update()
                out['transform-%s-%s-%s-%s' % (source, dest, mix, extrapolate)] = rows(o.matrix_world)


# Spaces on objects: parented and not, owner and target.
def spaced(label, kind, owner_space, target_space, parented, setup=None):
    fresh()
    p = empty('P', (0.3, -0.2, 0.4), (0.2, 0.1, -0.3), (1.1, 0.9, 1.3))
    o = empty('O', (1, 2, 0.5), (0.1, 0.2, 0.3), (1, 1.2, 0.9), parent=p if parented else None)
    q = empty('Q', (-1, 0.5, 0.2), (0.0, 0.3, 0.6))
    t = empty('T', (3, 0.3, 2.2), (0.4, -0.6, 1.1), (0.7, 2.5, 1.3), parent=q if parented else None)
    c = o.constraints.new(kind)
    if hasattr(c, 'target'):
        c.target = t
    c.owner_space = owner_space
    if hasattr(c, 'target_space'):
        c.target_space = target_space
    if setup:
        setup(c)
    update()
    out[label] = rows(o.matrix_world)


for parented in (False, True):
    for owner_space in ('WORLD', 'LOCAL'):
        for target_space in ('WORLD', 'LOCAL'):
            tag = '%s-%s-%s' % (owner_space, target_space, parented)
            spaced('copyLocation-' + tag, 'COPY_LOCATION', owner_space, target_space, parented)
            spaced('copyRotation-' + tag, 'COPY_ROTATION', owner_space, target_space, parented)
            spaced('transform-space-' + tag, 'TRANSFORM', owner_space, target_space, parented,
                   lambda c: (setattr(c, 'map_from', 'LOCATION'), setattr(c, 'map_to', 'ROTATION'), setattr(c, 'from_max_x', 4.0), setattr(c, 'to_max_z_rot', 1.0),
                              setattr(c, 'from_max_y', 2.0), setattr(c, 'to_max_x_rot', 0.5)))
    for space in ('WORLD', 'LOCAL'):
        spaced('limitRotation-%s-%s' % (space, parented), 'LIMIT_ROTATION', space, 'WORLD', parented,
               lambda c: (setattr(c, 'use_limit_x', True), setattr(c, 'min_x', -0.05), setattr(c, 'max_x', 0.05), setattr(c, 'use_limit_z', True),
                          setattr(c, 'min_z', 0.0), setattr(c, 'max_z', 0.1)))

# Spaces on bones.
RIG_BONES = [('P', (0, 0, 0), (0, 0.3, 1), 0.2, None, False), ('C', (0.1, 0.3, 1.2), (0.5, 0.5, 2), -0.4, 'P', False)]


def posed_rig():
    rig = armature('Rig', RIG_BONES, (0.5, 0.2, 0.1), (0.2, 0, 0.4))
    pose(rig, 'P', (0.1, 0.2, 0.0), (0.3, 0.1, -0.2), (1, 1.1, 1))
    pose(rig, 'C', (0.0, 0.1, 0.2), (-0.2, 0.4, 0.1), (0.9, 1, 1.2))
    return rig


for kind in ('COPY_LOCATION', 'COPY_ROTATION'):
    for owner_space in ('WORLD', 'POSE', 'LOCAL_WITH_PARENT', 'LOCAL'):
        for target_space in ('WORLD', 'POSE', 'LOCAL_WITH_PARENT', 'LOCAL'):
            fresh()
            rig = posed_rig()
            other = armature('Other', [('B', (0, 0, 0), (0, 0, 1), 0.3, None, False), ('D', (0, 0, 1), (0.2, 0.4, 1.8), 0.0, 'B', True)], (-1, 1, 0.3), (0, 0.3, 0.2))
            pose(other, 'B', (0.2, 0, 0.1), (0.1, 0.2, 0.3))
            pose(other, 'D', (0, 0, 0), (0.4, -0.1, 0.2), (1.2, 1, 1))
            c = rig.pose.bones['C'].constraints.new(kind)
            c.target = other
            c.subtarget = 'D'
            c.owner_space = owner_space
            c.target_space = target_space
            update()
            out['bone-%s-%s-%s' % (kind, owner_space, target_space)] = rows(bone_world(rig, 'C'))

# Action on objects.
for mix in ('REPLACE', 'BEFORE_FULL', 'BEFORE', 'BEFORE_SPLIT', 'AFTER_FULL', 'AFTER', 'AFTER_SPLIT'):
    for parented in (False, True):
        for uniform in (False, True):
            fresh()
            action = object_action()
            p = empty('P', (0.3, -0.2, 0.4), (0.2, 0.1, -0.3), (1.2, 1.2, 1.2) if uniform else (1.1, 0.9, 1.3))
            o = empty('O', (1, 2, 0.5), (0.1, 0.2, 0.3), (1.1, 1.1, 1.1) if uniform else (1, 1.2, 0.9), parent=p if parented else None)
            t = empty('T', (0, 0, 0), (0, 0, math.radians(30)))
            c = o.constraints.new('ACTION')
            c.target = t
            c.action = action
            c.action_slot = action.slots[0]
            c.transform_channel = 'ROTATION_Z'
            c.min = 0
            c.max = 60
            c.frame_start = 1
            c.frame_end = 21
            c.mix_mode = mix
            update()
            out['action-%s-%s-%s' % (mix, parented, uniform)] = rows(o.matrix_world)

# Action on a bone, driven by a target's location in its local space.
for mix in ('AFTER', 'BEFORE_FULL', 'REPLACE'):
    fresh()
    rig = armature('Rig', [('Root', (0, 0, 0), (0, 0, 1), 0.0, None, False), ('B', (0, 0, 1), (0, 0.3, 2), 0.2, 'Root', True)], (0.2, 0, 0), (0, 0, 0.3))
    pb = rig.pose.bones['B']
    pb.keyframe_insert('location', frame=1)
    pb.keyframe_insert('rotation_euler', frame=1)
    pb.location = (0.2, 0.1, 0)
    pb.rotation_euler = (0.6, 0, -0.4)
    pb.keyframe_insert('location', frame=11)
    pb.keyframe_insert('rotation_euler', frame=11)
    action = rig.animation_data.action
    rig.animation_data.action = None
    linear(action)
    pose(rig, 'B', (0, 0, 0), (0, 0.2, 0))
    pose(rig, 'Root', (0, 0, 0), (0.1, 0, 0))
    q = empty('Q', (1, 0, 0), (0, 0, 0.5))
    t = empty('T', (0.6, 0.25, 0), parent=q)
    c = pb.constraints.new('ACTION')
    c.target = t
    c.action = action
    c.action_slot = action.slots[0]
    c.transform_channel = 'LOCATION_X'
    c.target_space = 'LOCAL'
    c.min = 0
    c.max = 1
    c.frame_start = 1
    c.frame_end = 11
    c.mix_mode = mix
    update()
    out['action-bone-%s' % mix] = rows(bone_world(rig, 'B'))

# Armature constraint and dual quaternions.
ARM_BONES = [('B1', (0, 0, 0), (0, 0, 1), 0.3, None, False), ('B2', (0, 0, 1), (0, 0.2, 2), 0.0, 'B1', True)]


def deform_rig(scaled):
    rig = armature('Rig', ARM_BONES, (0.5, 0.2, 0.1), (0.2, 0, 0.4))
    pose(rig, 'B1', (0, 0, 0), (0.5, 0, 0.2))
    pose(rig, 'B2', (0, 0, 0), (0, 0.3, -1.9), (1, 1.4, 0.8) if scaled else (1, 1, 1))
    return rig


for weights in ((1.0,), (0.5,), (0.7, 0.3), (0.4, 0.4)):
    for preserve in (False, True):
        for scaled in (False, True):
            fresh()
            rig = deform_rig(scaled)
            o = empty('O', (0.3, 0.4, 1.4), (0.1, 0.2, 0.3))
            c = o.constraints.new('ARMATURE')
            for i, w in enumerate(weights):
                t = c.targets.new()
                t.target = rig
                t.subtarget = ['B1', 'B2'][i]
                t.weight = w
            c.use_deform_preserve_volume = preserve
            update()
            out['armature-%s-%s-%s' % ('-'.join(str(w) for w in weights), preserve, scaled)] = rows(o.matrix_world)

# The Armature modifier blending dual quaternions on a cylinder.
for scaled in (False, True):
    fresh()
    rig = deform_rig(scaled)
    # Rings of a tube along both bones, z 0 to 2.
    rings = [(0.3 * math.cos(a * math.pi / 4), 0.3 * math.sin(a * math.pi / 4), z * 0.25) for z in range(9) for a in range(8)]
    data = bpy.data.meshes.new('Tube')
    data.from_pydata(rings, [], [])
    mesh = bpy.data.objects.new('Tube', data)
    bpy.context.collection.objects.link(mesh)
    mesh.location = (0.5, 0.2, 0.1)
    mesh.rotation_euler = (0.2, 0, 0.4)
    update()
    g1 = mesh.vertex_groups.new(name='B1')
    g2 = mesh.vertex_groups.new(name='B2')
    weights = []
    for v in mesh.data.vertices:
        w2 = min(max((v.co.z - 0.6) / 0.8, 0), 1)
        if w2 < 1:
            g1.add([v.index], 1 - w2, 'REPLACE')
        if w2 > 0:
            g2.add([v.index], w2, 'REPLACE')
        weights.append([1 - w2, w2])
    modifier = mesh.modifiers.new('Armature', 'ARMATURE')
    modifier.object = rig
    modifier.use_deform_preserve_volume = True
    update()
    evaluated = mesh.evaluated_get(bpy.context.evaluated_depsgraph_get()).to_mesh()
    out['dq-mesh-%s' % scaled] = {
        'positions': [list(v.co) for v in mesh.data.vertices],
        'weights': weights,
        'deformed': [list(v.co) for v in evaluated.vertices],
    }


# Spline IK.
def spline_case(label, points, kind='POLY', bones=3, chain=3, curve_loc=(0, 0, 0), curve_rot=(0, 0, 0), radii=None, rig_loc=(0, 0, 0), **settings):
    fresh()
    curve = bpy.data.curves.new('C', 'CURVE')
    curve.dimensions = '3D'
    spline = curve.splines.new(kind)
    if kind == 'POLY':
        spline.points.add(len(points) - 1)
        for point, co in zip(spline.points, points):
            point.co = (*co, 1)
    else:
        spline.bezier_points.add(len(points) - 1)
        for point, co in zip(spline.bezier_points, points):
            point.co = co
            point.handle_left_type = 'AUTO'
            point.handle_right_type = 'AUTO'
    if radii:
        for point, radius in zip(spline.points if kind == 'POLY' else spline.bezier_points, radii):
            point.radius = radius
    curve_object = bpy.data.objects.new('Curve', curve)
    curve_object.location = curve_loc
    curve_object.rotation_euler = curve_rot
    bpy.context.collection.objects.link(curve_object)
    chain_bones = [('B%d' % i, (0, 0, i), (0, 0, i + 1), 0.0, 'B%d' % (i - 1) if i else None, i > 0) for i in range(bones)]
    rig = armature('Rig', chain_bones, rig_loc)
    c = rig.pose.bones['B%d' % (bones - 1)].constraints.new('SPLINE_IK')
    c.target = curve_object
    c.chain_count = chain
    for key, value in settings.items():
        setattr(c, key, value)
    update()
    out['spline-' + label] = {
        'curve': {'kind': kind, 'points': [list(p) for p in points], 'radii': radii or [], 'loc': list(curve_loc), 'rot': list(curve_rot),
                  'handles': [[list(p.handle_left), list(p.handle_right)] for p in spline.bezier_points] if kind == 'BEZIER' else []},
        'bones': [rows(bone_world(rig, 'B%d' % i)) for i in range(bones)],
    }


BENT = [(0, 0, 0), (2, 0, 0), (2, 2, 1)]
spline_case('straight', [(1, 0, 0), (1, 0, 6)])
spline_case('bent', BENT)
spline_case('bent-none', BENT, y_scale_mode='NONE')
spline_case('bent-original', BENT, y_scale_mode='BONE_ORIGINAL')
spline_case('bent-chain2', BENT, chain=2)
spline_case('bent-moved', BENT, curve_loc=(0.5, -0.3, 0.2), curve_rot=(0.2, 0.1, 0.4))
spline_case('bent-offset', BENT, rig_loc=(0.3, 0.2, -0.1), use_chain_offset=True)
spline_case('bent-radius', BENT, radii=[1.0, 0.5, 2.0], xz_scale_mode='NONE')
spline_case('bent-inverse', BENT, xz_scale_mode='INVERSE_PRESERVE', use_curve_radius=False)
spline_case('bent-volume', BENT, xz_scale_mode='VOLUME_PRESERVE', use_curve_radius=False)
spline_case('bezier', [(0, 0, 0), (1.5, 0.5, 0.5), (2, 2, 1), (1, 3, 2)], kind='BEZIER', bones=4, chain=4)

# Drivers.
fresh()
a = empty('A', (0, 0, 0))
a.keyframe_insert('location', frame=1)
a.location = (2, 1, -1)
a.keyframe_insert('location', frame=21)
a.rotation_euler = (0.2, 0.0, 0.0)
a.keyframe_insert('rotation_euler', frame=1)
a.rotation_euler = (0.2, 0.5, 1.0)
a.keyframe_insert('rotation_euler', frame=21)
linear(a.animation_data.action)
b = empty('B', (1, -1, 0.5), (0.1, 0.2, 0.7), (1.3, 0.8, 1.0))
p = empty('P', (0.5, 0.5, 0), (0, 0, 0.6), (1.5, 1.5, 1.5))
c = empty('C', (1, 0, 0), parent=p)
d = empty('D', (0, 0, 0))


def add_driver(data_path, index, kind, expression, variables):
    curve = d.driver_add(data_path, index)
    driver = curve.driver
    driver.type = kind
    if kind == 'SCRIPTED':
        driver.expression = expression
    for name, var_type, setup in variables:
        var = driver.variables.new()
        var.name = name
        var.type = var_type
        setup(var)


def transform(obj, channel, space='WORLD_SPACE'):
    def setup(var):
        var.targets[0].id = obj
        var.targets[0].transform_type = channel
        var.targets[0].transform_space = space
    return setup


def single(obj, data_path):
    def setup(var):
        var.targets[0].id_type = 'OBJECT'
        var.targets[0].id = obj
        var.targets[0].data_path = data_path
    return setup


def pair(first, second):
    def setup(var):
        var.targets[0].id = first
        var.targets[1].id = second
    return setup


add_driver('location', 0, 'SCRIPTED', 'ax * 2 + sin(frame / 10)', [('ax', 'TRANSFORMS', transform(a, 'LOC_X'))])
add_driver('location', 1, 'AVERAGE', '', [('ay', 'TRANSFORMS', transform(a, 'LOC_Y')), ('bz', 'SINGLE_PROP', single(b, 'rotation_euler[2]'))])
add_driver('location', 2, 'MAX', '', [('gap', 'LOC_DIFF', pair(a, b)), ('one', 'SINGLE_PROP', single(b, 'scale[2]'))])
add_driver('rotation_euler', 2, 'SCRIPTED', 'turn', [('turn', 'ROTATION_DIFF', pair(a, b))])
add_driver('rotation_euler', 0, 'SUM', '', [('cx', 'TRANSFORMS', transform(c, 'LOC_X', 'LOCAL_SPACE')), ('cw', 'TRANSFORMS', transform(c, 'LOC_X'))])
add_driver('rotation_euler', 1, 'MIN', '', [('ry', 'TRANSFORMS', transform(a, 'ROT_Y', 'TRANSFORM_SPACE')), ('sy', 'TRANSFORMS', transform(b, 'SCALE_Y'))])
add_driver('scale', 0, 'SCRIPTED', '1 + ay if ay > 0.5 else 2 - ay', [('ay', 'TRANSFORMS', transform(a, 'LOC_Y'))])
add_driver('scale', 1, 'SCRIPTED', 'max(sa, 0.5) * clamp(frame / 20)', [('sa', 'TRANSFORMS', transform(b, 'SCALE_AVG'))])
for frame in (1, 7, 15, 21):
    bpy.context.scene.frame_set(frame)
    out['drivers-%d' % frame] = list(d.location) + list(d.rotation_euler) + list(d.scale)

json.dump(out, open(sys.argv[sys.argv.index('--') + 1], 'w'))
