# Read a layer with Blender's bundled OpenUSD public API, never its implementation.
# blender -b --python usdc_stage.py -- <layer> <report.json>
import json
import sys
from pxr import Usd

source, target = sys.argv[sys.argv.index('--') + 1:]
stage = Usd.Stage.Open(source)
if stage is None:
    raise RuntimeError('OpenUSD did not open the layer')


def value(item):
    if item is None or isinstance(item, (str, bool, int, float)):
        return item
    if hasattr(item, 'GetReal'):
        return [item.GetReal(), *item.GetImaginary()]
    if hasattr(item, 'path'):
        return item.path
    if isinstance(item, dict):
        return {str(key): value(entry) for key, entry in item.items()}
    return [value(entry) for entry in item]


out = {}
for prim in stage.Traverse():
    properties = {}
    for prop in prim.GetAuthoredProperties():
        if isinstance(prop, Usd.Relationship):
            properties[prop.GetName()] = {'targets': [str(path) for path in prop.GetTargets()]}
        else:
            properties[prop.GetName()] = {
                'type': str(prop.GetTypeName()),
                'uniform': str(prop.GetVariability()) == 'Sdf.VariabilityUniform',
                'custom': prop.IsCustom(),
                'default': value(prop.Get(Usd.TimeCode.Default())),
                'samples': [[time, value(prop.Get(time))] for time in prop.GetTimeSamples()],
                'connections': [str(path) for path in prop.GetConnections()],
            }
    spec = stage.GetRootLayer().GetPrimAtPath(prim.GetPath())
    schemas = spec.GetInfo('apiSchemas').GetAppliedItems() if spec.HasInfo('apiSchemas') else []
    out[str(prim.GetPath())] = {
        'type': prim.GetTypeName(), 'properties': properties,
        'customData': value(dict(spec.customData)), 'apiSchemas': list(schemas),
    }
with open(target, 'w') as file:
    json.dump(out, file, sort_keys=True, indent=2)
