# Clean-room observations through Blender's public Python API and NLA editor operators.
# blender -b --factory-startup --python nla6.py -- cases.json reference.json
import json
import math
import sys
import bpy

args = sys.argv[sys.argv.index('--') + 1:]
cases = json.load(open(args[0]))
BLENDS = {'replace': 'REPLACE', 'add': 'ADD', 'subtract': 'SUBTRACT', 'multiply': 'MULTIPLY', 'combine': 'COMBINE'}

def modifier(fc, spec):
    m = fc.modifiers.new({'limits': 'LIMITS', 'generator': 'GENERATOR', 'stepped': 'STEPPED'}[spec['type']])
    if spec['type'] == 'limits':
        for field, rna in [('minX', 'min_x'), ('maxX', 'max_x'), ('minY', 'min_y'), ('maxY', 'max_y')]:
            if spec.get(field) is not None:
                setattr(m, 'use_' + rna, True)
                setattr(m, rna, spec[field])
    elif spec['type'] == 'generator':
        m.poly_order = len(spec['coefficients']) - 1
        m.coefficients = spec['coefficients']
        m.use_additive = spec.get('additive', False)
    else:
        m.frame_step = spec['step']
        m.frame_offset = spec.get('offset', 0)
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

def action(ob, name, spec):
    a = bpy.data.actions.new(name)
    ob.animation_data.action = a
    for channel in spec:
        for f, v in channel['keys']:
            getattr(ob, channel['property'])[channel['index']] = v
            ob.keyframe_insert(channel['property'], index=channel['index'], frame=f)
    bag = a.layers[0].strips[0].channelbags[0]
    for channel in spec:
        fc = bag.fcurves.find(channel['property'], index=channel['index'])
        for key in fc.keyframe_points:
            key.interpolation = 'LINEAR'
        for m in channel.get('modifiers', []):
            modifier(fc, m)
        fc.update()
    ob.animation_data.action = None
    return a

def settings(s, spec):
    for key, field in [('scale','scale'), ('repeat','repeat'), ('reversed','use_reverse'), ('blendIn','blend_in'), ('blendOut','blend_out'), ('muted','mute')]:
        if key in spec:
            setattr(s, field, spec[key])
    if 'blend' in spec:
        s.blend_type = BLENDS[spec['blend']]
    if 'extrapolation' in spec:
        s.extrapolation = {'nothing':'NOTHING', 'hold':'HOLD', 'holdForward':'HOLD_FORWARD'}[spec['extrapolation']]
    if 'influence' in spec:
        s.use_animated_influence = True
        s.influence = spec['influence']
        s.fcurves[0].keyframe_points[0].co = (s.frame_start, spec['influence'])
        s.fcurves[0].update()

def run(case):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    ob = bpy.data.objects.new('E', None)
    scene.collection.objects.link(ob)
    bpy.context.view_layer.objects.active = ob
    ob.select_set(True)
    ad = ob.animation_data_create()
    actions = {name: action(ob, name, channels) for name, channels in case['actions'].items()}
    if 'modifiers' in case:
        fc = actions['A'].layers[0].strips[0].channelbags[0].fcurves[0]
        for m in case['modifiers']:
            modifier(fc, m)
        return [fc.evaluate(f) for f in case['frames']]
    for spec in case['tracks']:
        track = ad.nla_tracks.new()
        for entry in spec:
            s = track.strips.new(entry['action'], int(entry['start']), actions[entry['action']])
            settings(s, entry)
    window = bpy.context.window_manager.windows[0]
    area = window.screen.areas[0]
    area.type = 'NLA_EDITOR'
    region = next(r for r in area.regions if r.type == 'WINDOW')
    for operation in case.get('operations', []):
        for track in ad.nla_tracks:
            for s in track.strips:
                s.select = False
        track = ad.nla_tracks[operation['track']]
        ad.nla_tracks.active = track
        for slot in operation.get('strips', []):
            track.strips[slot].select = True
        with bpy.context.temp_override(window=window, area=area, region=region):
            if operation['type'] == 'transition':
                assert bpy.ops.nla.transition_add() == {'FINISHED'}
            elif operation['type'] == 'meta':
                assert bpy.ops.nla.meta_add() == {'FINISHED'}
            elif operation['type'] == 'dissolve':
                assert bpy.ops.nla.meta_remove() == {'FINISHED'}
            elif operation['type'] == 'set':
                s = track.strips[operation['strip']]
                if 'start' in operation:
                    s.frame_start = operation['start']
                if 'end' in operation:
                    s.frame_end = operation['end']
                settings(s, operation)
    ob.location = (0, 0, 0)
    ob.scale = (1, 1, 1)
    out = []
    for f in case['frames']:
        scene.frame_set(math.floor(f), subframe=f % 1)
        out.append(list(ob.location) + list(ob.scale))
    return out

out = {'version': bpy.app.version_string, 'cases': {case['name']: run(case) for case in cases['cases']}}
json.dump(out, open(args[1], 'w'), indent=2)
