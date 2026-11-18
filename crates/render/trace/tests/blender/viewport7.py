"""Clean-room viewport facts and orbit reference images, from Blender's public Python API.
Run: Blender --background --threads 2 --python viewport7.py -- <output-directory>
The render uses a camera equivalent to the orbit; no Blender implementation is consulted.
Manual: https://docs.blender.org/manual/en/latest/editors/3dview/display/shading.html
"""
import bpy
import json
import math
import pathlib
import sys
from mathutils import Euler, Vector

out = pathlib.Path(sys.argv[sys.argv.index('--') + 1])
out.mkdir(parents=True, exist_ok=True)
bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene
world = bpy.data.worlds.new('World')
world.use_nodes = True
world.node_tree.nodes['Background'].inputs['Color'].default_value = (0.05, 0.05, 0.05, 1)
world.node_tree.nodes['Background'].inputs['Strength'].default_value = 1
scene.world = world

def material(name, color, roughness):
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    node = mat.node_tree.nodes['Principled BSDF']
    node.inputs['Base Color'].default_value = (*color, 1)
    node.inputs['Roughness'].default_value = roughness
    return mat

bpy.ops.mesh.primitive_cube_add(size=2, location=(0, 0, 1))
bpy.context.object.data.materials.append(material('Blue', (0.1, 0.25, 0.7), 0.35))
bpy.ops.mesh.primitive_plane_add(size=12)
bpy.context.object.data.materials.append(material('Floor', (0.65, 0.65, 0.65), 0.7))
bpy.ops.object.light_add(type='AREA', location=(0, 0, 5))
bpy.context.object.data.energy = 400
bpy.context.object.data.shape = 'SQUARE'
bpy.context.object.data.size = 2
rotation = Euler((math.radians(60), 0, math.radians(30)), 'XYZ')
location = Vector((0, 0, 1)) + rotation.to_matrix() @ Vector((0, 0, 8))
bpy.ops.object.camera_add(location=location, rotation=rotation)
scene.camera = bpy.context.object
camera = scene.camera.data
camera.lens = 50
camera.sensor_width = 72
camera.sensor_fit = 'HORIZONTAL'
scene.render.engine = 'CYCLES'
scene.cycles.device = 'CPU'
scene.cycles.samples = 512
scene.cycles.use_denoising = False
scene.cycles.use_adaptive_sampling = False
scene.cycles.seed = 0
scene.render.resolution_x, scene.render.resolution_y, scene.render.resolution_percentage = 64, 48, 100
scene.view_settings.view_transform = 'Standard'
scene.render.image_settings.file_format = 'PNG'
scene.render.image_settings.color_mode = 'RGBA'
for name in ('perspective', 'orthographic'):
    camera.type = 'PERSP' if name == 'perspective' else 'ORTHO'
    camera.ortho_scale = 8 * 72 / 50
    scene.render.filepath = str(out / (name + '.RENDER.CYCLES.png'))
    bpy.ops.render.render(write_still=True)
properties = bpy.types.View3DShading.bl_rna.properties
facts = {
    'version': bpy.app.version_string,
    'engine': 'CYCLES',
    'samples': 512,
    'size': [64, 48],
    'viewTransform': 'Standard',
    'shadingTypes': [item.identifier for item in properties['type'].enum_items],
    'renderedUsesSceneLights': properties['use_scene_lights_render'].default,
    'renderedUsesSceneWorld': properties['use_scene_world_render'].default,
    'orbit': {'target': [0, 0, 1], 'distance': 8, 'tilt': 60, 'turn': 30, 'lens': 50},
}
(out / 'reference.json').write_text(json.dumps(facts, indent=2) + '\n')
