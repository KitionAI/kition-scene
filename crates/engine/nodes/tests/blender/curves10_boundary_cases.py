"""Generated-knot boundaries and low-count cyclic Catmull-Rom conversion."""
from curves10_cases import source

def groups():
    out = {}
    for kind in ["bezier", "catmullRom"]:
        for cyclic in [False, True]:
            for order in [2, 3, 4, 5, 6, 8, 16]:
                t = source(7, cyclic, kind, True)
                t.then("setSplineType", {"splineType": "nurbs"})
                t.then("setNurbsOrder", {"Order": order}, "Curves", "Curves")
                t.then("resampleCurve", {"mode": "evaluated"})
                out[f"{kind}-{'cyclic' if cyclic else 'open'}-order-{order}"] = t.finish()
    for cyclic in [False, True]:
        for target in ["bezier", "nurbs"]:
            t = source(2, cyclic, "catmullRom", True)
            t.then("setSplineType", {"splineType": target})
            t.then("resampleCurve", {"mode": "evaluated"})
            out[f"two-point-{'cyclic' if cyclic else 'open'}-{target}"] = t.finish()
    for kind in ["catmullRom", "nurbs"]:
        t = source(7, False, kind)
        normal = t.add("normal")
        store = t.then("storeNamedAttribute", {"Name": "controlNormal", "dataType": "float_vector", "domain": "point"}, "Geometry", "Geometry")
        t.link((normal, "Normal"), store, "Value")
        out[f"{kind}-control-normal"] = t.finish()
    return out
