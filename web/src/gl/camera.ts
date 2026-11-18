/**
 * The viewport's orbit, as the engine keeps it (crates/render/raster/src/view.rs): the same
 * numbers, the same steps, the same matrices, so the editor can move the view on its own while a
 * gesture lasts and hand the engine the result at its end.
 */

export type Vec3 = [number, number, number]

export type OrbitView = {
  target: Vec3
  distance: number
  /** Radians: 0 looks straight down, π/2 level. */
  tilt: number
  /** Radians about the up axis. */
  turn: number
  perspective: boolean
  /** Millimetres against a 72 mm sensor. */
  lens: number
}

/** The sensor width the viewport lens is measured against. */
const SENSOR = 72
const TAU = Math.PI * 2

/** Column-major 4×4. */
export type Mat4 = Float32Array

/** The eye's rotation, rows of a 3×3 (X tilts first, then Z turns). */
function rotation(view: OrbitView): number[][] {
  const [sx, cx, sz, cz] = [Math.sin(view.tilt), Math.cos(view.tilt), Math.sin(view.turn), Math.cos(view.turn)]
  return [
    [cz, -sz * cx, sz * sx],
    [sz, cz * cx, -cz * sx],
    [0, sx, cx],
  ]
}

export function eye(view: OrbitView): Vec3 {
  const r = rotation(view)
  const d = view.distance
  return [view.target[0] + r[0][2] * d, view.target[1] + r[1][2] * d, view.target[2] + r[2][2] * d]
}

/** World to eye. */
export function viewMatrix(view: OrbitView): Mat4 {
  const r = rotation(view)
  const e = eye(view)
  const out = new Float32Array(16)
  for (let row = 0; row < 3; row += 1) {
    for (let col = 0; col < 3; col += 1) out[col * 4 + row] = r[col][row]
    out[12 + row] = -(r[0][row] * e[0] + r[1][row] * e[1] + r[2][row] * e[2])
  }
  out[15] = 1
  return out
}

function fovX(view: OrbitView) {
  return 2 * Math.atan(SENSOR / (2 * view.lens))
}

/** Eye to clip, in WebGL's depth convention (−1 near, 1 far), for a `width` × `height` picture. */
export function projection(view: OrbitView, width: number, height: number): Mat4 {
  const aspect = Math.max(1, width) / Math.max(1, height)
  const fx = fovX(view)
  const fy = aspect >= 1 ? 2 * Math.atan(Math.tan(fx / 2) / aspect) : fx
  const near = Math.max(view.distance * 0.001, 0.001)
  const far = Math.max(view.distance * 100, 1000)
  const out = new Float32Array(16)
  if (view.perspective) {
    const f = 1 / Math.tan(fy / 2)
    out[0] = f / aspect
    out[5] = f
    out[10] = (far + near) / (near - far)
    out[11] = -1
    out[14] = (2 * far * near) / (near - far)
  } else {
    // The engine's orthographic box reaches from −far to far.
    const halfHeight = view.distance * Math.tan(fy / 2)
    const halfWidth = halfHeight * aspect
    const [n, f] = [-far, far]
    out[0] = 1 / halfWidth
    out[5] = 1 / halfHeight
    out[10] = 2 / (n - f)
    out[14] = (f + n) / (n - f)
    out[15] = 1
  }
  return out
}

/** Turns the view by drag amounts in radians (the engine's `ViewState::orbit`). */
export function orbit(view: OrbitView, turn: number, tilt: number): OrbitView {
  return { ...view, turn: (((view.turn + turn) % TAU) + TAU) % TAU, tilt: Math.min(Math.PI, Math.max(0, view.tilt + tilt)) }
}

/** Moves the target across the screen by fractions of the view's height (`ViewState::pan`). */
export function pan(view: OrbitView, right: number, up: number): OrbitView {
  const r = rotation(view)
  const height = 2 * view.distance * Math.tan(fovX(view) / 2)
  const [dx, dy] = [-right * height, -up * height]
  return { ...view, target: [view.target[0] + r[0][0] * dx + r[0][1] * dy, view.target[1] + r[1][0] * dx + r[1][1] * dy, view.target[2] + r[2][0] * dx + r[2][1] * dy] }
}

/** Moves the eye towards (below 1) or away from the target (`ViewState::zoom`). */
export function zoom(view: OrbitView, factor: number): OrbitView {
  if (!Number.isFinite(factor) || factor <= 0) return view
  return { ...view, distance: Math.min(100_000, Math.max(0.01, view.distance * factor)) }
}

export function multiply(a: Mat4, b: Mat4): Mat4 {
  const out = new Float32Array(16)
  for (let col = 0; col < 4; col += 1) {
    for (let row = 0; row < 4; row += 1) {
      let sum = 0
      for (let k = 0; k < 4; k += 1) sum += a[k * 4 + row] * b[col * 4 + k]
      out[col * 4 + row] = sum
    }
  }
  return out
}

/** The inverse transpose of a matrix's upper 3×3 (for normals), as a column-major 3×3. */
export function normalMatrix(m: Mat4): Float32Array {
  const [a, b, c, d, e, f, g, h, i] = [m[0], m[4], m[8], m[1], m[5], m[9], m[2], m[6], m[10]]
  const det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g)
  const k = Math.abs(det) > 1e-20 ? 1 / det : 0
  // Inverse (rows), then transposed into column-major: the cofactor matrix over det.
  return new Float32Array([
    (e * i - f * h) * k, -(b * i - c * h) * k, (b * f - c * e) * k,
    -(d * i - f * g) * k, (a * i - c * g) * k, -(a * f - c * d) * k,
    (d * h - e * g) * k, -(a * h - b * g) * k, (a * e - b * d) * k,
  ])
}
