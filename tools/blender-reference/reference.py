import bpy, json, math
out = {}
sc = bpy.context.scene
out["defaultScene"] = [{ "name": o.name, "type": o.type, "loc": list(o.location), "rot": list(o.rotation_euler), "scale": list(o.scale),
  **({"light": o.data.type, "energy": o.data.energy} if o.type=="LIGHT" else {}),
  **({"lens": o.data.lens, "sensor": o.data.sensor_width, "clip": [o.data.clip_start, o.data.clip_end]} if o.type=="CAMERA" else {}) } for o in sc.objects]
out["render"] = [sc.render.resolution_x, sc.render.resolution_y]
out["world"] = list(bpy.data.worlds[0].color)
bpy.ops.object.select_all(action='SELECT'); bpy.ops.object.delete()
prims = {}
def rec(name, op, **kw):
    op(**kw); o = bpy.context.active_object; m = o.data
    prims[name] = {"v": len(m.vertices), "e": len(m.edges), "f": len(m.polygons),
      "bbox": [list(map(lambda x: round(x,4), min((v.co for v in m.vertices), key=lambda c: c.x+c.y+c.z))),],
      "dims": [round(d,4) for d in o.dimensions], "firstVerts": [[round(c,5) for c in v.co] for v in list(m.vertices)[:4]],
      "faceSizes": sorted(set(len(p.vertices) for p in m.polygons))}
    bpy.ops.object.delete()
rec("plane", bpy.ops.mesh.primitive_plane_add)
rec("cube", bpy.ops.mesh.primitive_cube_add)
rec("circle", bpy.ops.mesh.primitive_circle_add)
rec("uvSphere", bpy.ops.mesh.primitive_uv_sphere_add)
rec("icoSphere", bpy.ops.mesh.primitive_ico_sphere_add)
rec("cylinder", bpy.ops.mesh.primitive_cylinder_add)
rec("cone", bpy.ops.mesh.primitive_cone_add)
rec("torus", bpy.ops.mesh.primitive_torus_add)
rec("grid", bpy.ops.mesh.primitive_grid_add)
out["primitives"] = prims
import inspect
props = {}
for n in ["primitive_plane_add","primitive_cube_add","primitive_circle_add","primitive_uv_sphere_add","primitive_ico_sphere_add","primitive_cylinder_add","primitive_cone_add","primitive_torus_add","primitive_grid_add"]:
    rna = getattr(bpy.ops.mesh, n).get_rna_type()
    props[n] = {p.identifier: (getattr(p, "default", None) if not getattr(p,"is_array",False) else list(p.default_array)) for p in rna.properties if p.identifier not in ("rna_type","location","rotation","scale","align","enter_editmode")}
out["primitiveDefaults"] = props
mods = {}
for t in ["MIRROR","ARRAY","SUBSURF","SOLIDIFY","BEVEL","DECIMATE","TRIANGULATE","WELD","SCREW","DISPLACE","SMOOTH","WIREFRAME","BOOLEAN","SIMPLE_DEFORM"]:
    bpy.ops.mesh.primitive_cube_add(); o = bpy.context.active_object
    m = o.modifiers.new("m", t)
    mods[t] = {p.identifier: (list(getattr(m,p.identifier)) if getattr(p,"is_array",False) else getattr(m,p.identifier)) for p in m.bl_rna.properties if p.type in ("BOOLEAN","INT","FLOAT","ENUM") and not p.is_readonly and p.identifier not in ("show_viewport","show_render","show_in_editmode","show_on_cage","show_expanded","is_active","use_pin_to_last","show_in_editmode")}
    bpy.ops.object.delete()
out["modifierDefaults"] = mods
mat = bpy.data.materials.new("M"); 
out["materialDefaults"] = {"diffuse": list(mat.diffuse_color), "metallic": mat.metallic, "roughness": mat.roughness}
def fix(o):
    if isinstance(o, float): return round(o, 5)
    if isinstance(o, dict): return {k: fix(v) for k,v in o.items()}
    if isinstance(o, (list,tuple)): return [fix(v) for v in o]
    if isinstance(o, set): return sorted(o)
    return o
import sys
out_path = sys.argv[sys.argv.index("--") + 1] if "--" in sys.argv else "blender-reference.json"
json.dump(fix(out), open(out_path, "w"), indent=1, default=str)
