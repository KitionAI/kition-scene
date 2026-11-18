"""Additional spline consumers, constant fields, and conversion boundaries."""
import copy
from curves10_cases import source

def groups():
    out = {}
    for kind in ["catmullRom", "nurbs"]:
        for cyclic in [False, True]:
            side = "cyclic" if cyclic else "open"
            t = source(7, cyclic, kind, True)
            parameter = t.add("splineParameter")
            length = t.add("curveLength")
            t.link(t.last, length, "Curve")
            for name, socket in [("factor", "Factor"), ("length", "Length"), ("index", "Index")]:
                store = t.then("storeNamedAttribute", {"Name": name, "domain": "point", "dataType": "int" if name == "index" else "float"}, "Geometry", "Geometry")
                t.link((parameter, socket), store, "Value")
            store = t.then("storeNamedAttribute", {"Name": "total", "domain": "curve", "dataType": "float"}, "Geometry", "Geometry")
            t.link((length, "Length"), store, "Value")
            out[f"parameter-{kind}-{side}"] = t.finish()
            t = source(7, cyclic, kind, True)
            profile = t.add("curveCircle", {"Resolution": 4, "Radius": .1})
            sweep = t.then("curveToMesh", {"Fill Caps": True}, "Curve", "Mesh")
            t.link((profile, "Curve"), sweep, "Profile Curve")
            out[f"sweep-{kind}-{side}"] = t.finish()
            t = source(7, cyclic, kind, True)
            original = t.last
            points = t.add("meshLine", {"Count": 2, "Offset": [2, 0, .3]})
            inst = t.add("instanceOnPoints", {"Rotation": [.1, .2, .3]})
            t.link((points, "Mesh"), inst, "Points")
            t.link(original, inst, "Instance")
            t.last = (inst, "Instances")
            t.then("realizeInstances", {}, "Geometry", "Geometry")
            t.then("resampleCurve", {"mode": "evaluated"})
            out[f"realize-{kind}-{side}"] = t.finish()
    for node_type in ["poly", "catmullRom", "nurbs"]:
        t = source(7, False, node_type)
        t.then("setCurveRadius", {"Radius": .4})
        t.then("setCurveTilt", {"Tilt": .2})
        t.then("storeNamedAttribute", {"Name": "constant", "Value": .8, "dataType": "float", "domain": "point"}, "Geometry", "Geometry")
        t.then("setNurbsWeight", {"Weight": 0}, "Curves", "Curves")
        t.then("resampleCurve", {"mode": "evaluated"})
        out[f"constant-{node_type}"] = t.finish()
    for order in range(2, 7):
        t = source(7, False, "bezier", True)
        t.then("setSplineType", {"splineType": "nurbs"})
        t.then("setNurbsOrder", {"Order": order}, "Curves", "Curves")
        t.then("resampleCurve", {"mode": "evaluated"})
        out[f"bezier-knots-order-{order}"] = t.finish()
    return out
