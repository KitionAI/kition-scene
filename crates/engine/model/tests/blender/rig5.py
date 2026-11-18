# The reference side of tests/rig5_blender.rs: builds each case in Blender (run with
# `blender -b --factory-startup --python rig5.py -- out.json`) and writes each case's definition
# with what Blender made of it: envelope and vertex group deformation, Multi Modifier stacks,
# dual quaternions on turned and scaled bones, the weight Gradient tool, X-mirror strokes, flipped
# names, and custom bone shapes as the viewport draws them.
import json
import math
import random
import sys

import bpy
import gpu
from bpy_extras import view3d_utils
from mathutils import Euler, Matrix, Quaternion, Vector

gpu.init()
out = {}


def fresh():
    bpy.ops.object.select_all(action='SELECT')
    bpy.ops.object.delete()
    for collection in (bpy.data.meshes, bpy.data.armatures):
        for block in list(collection):
            collection.remove(block)


def update():
    bpy.context.view_layer.update()


def vec(values):
    return [round(float(value), 7) for value in values]


# Deformation: rigs, a point cloud with vertex groups, and a modifier stack.
def make_rig(spec):
    data = bpy.data.armatures.new(spec['name'])
    rig = bpy.data.objects.new(spec['name'], data)
    bpy.context.collection.objects.link(rig)
    rig.location = spec.get('location', (0, 0, 0))
    rig.rotation_euler = spec.get('rotation', (0, 0, 0))
    bpy.context.view_layer.objects.active = rig
    bpy.ops.object.mode_set(mode='EDIT')
    for bone in spec['bones']:
        b = data.edit_bones.new(bone['name'])
        b.head = bone['head']
        b.tail = bone['tail']
        b.roll = bone.get('roll', 0.0)
        if bone.get('parent'):
            b.parent = data.edit_bones[bone['parent']]
            b.use_connect = bone.get('connected', False)
        b.use_deform = bone.get('deform', True)
        b.head_radius = bone.get('headRadius', 0.1)
        b.tail_radius = bone.get('tailRadius', 0.05)
        b.envelope_distance = bone.get('distance', 0.25)
        b.envelope_weight = bone.get('weight', 1.0)
        b.use_envelope_multiply = bone.get('multiply', False)
    bpy.ops.object.mode_set(mode='OBJECT')
    for name, pose in spec.get('pose', {}).items():
        pb = rig.pose.bones[name]
        pb.rotation_mode = 'XYZ'
        pb.location = pose.get('location', (0, 0, 0))
        pb.rotation_euler = pose.get('rotation', (0, 0, 0))
        pb.scale = pose.get('scale', (1, 1, 1))
    return rig


def deform_case(label, case):
    fresh()
    rigs = [make_rig(spec) for spec in case['rigs']]
    data = bpy.data.meshes.new('Points')
    data.from_pydata(case['points'], [], [])
    mesh = bpy.data.objects.new('Points', data)
    bpy.context.collection.objects.link(mesh)
    mesh.location = case.get('meshLocation', (0, 0, 0))
    mesh.rotation_euler = case.get('meshRotation', (0, 0, 0))
    for name, weights in case['groups'].items():
        group = mesh.vertex_groups.new(name=name)
        for index, weight in enumerate(weights):
            if weight > 0:
                group.add([index], weight, 'REPLACE')
    for spec in case['modifiers']:
        if spec['type'] == 'displace':
            modifier = mesh.modifiers.new('Displace', 'DISPLACE')
            modifier.direction = spec['direction'].upper()
            modifier.strength = spec['strength']
            modifier.mid_level = spec['midLevel']
            continue
        modifier = mesh.modifiers.new('Armature', 'ARMATURE')
        modifier.object = rigs[spec['rig']]
        modifier.use_deform_preserve_volume = spec.get('preserveVolume', False)
        modifier.use_vertex_groups = spec.get('vertexGroups', True)
        modifier.use_bone_envelopes = spec.get('envelopes', False)
        modifier.vertex_group = spec.get('vertexGroup', '')
        modifier.invert_vertex_group = spec.get('invertVertexGroup', False)
        modifier.use_multi_modifier = spec.get('multiModifier', False)
    update()
    evaluated = mesh.evaluated_get(bpy.context.evaluated_depsgraph_get())
    case['deformed'] = [vec(v.co) for v in evaluated.data.vertices]
    out.setdefault('deform', {})[label] = case


def points(seed, count, low, high):
    rng = random.Random(seed)
    return [vec(rng.uniform(a, b) for a, b in zip(low, high)) for _ in range(count)]


def weights(seed, count, share=1.0):
    rng = random.Random(seed)
    return [round(rng.uniform(0.05, 1.0), 4) if rng.random() < share else 0.0 for _ in range(count)]


CHAIN = {
    'name': 'Rig',
    'location': (0.4, -0.2, 0.1),
    'rotation': (0.1, -0.2, 0.3),
    'bones': [
        {'name': 'Root', 'head': (0, 0, 0), 'tail': (0, 0, 1), 'roll': 0.2, 'headRadius': 0.15, 'tailRadius': 0.1, 'distance': 0.3},
        {'name': 'Mid', 'head': (0, 0, 1), 'tail': (0.1, 0.2, 1.9), 'parent': 'Root', 'connected': True, 'headRadius': 0.1, 'tailRadius': 0.08, 'distance': 0.35, 'weight': 0.6},
        {'name': 'Tip', 'head': (0.1, 0.2, 1.9), 'tail': (0.1, 0.5, 2.4), 'roll': -0.4, 'parent': 'Mid', 'connected': True, 'headRadius': 0.08, 'tailRadius': 0.02, 'distance': 0.2, 'weight': 1.5},
        {'name': 'Helper', 'head': (0.5, 0, 0.5), 'tail': (0.5, 0, 1.5), 'deform': False, 'headRadius': 0.5, 'tailRadius': 0.5, 'distance': 1.0},
        {'name': 'Side', 'head': (-0.6, 0, 0.8), 'tail': (-0.6, 0.6, 0.8), 'headRadius': 0.05, 'tailRadius': 0.12, 'distance': 0.0},
    ],
    'pose': {
        'Root': {'rotation': (0.3, 0.1, -0.2)},
        'Mid': {'rotation': (-0.5, 0.2, 0.4)},
        'Tip': {'rotation': (0.2, -0.6, 0.1), 'location': (0, 0, 0)},
        'Helper': {'rotation': (0.9, 0, 0)},
        'Side': {'location': (0.1, -0.05, 0.2), 'rotation': (0, 0.4, 0.7)},
    },
}
SCALED = json.loads(json.dumps(CHAIN))
SCALED['pose']['Mid']['scale'] = (1.0, 1.3, 0.8)
SCALED['pose']['Side']['scale'] = (1.2, 1.2, 1.2)
CLOUD = points(1, 80, (-0.9, -0.5, -0.3), (0.7, 0.9, 2.6))
NAMES = ['Root', 'Mid', 'Tip', 'Helper', 'Side']


def chain_case(rig, **settings):
    return {
        'rigs': [rig],
        'points': CLOUD,
        'meshLocation': (0.3, 0.1, -0.2),
        'meshRotation': (0.05, 0.2, 0.25),
        'groups': settings.pop('groups', {}),
        'modifiers': [dict({'type': 'armature', 'rig': 0}, **settings)],
    }


# Envelopes alone, linear and dual quaternions (a turned and scaled bone).
deform_case('envelopes', chain_case(CHAIN, vertexGroups=False, envelopes=True))
deform_case('envelopes-dq', chain_case(SCALED, vertexGroups=False, envelopes=True, preserveVolume=True))
# Groups where a vertex has them, envelopes elsewhere (a group no bone has does not count).
MIXED = {name: weights(10 + i, len(CLOUD), 0.35) for i, name in enumerate(NAMES)}
MIXED['Extra'] = weights(20, len(CLOUD), 0.5)
deform_case('groups-and-envelopes', chain_case(CHAIN, envelopes=True, groups=MIXED))
MULTIPLY = json.loads(json.dumps(CHAIN))
MULTIPLY['bones'][0]['multiply'] = True
MULTIPLY['bones'][1]['multiply'] = True
deform_case('multiply', chain_case(MULTIPLY, groups=MIXED))
deform_case('multiply-and-envelopes-dq', chain_case(json.loads(json.dumps(MULTIPLY, default=list)) | {'pose': SCALED['pose']}, envelopes=True, preserveVolume=True, groups=MIXED))
FULL = {name: weights(30 + i, len(CLOUD), 0.6) for i, name in enumerate(NAMES)}
FULL['Mask'] = weights(40, len(CLOUD), 0.8)
deform_case('vertex-group', chain_case(CHAIN, groups=FULL, vertexGroup='Mask'))
deform_case('vertex-group-inverted', chain_case(CHAIN, groups=FULL, vertexGroup='Mask', invertVertexGroup=True, envelopes=True))
deform_case('vertex-group-missing', chain_case(CHAIN, groups={name: FULL[name] for name in NAMES}, vertexGroup='Mask', invertVertexGroup=True))

# Multi Modifier: the second Armature modifier deforms what the first got.
SECOND = {
    'name': 'Second',
    'location': (-0.1, 0.3, 0.0),
    'rotation': (0, 0, -0.4),
    'bones': [{'name': 'Lift', 'head': (0, 0, 0), 'tail': (0, 0, 2.5), 'headRadius': 2.0, 'tailRadius': 2.0}],
    'pose': {'Lift': {'location': (0.3, 0.2, -0.1), 'rotation': (0.4, 0.3, 0.0)}},
}
THIRD = {
    'name': 'Third',
    'bones': [{'name': 'Twist', 'head': (0, 0, 0), 'tail': (0, 0, 2.5)}],
    'pose': {'Twist': {'rotation': (0, 0.8, 0)}},
}
STACK_GROUPS = dict(FULL, Lift=[1.0] * len(CLOUD), Twist=[1.0] * len(CLOUD))


def stack_case(modifiers):
    return {
        'rigs': [CHAIN, SECOND, THIRD],
        'points': CLOUD,
        'meshLocation': (0.3, 0.1, -0.2),
        'meshRotation': (0.05, 0.2, 0.25),
        'groups': STACK_GROUPS,
        'modifiers': modifiers,
    }


deform_case('multi', stack_case([{'type': 'armature', 'rig': 0}, {'type': 'armature', 'rig': 1, 'multiModifier': True, 'vertexGroup': 'Mask'}]))
deform_case('multi-three', stack_case([
    {'type': 'armature', 'rig': 0},
    {'type': 'armature', 'rig': 1, 'multiModifier': True, 'vertexGroup': 'Mask', 'invertVertexGroup': True},
    {'type': 'armature', 'rig': 2, 'multiModifier': True, 'vertexGroup': 'Mask'},
]))
deform_case('multi-after-displace', stack_case([
    {'type': 'displace', 'direction': 'x', 'strength': 0.5, 'midLevel': 0.0},
    {'type': 'armature', 'rig': 1, 'multiModifier': True, 'vertexGroup': 'Mask'},
]))
# Dual quaternions blending turned and scaled bones with others.
DQ = {
    'name': 'Rig',
    'bones': [
        {'name': 'B1', 'head': (0, 0, 0), 'tail': (0, 0, 1), 'roll': 0.3},
        {'name': 'B2', 'head': (0, 0, 1), 'tail': (0, 0.2, 2), 'parent': 'B1', 'connected': True},
        {'name': 'B3', 'head': (0.2, -0.3, 0.4), 'tail': (0.5, 0.1, 1.6), 'roll': 0.4},
    ],
    'pose': {
        'B1': {'rotation': (0.5, 0, 0.2), 'scale': (1.1, 0.9, 1.0)},
        'B2': {'rotation': (0, 0.3, -1.9), 'scale': (1, 1.4, 0.8)},
        'B3': {'rotation': (0.3, 0.2, 0.5), 'location': (0.1, 0, -0.2), 'scale': (1.5, 1.5, 1.5)},
    },
}
DQ_POINTS = points(3, 60, (-0.4, -0.4, 0.0), (0.6, 0.4, 2.0))
deform_case('dual-quaternions', {
    'rigs': [DQ],
    'points': DQ_POINTS,
    'meshLocation': (0.5, 0.2, 0.1),
    'meshRotation': (0.2, 0, 0.4),
    'groups': {name: weights(50 + i, len(DQ_POINTS), 0.7) for i, name in enumerate(['B1', 'B2', 'B3'])},
    'modifiers': [{'type': 'armature', 'rig': 0, 'preserveVolume': True}],
})


# Weight paint: the 3D view seen from the top, orthographic, and the vertices' places on it.
window = bpy.context.window_manager.windows[0]
area = [a for a in window.screen.areas if a.type == 'VIEW_3D'][0]
region = [r for r in area.regions if r.type == 'WINDOW'][0]
space = area.spaces.active
view = space.region_3d
view.view_perspective = 'ORTHO'
view.view_rotation = Quaternion((1, 0, 0, 0))
view.view_location = (0, 0, 0)
view.view_distance = 3
view.update()
PRESETS = {'smooth': 'SMOOTH', 'sphere': 'SPHERE', 'root': 'ROOT', 'sharp': 'SHARP', 'linear': 'LIN', 'constant': 'CONSTANT'}


def screen_of(mesh):
    return [vec(view3d_utils.location_3d_to_region_2d(region, view, mesh.matrix_world @ v.co)) for v in mesh.data.vertices]


def group_weights(mesh):
    result = {}
    for group in mesh.vertex_groups:
        result[group.name] = [0.0] * len(mesh.data.vertices)
    for v in mesh.data.vertices:
        for entry in v.groups:
            result[mesh.vertex_groups[entry.group].name][v.index] = round(entry.weight, 7)
    return result


def paint_mesh(positions, faces, groups, active):
    fresh()
    data = bpy.data.meshes.new('Paint')
    data.from_pydata(positions, [], faces)
    mesh = bpy.data.objects.new('Paint', data)
    bpy.context.collection.objects.link(mesh)
    for name, values in groups.items():
        group = mesh.vertex_groups.new(name=name)
        for index, weight in enumerate(values):
            if weight > 0:
                group.add([index], weight, 'REPLACE')
    mesh.vertex_groups.active = mesh.vertex_groups[active]
    bpy.context.view_layer.objects.active = mesh
    mesh.select_set(True)
    return mesh


def brush_for(settings):
    paint = bpy.context.scene.tool_settings.weight_paint
    brush = paint.brush
    brush.curve_distance_falloff_preset = PRESETS[settings['falloff']]
    brush.weight = settings['weight']
    brush.strength = settings['strength']
    brush.falloff_shape = 'PROJECTED'
    unified = paint.unified_paint_settings
    unified.use_unified_size = True
    # The unified size is the brush's diameter in pixels.
    unified.size = int(round(settings.get('radius', 50) * 2))
    return brush


def grid(nx, ny, size):
    positions = [(size * (x / nx - 0.5), size * (y / ny - 0.5), 0.0) for y in range(ny + 1) for x in range(nx + 1)]
    faces = [(y * (nx + 1) + x, y * (nx + 1) + x + 1, (y + 1) * (nx + 1) + x + 1, (y + 1) * (nx + 1) + x) for y in range(ny) for x in range(nx)]
    return [vec(p) for p in positions], faces


def gradient_case(label, kind, line, settings, before):
    positions, faces = grid(12, 8, 2.0)
    groups = {'Group': before(positions)}
    mesh = paint_mesh(positions, faces, groups, 'Group')
    with bpy.context.temp_override(window=window, area=area, region=region):
        bpy.ops.object.mode_set(mode='WEIGHT_PAINT')
        brush_for(settings)
        bpy.ops.paint.weight_gradient(type=kind.upper(), xstart=line[0], ystart=line[1], xend=line[2], yend=line[3])
        bpy.ops.object.mode_set(mode='OBJECT')
    out.setdefault('gradient', {})[label] = {
        'kind': kind, 'line': line, 'brush': settings, 'screen': screen_of(mesh), 'before': groups['Group'], 'after': group_weights(mesh)['Group'],
    }


flat = lambda value: (lambda positions: [value] * len(positions))
ramp = lambda positions: [round(0.1 + 0.35 * (p[0] + 1) + 0.1 * p[1], 4) for p in positions]
gradient_case('linear', 'linear', [650, 420, 1000, 520], {'weight': 1.0, 'strength': 1.0, 'falloff': 'linear'}, flat(0.0))
gradient_case('linear-smooth', 'linear', [1100, 300, 600, 650], {'weight': 0.7, 'strength': 0.6, 'falloff': 'smooth'}, ramp)
gradient_case('linear-constant', 'linear', [500, 477, 900, 477], {'weight': 0.3, 'strength': 0.8, 'falloff': 'constant'}, flat(0.6))
gradient_case('radial-sphere', 'radial', [800, 480, 1050, 700], {'weight': 0.9, 'strength': 0.9, 'falloff': 'sphere'}, flat(0.1))
gradient_case('radial-root', 'radial', [700, 400, 900, 400], {'weight': 0.0, 'strength': 0.7, 'falloff': 'root'}, ramp)
gradient_case('radial-sharp', 'radial', [787, 477, 1000, 477], {'weight': 0.5, 'strength': 1.0, 'falloff': 'sharp'}, flat(0.0))


# X mirror: a grid with a triangle hanging below its middle (so only X mirrors it), painted with
# one-dab strokes at given places.
def pendant(jitter):
    positions, faces = grid(12, 6, 1.6)
    centre = 6
    positions.append(vec((0.0, -1.1, 0.0)))
    tip = len(positions) - 1
    faces = faces + [(centre - 1, tip, centre), (centre, tip, centre + 1)]
    rng = random.Random(7)
    if jitter:
        positions = [vec((p[0] + rng.uniform(-jitter, jitter), p[1] + rng.uniform(-jitter, jitter), p[2])) if p[0] < -1e-6 else p for p in positions]
    return positions, faces


def mirror_case(label, groups, active, dabs, settings, jitter=0.0, names=True, topology=False):
    positions, faces = pendant(jitter)
    before = {name: values(positions) for name, values in groups.items()}
    mesh = paint_mesh(positions, faces, before, active)
    mesh.data.use_mirror_x = True
    mesh.data.use_mirror_vertex_groups = names
    mesh.data.use_mirror_topology = topology
    with bpy.context.temp_override(window=window, area=area, region=region):
        bpy.ops.object.mode_set(mode='WEIGHT_PAINT')
        brush_for(settings)
        for at in dabs:
            location = tuple(view3d_utils.region_2d_to_location_3d(region, view, at, (0, 0, 0)))
            stroke = [{'name': '', 'location': location, 'mouse': at, 'mouse_event': at, 'pressure': 1.0, 'size': settings['radius'] * 2, 'time': 0.0, 'is_start': True, 'x_tilt': 0, 'y_tilt': 0}]
            bpy.ops.paint.weight_paint(stroke=stroke)
        bpy.ops.object.mode_set(mode='OBJECT')
    out.setdefault('mirror', {})[label] = {
        'positions': positions, 'faces': faces, 'screen': screen_of(mesh), 'active': active, 'dabs': dabs, 'brush': settings,
        'names': names, 'topology': topology, 'before': before, 'after': group_weights(mesh),
    }


uneven = lambda base, slope: (lambda positions: [round(base + slope * p[0] + 0.05 * p[1], 4) for p in positions])
mirror_case('sides', {'Arm.L': uneven(0.4, 0.2), 'Arm.R': flat(0.6)}, 'Arm.L', [[1000, 477], [900, 560], [800, 477]], {'weight': 0.9, 'strength': 0.8, 'falloff': 'linear', 'radius': 70})
mirror_case('made-side', {'Leg_L': flat(0.2)}, 'Leg_L', [[1030, 420], [787, 600]], {'weight': 1.0, 'strength': 1.0, 'falloff': 'smooth', 'radius': 60})
mirror_case('one-group', {'Spine': uneven(0.5, 0.3)}, 'Spine', [[774, 477], [820, 420], [700, 600]], {'weight': 0.0, 'strength': 0.8, 'falloff': 'linear', 'radius': 80})
mirror_case('names-off', {'Arm.L': flat(0.1), 'Arm.R': flat(0.7)}, 'Arm.L', [[1000, 477], [760, 520]], {'weight': 0.8, 'strength': 0.9, 'falloff': 'sphere', 'radius': 65}, names=False)
mirror_case('topology', {'Hand.R': flat(0.3), 'Hand.L': flat(0.5)}, 'Hand.R', [[1000, 477], [900, 600], [800, 200]], {'weight': 1.0, 'strength': 0.7, 'falloff': 'linear', 'radius': 70}, jitter=0.03, topology=True)
mirror_case('jittered', {'Hand.R': flat(0.3), 'Hand.L': flat(0.5)}, 'Hand.R', [[1000, 477], [900, 600]], {'weight': 1.0, 'strength': 0.7, 'falloff': 'linear', 'radius': 70}, jitter=0.03)

# Names flipped for the other side.
FLIPS = [
    'Arm.L', 'Arm.R', 'Arm.l', 'Arm.r', 'Arm_L', 'Arm_R', 'Arm-L', 'Arm L', 'ArmL', 'L.Arm', 'R_Arm', 'l-arm', 'r Arm', 'A.L', 'A_r', '_L', 'L_',
    'Left Arm', 'LeftArm', 'leftArm', 'Arm.Left', 'ArmLeft', 'ArmRight', 'arm_left', 'ARM_LEFT', 'Arm.LEFT', 'Arm_Right', 'Arm-right', 'Leftover',
    'upperLeft', 'Body.left', 'Bright', 'LeftRight', 'RightLeft', 'rightRight', 'LeftLeft', 'xLeft', 'xright', 'Leftx', 'rightx', 'Lefts', 'Right_',
    'Left', 'Right', 'L', 'Hand', 'aLEFTb', 'a.left.b', 'Arm.Lefty', 'LeFT.xy', 'LEft.xy', 'lEFT.xy', 'rIGHT.xy', 'RiGHT.xy', 'RIght.xy', 'xy.LeFT',
    'Arm.L.001', 'Arm.R.002', 'Arm.L001', 'Arm.L.1', 'Arm.L.a12', 'Arm_L.001', 'L.001', 'Arm.L.001.002', 'Leftover.001', 'xx.Right.001', 'Hand.L.Finger',
    'Arm.L.R', 'L.Arm.R', 'Arm..L', 'Arm.-L', 'Arm.L.', 'Arm.L ', 'ab:L', 'Lefty.L', 'R.Leftover', 'Right.001', 'Lefter.L.003',
]
out['names'] = {name: bpy.utils.flip_name(name) for name in FLIPS}


# Custom bone shapes: the shape's edges as the viewport draws them, seen from the top and the
# front (orthographic), as the pixels that differ from the same view without the armature.
SHAPE = [(0, 0, 0), (0.6, 0, 0), (0.6, 0.3, 0.2), (0.1, 0.4, -0.3)]
SHAPE_EDGES = [(0, 1), (1, 2), (2, 3)]
SIZE = 256
SCALE = 0.4
VIEWS = {
    'top': Matrix.Translation((0, 0, -10)),
    'front': Matrix.Translation((0, 0, -10)) @ Matrix.Rotation(math.radians(-90), 4, 'X'),
}
PROJECTION = Matrix(((SCALE, 0, 0, 0), (0, SCALE, 0, 0), (0, 0, -0.05, 0), (0, 0, 0, 1)))
overlay = space.overlay
overlay.show_floor = False
overlay.show_axis_x = overlay.show_axis_y = overlay.show_axis_z = False
overlay.show_object_origins = False


def grab(matrix):
    offscreen = gpu.types.GPUOffScreen(SIZE, SIZE)
    offscreen.draw_view3d(bpy.context.scene, bpy.context.view_layer, space, region, matrix, PROJECTION, do_color_management=False)
    with offscreen.bind():
        pixels = gpu.state.active_framebuffer_get().read_color(0, 0, SIZE, SIZE, 4, 0, 'UBYTE').to_list()
    offscreen.free()
    return pixels


def shape_case(label, case):
    fresh()
    rig = make_rig(case['rig'])
    data = bpy.data.meshes.new('Shape')
    data.from_pydata(SHAPE, SHAPE_EDGES, [])
    shape = bpy.data.objects.new('Shape', data)
    bpy.context.collection.objects.link(shape)
    shape.location = (7, 7, 7)
    shape.hide_set(True)
    for name, settings in case['shapes'].items():
        pb = rig.pose.bones[name]
        pb.custom_shape = shape
        pb.custom_shape_translation = settings.get('translation', (0, 0, 0))
        pb.custom_shape_rotation_euler = settings.get('rotation', (0, 0, 0))
        pb.custom_shape_scale_xyz = settings.get('scale', (1, 1, 1))
        pb.use_custom_shape_bone_size = settings.get('boneSize', True)
        rig.data.bones[name].show_wire = True
    update()
    drawn = {}
    for name, matrix in VIEWS.items():
        shown = grab(matrix)
        rig.hide_set(True)
        update()
        hidden = grab(matrix)
        rig.hide_set(False)
        update()
        drawn[name] = [[x, y] for y in range(SIZE) for x in range(SIZE) if sum(abs(shown[y][x][k] - hidden[y][x][k]) for k in range(3)) > 6]
    case['pixels'] = drawn
    out.setdefault('shapes', {})[label] = case


out['shapeMesh'] = {'positions': [list(p) for p in SHAPE], 'edges': [list(e) for e in SHAPE_EDGES]}
out['shapeViews'] = {name: [list(row) for row in matrix] for name, matrix in VIEWS.items()}
out['shapeProjection'] = [list(row) for row in PROJECTION]
out['shapeSize'] = SIZE
shape_case('offset', {
    'rig': {'name': 'Rig', 'bones': [{'name': 'B', 'head': (0, 0, 0), 'tail': (0, 0, 2), 'roll': 0.3}]},
    'shapes': {'B': {'translation': (0.2, 0.1, -0.3), 'rotation': (0.4, 0.2, -0.5), 'scale': (1.5, 0.7, 1.2)}},
})
shape_case('posed-unscaled', {
    'rig': {
        'name': 'Rig', 'location': (0.2, 0.1, 0.0), 'rotation': (0, 0, 0.4),
        'bones': [{'name': 'B', 'head': (0.3, -0.2, 0.1), 'tail': (1.2, 0.4, 1.0), 'roll': -0.6}],
        'pose': {'B': {'location': (0.1, 0.05, -0.1), 'rotation': (0.2, -0.4, 0.3), 'scale': (1.2, 0.9, 1.1)}},
    },
    'shapes': {'B': {'translation': (-0.2, 0.3, 0.1), 'rotation': (-0.3, 0.5, 0.2), 'scale': (0.8, 1.3, 0.9), 'boneSize': False}},
})
shape_case('chain', {
    'rig': {
        'name': 'Rig',
        'bones': [
            {'name': 'Lower', 'head': (-1, 0, -1), 'tail': (-1, 0, 0)},
            {'name': 'Upper', 'head': (-1, 0, 0), 'tail': (0, 0.3, 1.2), 'parent': 'Lower', 'connected': True, 'roll': 0.8},
        ],
        'pose': {'Lower': {'rotation': (0.3, 0, -0.4)}, 'Upper': {'rotation': (0, 0.6, 0.2), 'scale': (0.7, 0.7, 0.7)}},
    },
    'shapes': {'Lower': {'scale': (2, 2, 2)}, 'Upper': {'translation': (0, 0.5, 0), 'rotation': (0, 0, 1.2)}},
})

json.dump(out, open(sys.argv[sys.argv.index('--') + 1], 'w'))
