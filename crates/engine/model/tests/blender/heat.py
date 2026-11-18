import bpy, bmesh, json, sys, math
from mathutils import Vector
def build(name, mesh_fn, bones):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    me = bpy.data.meshes.new(name); bm = bmesh.new(); mesh_fn(bm); bm.to_mesh(me); bm.free()
    ob = bpy.data.objects.new(name, me); bpy.context.collection.objects.link(ob)
    arm = bpy.data.armatures.new('A'); rig = bpy.data.objects.new('Rig', arm); bpy.context.collection.objects.link(rig)
    bpy.context.view_layer.objects.active = rig; bpy.ops.object.mode_set(mode='EDIT')
    made = {}
    for bname, h, t, parent in bones:
        b = arm.edit_bones.new(bname); b.head = h; b.tail = t; b.roll = 0
        if parent: b.parent = made[parent]; b.use_connect = (Vector(h) - made[parent].tail).length < 1e-6
        made[bname] = b
    bpy.ops.object.mode_set(mode='OBJECT')
    bpy.ops.object.select_all(action='DESELECT'); ob.select_set(True); rig.select_set(True); bpy.context.view_layer.objects.active = rig
    bpy.ops.object.parent_set(type='ARMATURE_AUTO')
    weights = {g.name: [0.0]*len(me.vertices) for g in ob.vertex_groups}
    for v in me.vertices:
        for g in v.groups: weights[ob.vertex_groups[g.group].name][v.index] = g.weight
    return {'positions': [list(v.co) for v in me.vertices], 'faces': [list(p.vertices) for p in me.polygons],
            'bones': [[n, list(h), list(t), p] for n, h, t, p in bones], 'weights': weights}
def cylinder(bm):
    rings, n = 9, 12
    verts = [[bm.verts.new((0.25*math.cos(2*math.pi*k/n), 0.25*math.sin(2*math.pi*k/n), -1 + 2*r/(rings-1))) for k in range(n)] for r in range(rings)]
    for r in range(rings-1):
        for k in range(n): bm.faces.new((verts[r][k], verts[r][(k+1)%n], verts[r+1][(k+1)%n], verts[r+1][k]))
    bm.faces.new(list(reversed(verts[0]))); bm.faces.new(verts[-1])
def tee(bm):
    bmesh.ops.create_cube(bm, size=1.0)
    bmesh.ops.scale(bm, vec=(0.6, 0.4, 2.0), verts=bm.verts); bmesh.ops.translate(bm, vec=(0, 0, 1.0), verts=bm.verts)
    bmesh.ops.subdivide_edges(bm, edges=bm.edges[:], cuts=3, use_grid_fill=True)
    bm.faces.ensure_lookup_table()
    for side in (1, -1):
        faces = [f for f in bm.faces if f.normal.x * side > 0.9 and f.calc_center_median().z > 1.45]
        for step in range(4):
            res = bmesh.ops.extrude_face_region(bm, geom=faces)
            new_faces = [g for g in res['geom'] if isinstance(g, bmesh.types.BMFace)]
            verts = [g for g in res['geom'] if isinstance(g, bmesh.types.BMVert)]
            bmesh.ops.translate(bm, vec=(0.3 * side, 0, 0), verts=verts)
            # The faces extruded from are inside now: they go.
            bmesh.ops.delete(bm, geom=faces, context='FACES_ONLY')
            faces = new_faces
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
out = {
 'cylinder': build('Cyl', cylinder, [('Lower', (0,0,-1), (0,0,0), None), ('Upper', (0,0,0), (0,0,1), 'Lower')]),
 'tee': build('Tee', tee, [('Spine', (0,0,0.2), (0,0,1.6), None), ('Arm.L', (0.1,0,1.72), (1.5,0,1.72), 'Spine'), ('Arm.R', (-0.1,0,1.78), (-1.5,0,1.78), 'Spine')]),
}
json.dump(out, open(sys.argv[sys.argv.index('--')+1], 'w'))
