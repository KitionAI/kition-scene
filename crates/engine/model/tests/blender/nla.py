# The reference side of tests/nla_blender.rs: builds each case of tests/fixtures/nla-cases.json
# in Blender (run with `blender -b --factory-startup --python nla.py -- cases.json out.json`)
# and writes the evaluated values at each case's frames (and, for auto-blend cases, the blends
# Blender worked out). The NLA, the active action's blending, tweak mode, and F-curve modifiers
# are set up only through Blender's Python API and its NLA editor operators.
import json
import math
import sys

import bpy

args = sys.argv[sys.argv.index('--') + 1:]
cases = json.load(open(args[0]))['cases']
out = {}

BLEND = {'replace': 'REPLACE', 'combine': 'COMBINE', 'add': 'ADD', 'subtract': 'SUBTRACT', 'multiply': 'MULTIPLY'}
EXTRAPOLATION = {'hold': 'HOLD', 'holdForward': 'HOLD_FORWARD', 'nothing': 'NOTHING'}
INTERPOLATION = {'bezier': 'BEZIER', 'linear': 'LINEAR', 'constant': 'CONSTANT'}
CYCLE = {'none': 'NONE', 'repeat': 'REPEAT', 'repeatOffset': 'REPEAT_OFFSET', 'mirror': 'MIRROR'}


def fresh():
    bpy.ops.wm.read_factory_settings(use_empty=True)


def split(path):
    """(owner name or None, rna path, index or None)"""
    bone = None
    if path.startswith('bone:'):
        _, bone, path = path.split(':', 2)
    if '.' in path:
        name, index = path.rsplit('.', 1)
        return bone, name, int(index)
    return bone, path, None


def holder(ob, bone):
    return ob.pose.bones[bone] if bone else ob


def write(ob, path, value):
    bone, name, index = split(path)
    owner = holder(ob, bone)
    if index is None:
        setattr(owner, name, bool(value) if name.startswith('hide') else value)
    else:
        getattr(owner, name)[index] = value


def read(ob, path):
    bone, name, index = split(path)
    owner = holder(ob, bone)
    value = getattr(owner, name) if index is None else getattr(owner, name)[index]
    return float(value)


def add_modifier(fc, spec):
    kind = spec['type']
    if kind == 'generator':
        m = fc.modifiers.new('GENERATOR')
        coefficients = spec['coefficients']
        if spec.get('mode') == 'factorized':
            m.mode = 'POLYNOMIAL_FACTORISED'
            m.poly_order = len(coefficients) // 2
        else:
            m.mode = 'POLYNOMIAL'
            m.poly_order = max(1, len(coefficients) - 1)
            coefficients = coefficients + [0.0] * (m.poly_order + 1 - len(coefficients))
        m.coefficients = coefficients
        m.use_additive = spec.get('additive', False)
    elif kind == 'function':
        m = fc.modifiers.new('FNGENERATOR')
        m.function_type = spec['function'].upper()
        m.amplitude = spec['amplitude']
        m.phase_multiplier = spec['phaseMultiplier']
        m.phase_offset = spec['phaseOffset']
        m.value_offset = spec['valueOffset']
        m.use_additive = spec.get('additive', False)
    elif kind == 'envelope':
        m = fc.modifiers.new('ENVELOPE')
        m.reference_value = spec['reference']
        m.default_min = spec['min']
        m.default_max = spec['max']
        for point in spec['points']:
            p = m.control_points.add(point['frame'])
            p.min = point['min']
            p.max = point['max']
    elif kind == 'cycles':
        m = fc.modifiers.new('CYCLES')
        m.mode_before = CYCLE[spec['before']]
        m.mode_after = CYCLE[spec['after']]
        m.cycles_before = spec.get('beforeCount', 0)
        m.cycles_after = spec.get('afterCount', 0)
    elif kind == 'noise':
        m = fc.modifiers.new('NOISE')
        m.blend_type = spec['blend'].upper()
        m.scale = spec['scale']
        m.strength = spec['strength']
        m.phase = spec['phase']
        m.offset = spec['offset']
        m.depth = spec['depth']
        m.lacunarity = spec['lacunarity']
        m.roughness = spec['roughness']
    elif kind == 'stepped':
        m = fc.modifiers.new('STEPPED')
        m.frame_step = spec['step']
        m.frame_offset = spec['offset']
        if 'start' in spec:
            m.use_frame_start = True
            m.frame_start = spec['start']
        if 'end' in spec:
            m.use_frame_end = True
            m.frame_end = spec['end']
    else:
        raise ValueError(kind)
    if 'influence' in spec:
        m.use_influence = True
        m.influence = spec['influence']
    if 'range' in spec:
        r = spec['range']
        m.use_restricted_range = True
        m.frame_end = r['end']
        m.frame_start = r['start']
        m.blend_in = r['blendIn']
        m.blend_out = r['blendOut']
    m.mute = spec.get('muted', False)


def make_action(ob, name, channels):
    ad = ob.animation_data
    action = bpy.data.actions.new(name)
    ad.action = action
    for path, channel in channels.items():
        bone, rna, index = split(path)
        owner = holder(ob, bone)
        for frame, value in channel['keys']:
            write(ob, path, value)
            if index is None:
                owner.keyframe_insert(rna, frame=frame)
            else:
                owner.keyframe_insert(rna, index=index, frame=frame)
    bag = action.layers[0].strips[0].channelbags[0]
    for path, channel in channels.items():
        bone, rna, index = split(path)
        data_path = f'pose.bones["{bone}"].{rna}' if bone else rna
        fc = bag.fcurves.find(data_path, index=index or 0)
        for key in fc.keyframe_points:
            key.interpolation = INTERPOLATION[channel.get('interpolation', 'bezier')]
        for spec in channel.get('modifiers', []):
            add_modifier(fc, spec)
        fc.update()
    ad.action = None
    return action


def build(case):
    fresh()
    scene = bpy.context.scene
    if case.get('armature'):
        data = bpy.data.armatures.new('Rig')
        ob = bpy.data.objects.new('Rig', data)
        scene.collection.objects.link(ob)
        bpy.context.view_layer.objects.active = ob
        bpy.ops.object.mode_set(mode='EDIT')
        bone = data.edit_bones.new('Bone')
        bone.head = (0, 0, 0)
        bone.tail = (0, 0, 1)
        bpy.ops.object.mode_set(mode='OBJECT')
        for pb in ob.pose.bones:
            pb.rotation_mode = 'XYZ'
    else:
        ob = bpy.data.objects.new('E', None)
        scene.collection.objects.link(ob)
    bpy.context.view_layer.objects.active = ob
    ob.select_set(True)
    ob.animation_data_create()
    ad = ob.animation_data
    actions = {name: make_action(ob, name, channels) for name, channels in case.get('actions', {}).items()}
    own = make_action(ob, 'Own', case['own']) if 'own' in case else None
    # Keying left the last keyed values behind: back to rest.
    ob.location = (0, 0, 0)
    ob.rotation_euler = (0, 0, 0)
    ob.scale = (1, 1, 1)
    ob.hide_viewport = False
    if case.get('armature'):
        for pb in ob.pose.bones:
            pb.location = (0, 0, 0)
            pb.rotation_euler = (0, 0, 0)
            pb.scale = (1, 1, 1)
    strips = []
    for spec in case.get('tracks', []):
        track = ad.nla_tracks.new()
        row = []
        for s in spec['strips']:
            strip = track.strips.new(s['action'], int(s['start']), actions[s['action']])
            if 'actionEnd' in s:
                strip.action_frame_end = s['actionEnd']
            if 'actionStart' in s:
                strip.action_frame_start = s['actionStart']
            strip.scale = s.get('scale', 1.0)
            strip.repeat = s.get('repeat', 1.0)
            strip.use_reverse = s.get('reversed', False)
            strip.extrapolation = EXTRAPOLATION[s.get('extrapolation', 'hold')]
            strip.blend_type = BLEND[s.get('blend', 'replace')]
            strip.blend_in = s.get('blendIn', 0.0)
            strip.blend_out = s.get('blendOut', 0.0)
            if 'influence' in s:
                strip.use_animated_influence = True
                strip.influence = s['influence']
                strip.fcurves[0].keyframe_points[0].co = (strip.frame_start, s['influence'])
                strip.fcurves[0].update()
            strip.mute = s.get('muted', False)
            row.append((strip, s))
        track.mute = spec.get('muted', False)
        strips.append((track, row))
    # Auto blends are worked out when switched on (switching them off clears the blends).
    for _, row in strips:
        for strip, s in row:
            if s.get('autoBlend'):
                strip.use_auto_blend = True
    for (track, _), spec in zip(strips, case.get('tracks', [])):
        if spec.get('solo'):
            track.is_solo = True
    if own is not None:
        ad.action = own
    blend = case.get('ownBlend', {})
    ad.action_blend_type = BLEND[blend.get('blend', 'replace')]
    ad.action_influence = blend.get('influence', 1.0)
    ad.action_extrapolation = EXTRAPOLATION[blend.get('extrapolation', 'hold')]
    if 'tweak' in case:
        tweak = case['tweak']
        for track, row in strips:
            for strip, _ in row:
                strip.select = False
        track, row = strips[tweak['track']]
        row[tweak['strip']][0].select = True
        ad.nla_tracks.active = track
        window = bpy.context.window_manager.windows[0]
        area = window.screen.areas[0]
        area.type = 'NLA_EDITOR'
        region = [r for r in area.regions if r.type == 'WINDOW'][0]
        with bpy.context.temp_override(window=window, area=area, region=region):
            bpy.ops.nla.tweakmode_enter(use_upper_stack_evaluation=tweak['upperStack'])
        assert ad.use_tweak_mode
    for path, value in case.get('static', {}).items():
        write(ob, path, value)
    return ob, strips


for case in cases:
    ob, strips = build(case)
    scene = bpy.context.scene
    values = []
    for frame in case['frames']:
        whole = math.floor(frame)
        scene.frame_set(whole, subframe=frame - whole)
        values.append([read(ob, path) for path in case['paths']])
    result = {'values': values}
    if case.get('blends'):
        result['blends'] = [[[strip.blend_in, strip.blend_out] for strip, _ in row] for _, row in strips]
    out[case['name']] = result

json.dump(out, open(args[1], 'w'))
