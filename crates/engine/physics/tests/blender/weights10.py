"""Record cloth and soft-body EffectorWeights through the public Blender application API.
Run: blender -b --factory-startup --python weights10.py -- out.json.
No Blender implementation sources are used. Positions preserve observed solver differences;
weight transfer identities are checked independently of each solver's trajectory baseline.
"""
import bpy, json, sys

def observe(system,kind,weights,overrides=None):
 bpy.ops.wm.read_factory_settings(use_empty=True)
 s=bpy.context.scene;s.frame_end=12;s.gravity=(0,0,-9.81 if kind=='gravity' else 0)
 mesh=bpy.data.meshes.new('Probe');mesh.from_pydata([(-.5,-.5,0),(.5,-.5,0),(.5,.5,0),(-.5,.5,0)],[],[(0,1,2,3)])
 obj=bpy.data.objects.new('Probe',mesh);s.collection.objects.link(obj)
 m=obj.modifiers.new('Simulation','CLOTH' if system=='cloth' else 'SOFT_BODY');m.point_cache.frame_end=12
 if system=='cloth':m.settings.air_damping=0
 else:m.settings.use_goal=False;m.settings.use_edges=False;m.settings.friction=0
 for key,value in weights.items():setattr(m.settings.effector_weights,key,value)
 if kind!='gravity':
  bpy.ops.object.effector_add(type=kind.upper(),location=(3,0,-2))
  f=bpy.context.object.field;f.strength=2;f.flow=0
  if kind=='drag':f.linear_drag=1;f.quadratic_drag=0;s.gravity=(0,0,-9.81)
  for key,value in (overrides or {}).items():setattr(f,key,value)
 out=[]
 for frame in range(1,13):
  s.frame_set(frame);e=obj.evaluated_get(bpy.context.evaluated_depsgraph_get())
  if frame in [1,2,4,12]:out.append({'frame':frame,'mean':[sum(v.co[i] for v in e.data.vertices)/4 for i in range(3)]})
 return out



def defaults():
 bpy.ops.wm.read_factory_settings(use_empty=True)
 bpy.ops.mesh.primitive_plane_add()
 obj=bpy.context.object
 out={}
 for system,kind in [('cloth','CLOTH'),('soft','SOFT_BODY')]:
  modifier=obj.modifiers.new('Simulation',kind)
  weights=modifier.settings.effector_weights
  out[system]={key:{'default':getattr(weights,key),'min':weights.bl_rna.properties[key].hard_min,'max':weights.bl_rna.properties[key].hard_max} for key in ['all','gravity','force','wind','vortex','turbulence','drag']}
  obj.modifiers.remove(modifier)
 return out

cases=[]
for system in ['cloth','soft']:
 for kind in ['gravity','force','wind','vortex','turbulence','drag']:
  variants=[('default',{}),('all_zero',{'all':0}),('kind_zero',{kind:0}),('all_half',{'all':.5}),('kind_half',{kind:.5}),('quarter',{'all':.5,kind:.5}),('negative',{kind:-1}),('double_negative',{'all':-1,kind:-1}),('product',{'all':.3,kind:.7}),('product_control',{kind:.21})]
  for label,weights in variants:
   cases.append({'system':system,'kind':kind,'label':label,'weights':weights,'samples':observe(system,kind,weights)})
controls=[]
for system in ['cloth','soft']:
 for kind in ['force','wind','vortex','turbulence','drag']:
  variants=[('weight',{kind:.5},{'flow':1}),('strength',{}, {'strength':1,'flow':1}),('strength_flow',{}, {'strength':1,'flow':.5})]
  if kind=='drag':
   variants=[('weight',{kind:.5},{'linear_drag':1,'quadratic_drag':1}),('drag',{}, {'linear_drag':.5,'quadratic_drag':.5})]
  for label,weights,overrides in variants:
   controls.append({'system':system,'kind':kind,'label':label,'weights':weights,'overrides':overrides,'samples':observe(system,kind,weights,overrides)})
with open(sys.argv[sys.argv.index('--')+1], 'w') as handle:
 json.dump({'version':bpy.app.version_string,'defaults':defaults(),'cases':cases,'controls':controls},handle,indent=2,allow_nan=False)
 handle.write('\n')
