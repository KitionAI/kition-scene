# Clean-room reference: runs Blender's public Python API only. No Blender source is read.
# Record: blender -b --factory-startup --python usdc.py -- <output-directory>
# Check a Cube export: blender -b --factory-startup --python usdc.py -- --report <file.usdc> <report.json>

import bpy, json, math, os, sys
args = sys.argv[sys.argv.index('--') + 1:]
FRAMES = [1, 11, 21]

def report(path):
    scene = bpy.context.scene
    out = {}
    for frame in FRAMES:
        scene.frame_set(frame)
        depsgraph = bpy.context.evaluated_depsgraph_get()
        for obj in scene.objects:
            entry = out.setdefault(obj.name, {'type': obj.type, 'worlds': [], 'deformed': []})
            entry['worlds'].append([v for col in zip(*obj.matrix_world) for v in col])
            if obj.type == 'MESH':
                mesh = obj.evaluated_get(depsgraph).to_mesh()
                entry['deformed'].append([list(obj.matrix_world @ v.co) for v in mesh.vertices])
                entry['vertices'] = len(obj.data.vertices)
                entry['faces'] = len(obj.data.polygons)
                # Vertex group weights, normalised per vertex.
                names = {group.index: group.name for group in obj.vertex_groups}
                groups = {name: [0.0] * len(obj.data.vertices) for name in names.values()}
                for vertex in obj.data.vertices:
                    total = sum(item.weight for item in vertex.groups if item.group in names)
                    for item in vertex.groups:
                        if item.group in names and total > 0:
                            groups[names[item.group]][vertex.index] = item.weight / total
                entry['groups'] = groups
                material = obj.active_material
                entry['material'] = None
                if material and material.node_tree:
                    surface = next((n for n in material.node_tree.nodes if n.type == 'BSDF_PRINCIPLED'), None)
                    links = surface.inputs['Base Color'].links if surface else []
                    if links and links[0].from_node.type == 'TEX_IMAGE' and links[0].from_node.image:
                        # A base colour image: its size (the factor stays on the shader).
                        entry['texture'] = list(links[0].from_node.image.size)
                    elif surface:
                        entry['material'] = list(surface.inputs['Base Color'].default_value)[:3] + [surface.inputs['Metallic'].default_value, surface.inputs['Roughness'].default_value]
                obj.evaluated_get(depsgraph).to_mesh_clear()
            if obj.type == 'LIGHT':
                light = obj.data
                size = light.size if light.type == 'AREA' else getattr(light, 'shadow_soft_size', 0.0)
                entry['light'] = [light.type, light.energy, size, getattr(light, 'spot_size', 0.0), list(light.color)]
            if obj.type == 'CAMERA':
                entry['camera'] = [obj.data.type, obj.data.lens, obj.data.sensor_width, obj.data.ortho_scale]
    json.dump(out, open(path, 'w'))

if args[0] == '--report':
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.wm.usd_import(filepath=args[1])
    report(args[2])
    sys.exit(0)

out = args[0]
os.makedirs(out, exist_ok=True)
json.dump({'blender': bpy.app.version_string}, open(os.path.join(out, 'version.json'), 'w'))
def save(name):
    for extension in ('usda', 'usdc', 'usdz'):
        bpy.ops.wm.usd_export(filepath=os.path.join(out, name + '.' + extension), export_animation=True)
    report(os.path.join(out, name + '.json'))

save('default')

bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene
scene.frame_start = 1; scene.frame_end = 21
bpy.ops.mesh.primitive_cube_add(location=(1, 2, 3), rotation=(0.3, 0.2, 0.1))
cube = bpy.context.active_object
material = bpy.data.materials.new('Painted'); material.use_nodes = True
surface = material.node_tree.nodes['Principled BSDF']
surface.inputs['Base Color'].default_value = (0.2, 0.4, 0.6, 1); surface.inputs['Metallic'].default_value = 0.3; surface.inputs['Roughness'].default_value = 0.7
cube.data.materials.append(material)
cube.keyframe_insert('location', frame=1); cube.keyframe_insert('rotation_euler', frame=1)
cube.location = (2, 2, 4); cube.rotation_euler = (0.3, 1.2, 0.1)
cube.keyframe_insert('location', frame=21); cube.keyframe_insert('rotation_euler', frame=21)
bpy.ops.mesh.primitive_uv_sphere_add(segments=12, ring_count=6, radius=0.5, location=(1, 2, 5))
sphere = bpy.context.active_object
sphere.parent = cube; sphere.matrix_parent_inverse = cube.matrix_world.inverted()
bpy.ops.object.shade_smooth()
bpy.ops.mesh.primitive_plane_add(location=(-2, 0, 0))
plane = bpy.context.active_object
bpy.ops.object.empty_add(location=(0, 0, 2), rotation=(0, 0, 0.5)); parent = bpy.context.active_object
parent.scale = (2, 2, 2)
plane.parent = parent
for kind, location in [('POINT', (0, 3, 3)), ('SUN', (0, 0, 5)), ('SPOT', (3, 0, 3)), ('AREA', (-3, 0, 3))]:
    bpy.ops.object.light_add(type=kind, location=location, rotation=(0.4, 0.1, 0))
    light = bpy.context.active_object; light.data.energy = {'POINT': 800.0, 'SUN': 3.0, 'SPOT': 500.0, 'AREA': 200.0}[kind]
    light.data.color = (1, 0.8, 0.6)
    if kind == 'SPOT':
        light.data.spot_size = math.radians(60); light.data.shadow_soft_size = 0.2
    if kind == 'AREA':
        light.data.size = 1.5
bpy.ops.object.camera_add(location=(0, -5, 2), rotation=(1.2, 0, 0.1))
bpy.context.active_object.data.lens = 35
# A plane with an image on its base colour.
bpy.ops.mesh.primitive_plane_add(location=(0, -2, 0))
tile = bpy.context.active_object; tile.name = 'Tile'
image = bpy.data.images.new('Checker', 8, 4)
image.pixels = [value for index in range(32) for value in ((1, 1, 1, 1) if (index + index // 8) % 2 else (0.1, 0.2, 0.8, 1))]
image.filepath_raw = os.path.join(out, 'checker.png'); image.file_format = 'PNG'; image.save()
tiled = bpy.data.materials.new('Tiled'); tiled.use_nodes = True
node = tiled.node_tree.nodes.new('ShaderNodeTexImage'); node.image = image
tiled.node_tree.links.new(node.outputs['Color'], tiled.node_tree.nodes['Principled BSDF'].inputs['Base Color'])
tile.data.materials.append(tiled)
save('things')

# A column skinned to a two-bone rig with automatic weights, a shape key, and keys on both.
import bmesh
bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene
scene.frame_start = 1; scene.frame_end = 21
mesh = bpy.data.meshes.new('Column')
column = bpy.data.objects.new('Column', mesh)
scene.collection.objects.link(column)
built = bmesh.new()
bmesh.ops.create_cone(built, cap_ends=True, segments=8, radius1=0.3, radius2=0.3, depth=2)
bmesh.ops.subdivide_edges(built, edges=[e for e in built.edges if abs(e.verts[0].co.z - e.verts[1].co.z) > 1.5], cuts=7)
built.to_mesh(mesh); built.free()
column.location = (0, 0, 1)
armature = bpy.data.armatures.new('Armature')
rig = bpy.data.objects.new('Rig', armature)
scene.collection.objects.link(rig)
bpy.context.view_layer.objects.active = rig
bpy.ops.object.mode_set(mode='EDIT')
lower = armature.edit_bones.new('Lower'); lower.head = (0, 0, 0); lower.tail = (0, 0, 1)
upper = armature.edit_bones.new('Upper'); upper.head = (0, 0, 1); upper.tail = (0, 0, 2); upper.parent = lower; upper.use_connect = True
bpy.ops.object.mode_set(mode='OBJECT')
bpy.ops.object.select_all(action='DESELECT')
column.select_set(True); rig.select_set(True); bpy.context.view_layer.objects.active = rig
bpy.ops.object.parent_set(type='ARMATURE_AUTO')
column.shape_key_add(name='Basis')
wide = column.shape_key_add(name='Wide')
for point in wide.data:
    point.co.x *= 1.5; point.co.y *= 1.5
wide.value = 0.0; wide.keyframe_insert('value', frame=1)
wide.value = 1.0; wide.keyframe_insert('value', frame=21)
bpy.context.view_layer.objects.active = rig
bpy.ops.object.mode_set(mode='POSE')
pose = rig.pose.bones['Upper']
pose.rotation_mode = 'XYZ'
pose.rotation_euler = (0, 0, 0); pose.keyframe_insert('rotation_euler', frame=1)
pose.rotation_euler = (0.8, 0.2, 0); pose.keyframe_insert('rotation_euler', frame=21)
bpy.ops.object.mode_set(mode='OBJECT')
# A shape-keyed grid no armature deforms.
bpy.ops.mesh.primitive_grid_add(x_subdivisions=3, y_subdivisions=3, size=1, location=(2, 0, 0))
flag = bpy.context.active_object; flag.name = 'Flag'
flag.shape_key_add(name='Basis')
lift = flag.shape_key_add(name='Lift')
for point in lift.data:
    point.co.z += point.co.x
lift.value = 0.25; lift.keyframe_insert('value', frame=1)
lift.value = 0.75; lift.keyframe_insert('value', frame=21)
for extension in ('usda', 'usdc', 'usdz'):
    bpy.ops.wm.usd_export(filepath=os.path.join(out, 'skinned.' + extension), export_animation=True, export_armatures=True, export_shapekeys=True)
report(os.path.join(out, 'skinned.json'))
