# Geometry-node spline types

The node editor and `nodes.*` commands expose **Set Spline Type** (`setSplineType`),
**Set NURBS Order** (`setNurbsOrder`) and **Set NURBS Weight** (`setNurbsWeight`). These
operate on geometry inside a node graph. Native curve objects keep their existing model.

Set Spline Type has a curve-domain Selection and the `splineType` setting: `poly`,
`catmullRom`, `bezier` or `nurbs`. Poly and Catmull-Rom conversions retain the control
points. Poly-to-Bézier makes vector handles; Catmull-Rom-to-Bézier uses the uniform
cubic tangents. Bézier and Catmull-Rom become NURBS by expanding each control point
into its left handle, position and right handle, using cubic Bézier knots. Poly-to-NURBS
uses order 4, Endpoint knots when open and Normal knots when cyclic. Switching cyclic
later preserves the knot mode. Resolution remains controlled by Set Spline Resolution.

NURBS-to-Bézier is supported for cubic Bézier-knot curves created by these conversions,
with equal positive weights.
Other NURBS-to-Bézier conversions report `scene.nodesError`; first use Resample Curve to
make a Poly curve when a sampled conversion is intended. Custom knots and direct editing
of knot mode are not exposed. The public Blender app ignores Store Named Attribute writes
to `knots_mode`; this kit likewise does not offer that path as a knot editor.

Set NURBS Order uses Curves, Selection and Order sockets; Order defaults to 4 and is a
curve-domain field. Values below 1 become 1. The supported upper bound is 16. A curve
with too few control points for its order evaluates as its control polygon, as in the
recorded application. Set NURBS Weight uses Curves, Selection and Weight sockets; Weight
defaults to 1 and is a point-domain field. Negative weights become zero, all-zero weights
evaluate to zero positions, and weights must be finite and at most 100000000. Evaluation
uses local B-spline support, so work per evaluated point depends on bounded order rather
than the whole control-point count. The geometry evaluator's existing 4000000-element cap
counts control and evaluated points across all splines and instances. Sweeps check their
point-count product before allocation. Catmull-Rom and NURBS coordinates, handles, radius
and tilt must have magnitude at most 1000000000000. Their consumers reject non-finite
positions and attributes, including overflow in sweep products, with a node diagnostic.

The additional types pass through joins, transforms, point movement, reverse, resampling,
trimming, Curve to Points and Curve to Mesh. Float and vector point attributes, radius and
tilt use the same interpolation weights as each evaluated curve; curve-domain attributes
follow their source spline. Poly-to-Bézier uses one evaluated step per vector-handle segment;
trim locates segments from the evaluated control boundaries. Existing Bézier Segment nodes
create aligned handles; geometry nodes do not yet expose Set Handle Type. Constant fields retain their constant value even at a zero
rational denominator; varying fields and positions evaluate to zero there. NURBS
point-domain Spline Parameter and Normal follow the control polygon; Curve Length and
Curve to Points use the evaluated spline. Trimming a Catmull-Rom spline keeps its interior
control points and sampled endpoints; trimming NURBS makes an evaluated Poly spline. These are the
reference application's operations and need not preserve the original smooth curve's
shape exactly after trimming.

The implementation follows the standard uniform Catmull-Rom and rational Cox–de Boor
basis equations and public application recordings, never Blender source. The recorder and
its scene definitions are in `crates/engine/nodes/tests/blender/curves10.py` and the companion
case definitions. The immutable offline fixtures in `crates/engine/nodes/tests/fixtures/`
are `curves10-reference.json`, `curves10-consumers-reference.json` and
`curves10-boundary-reference.json` and `curves10-vector-reference.json` (230 cases,
Blender 5.2.3 LTS).
`KITION_BLENDER=<executable> cargo +1.95.0 test -p kition-scene-nodes --test curves10`
rechecks the fixture live. Tests compare coordinates and interpolated attributes within
0.00001, with exact counts and discrete settings.

Public references:

- [Set Spline Type manual](https://docs.blender.org/manual/en/4.1/modeling/geometry_nodes/curve/write/set_spline_type.html)
- [Curve properties manual](https://docs.blender.org/manual/en/latest/modeling/curves_new/properties.html)
- [Conversion Between Cubic Bézier Curves and Catmull-Rom Splines](https://arxiv.org/abs/2011.08232)

The existing node values JSON registers store the settings, so KDM schema 6 is unchanged.
The regular command transaction and file path provide undo and persistence.
