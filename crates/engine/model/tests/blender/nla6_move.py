# Clean-room transition timing measured through Blender's public Python API.
# blender -b --factory-startup --python nla6_move.py -- reference.json
import json
import sys
import bpy

bpy.ops.wm.read_factory_settings(use_empty=True)
ob = bpy.data.objects.new('E', None)
bpy.context.scene.collection.objects.link(ob)
bpy.context.view_layer.objects.active = ob
ob.select_set(True)
ad = ob.animation_data_create()
for frame, value in [(1, 0), (11, 10)]:
    ob.location.x = value
    ob.keyframe_insert('location', index=0, frame=frame)
action = ad.action
ad.action = None
track = ad.nla_tracks.new()
left = track.strips.new('Left', 1, action)
right = track.strips.new('Right', 31, action)
left.select = right.select = True
window = bpy.context.window_manager.windows[0]
area = window.screen.areas[0]
area.type = 'NLA_EDITOR'
region = next(r for r in area.regions if r.type == 'WINDOW')
with bpy.context.temp_override(window=window, area=area, region=region):
    assert bpy.ops.nla.transition_add() == {'FINISHED'}

def strips():
    return [{'kind': {'CLIP': 'action', 'TRANSITION': 'transition', 'META': 'meta'}[s.type],
             'start': s.frame_start, 'end': s.frame_end} for s in track.strips]

def move(strip, start):
    length = strip.frame_end - strip.frame_start
    strip.frame_start = start
    strip.frame_end = start + length

recording = {'initial': strips()}
move(left, 12)
recording['leftMoved'] = strips()
left.scale = 0.5
recording['leftScaled'] = strips()
move(right, 24)
recording['rightMoved'] = strips()
for s in track.strips:
    s.select = s == left
with bpy.context.temp_override(window=window, area=area, region=region):
    assert bpy.ops.nla.meta_add() == {'FINISHED'}
meta = track.strips[0]
move(meta, 18)
recording['metaMoved'] = strips()
out = {'version': bpy.app.version_string, 'states': recording}
json.dump(out, open(sys.argv[sys.argv.index('--') + 1], 'w'), indent=2)
