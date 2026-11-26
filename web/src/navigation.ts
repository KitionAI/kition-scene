/**
 * The viewport's navigation as a modeller expects it (Blender's keys and defaults): the keypad's
 * views and steps, the opposite view, automatic perspective, the smooth change of view, the
 * trackpad told from the wheel, and the limits of zoom. The page computes every view here and the
 * viewport hands the engine one `view.set` when a gesture or a transition ends.
 */
import { eye, orbit, pan, zoom, type OrbitView, type Vec3 } from './gl/camera'

/** How long a change of view takes (Blender's Smooth View). */
export const SMOOTH_MS = 200
/** Degrees a keypad orbit step turns (Blender's Rotation Angle). */
export const STEP_DEGREES = 15
/** A keypad zoom step, and a wheel notch's about the same. */
export const ZOOM_STEP = 1.2
/** Degrees the view turns per CSS pixel of an orbit drag (trackpad scrolls too). */
export const ORBIT_SPEED = 0.4
/** The wheel's zoom rate per pixel of scroll. */
export const WHEEL_SPEED = 0.0015
/** The zoom rate per pixel of a Ctrl+middle drag or of the navigation's zoom button. */
export const DRAG_ZOOM_SPEED = 0.005
/** Pixels a keypad pan step moves the picture by, across and up. */
const PAN_STEP: [number, number] = [32, 25]
/** The closest the eye comes to the point it turns around, in metres. */
export const NEAREST = 0.05
/** How many times the scene's size the farthest view is: the scene stays some pixels across. */
const FARTHEST = 40

const TAU = Math.PI * 2
const DEGREE = Math.PI / 180

export type AxisView = 'front' | 'back' | 'right' | 'left' | 'top' | 'bottom'

/** Each axis view's tilt and turn in degrees (the engine's `Axis`), and its opposite. */
export const AXIS_VIEWS: Record<AxisView, { tilt: number; turn: number; opposite: AxisView }> = {
  front: { tilt: 90, turn: 0, opposite: 'back' },
  back: { tilt: 90, turn: 180, opposite: 'front' },
  right: { tilt: 90, turn: 90, opposite: 'left' },
  left: { tilt: 90, turn: 270, opposite: 'right' },
  top: { tilt: 0, turn: 0, opposite: 'bottom' },
  bottom: { tilt: 180, turn: 0, opposite: 'top' },
}

/** `a − b` as the shortest turn, in −π..π. */
export function angleBetween(a: number, b: number) {
  let difference = (a - b) % TAU
  if (difference > Math.PI) difference -= TAU
  if (difference < -Math.PI) difference += TAU
  return difference
}

const same = (a: number, b: number) => Math.abs(angleBetween(a, b)) < 1e-3

/** The axis `view` looks along, or null for a view between them. Top and bottom at any turn. */
export function axisOf(view: Pick<OrbitView, 'tilt' | 'turn'>): AxisView | null {
  if (same(view.tilt, 0)) return 'top'
  if (same(view.tilt, Math.PI)) return 'bottom'
  if (!same(view.tilt, Math.PI / 2)) return null
  for (const name of ['front', 'back', 'right', 'left'] as const) if (same(view.turn, AXIS_VIEWS[name].turn * DEGREE)) return name
  return null
}

/** Looking along `axis`, orthographically (automatic perspective), around the same point. */
export function axisView(view: OrbitView, axis: AxisView): OrbitView {
  const { tilt, turn } = AXIS_VIEWS[axis]
  return { ...view, target: [...view.target] as Vec3, tilt: tilt * DEGREE, turn: turn * DEGREE, perspective: false }
}

/** The view from the other side: an axis view's opposite, or half a turn about the up axis. */
export function oppositeView(view: OrbitView): OrbitView {
  const axis = axisOf(view)
  if (axis) return { ...axisView(view, AXIS_VIEWS[axis].opposite), perspective: view.perspective }
  return { ...view, turn: (((view.turn + Math.PI) % TAU) + TAU) % TAU }
}

/**
 * Automatic perspective: a view that turns away from an orthographic axis view becomes a
 * perspective one (Blender's Auto Perspective, on by default). Any other view keeps its projection.
 */
export function leavingAxis(view: OrbitView): OrbitView {
  return !view.perspective && axisOf(view) ? { ...view, perspective: true } : view
}

export type Direction = 'left' | 'right' | 'up' | 'down'

/**
 * One keypad orbit step: 4 and 6 turn the eye about the up axis, 8 raises it towards the top and
 * 2 lowers it (the tilt keeps between straight down and straight up).
 */
export function orbitStep(view: OrbitView, direction: Direction, degrees = STEP_DEGREES): OrbitView {
  const step = degrees * DEGREE
  const from = leavingAxis(view)
  if (direction === 'left' || direction === 'right') return orbit(from, direction === 'left' ? -step : step, 0)
  return orbit(from, 0, direction === 'up' ? -step : step)
}

/** One keypad pan step (Ctrl+4 / 6 / 8 / 2): the view moves that way, `height` CSS pixels tall. */
export function panStep(view: OrbitView, direction: Direction, height: number): OrbitView {
  const h = Math.max(1, height)
  const [across, up] = PAN_STEP
  switch (direction) {
    case 'left':
      return pan(view, across / h, 0)
    case 'right':
      return pan(view, -across / h, 0)
    case 'up':
      return pan(view, 0, -up / h)
    default:
      return pan(view, 0, up / h)
  }
}

/**
 * How far the eye may be from the point it turns around: near enough to look at a detail, never
 * inside nothing; far enough to see the whole scene, never so far that it is gone. `extent` is the
 * scene's size about that point, in metres.
 */
export function zoomLimits(extent: number): [number, number] {
  return [NEAREST, FARTHEST * Math.max(5, Number.isFinite(extent) ? extent : 0)]
}

/** Zooms by `factor` (below 1: closer), kept within `zoomLimits(extent)`. */
export function zoomWithin(view: OrbitView, factor: number, extent: number): OrbitView {
  const [low, high] = zoomLimits(extent)
  const moved = zoom(view, factor)
  // A view already past a limit (an older view state) may still come back, never go further.
  const distance = factor < 1 ? Math.max(moved.distance, Math.min(low, view.distance)) : Math.min(moved.distance, Math.max(high, view.distance))
  return { ...moved, distance }
}

/** The scene's size about `target`: the farthest object origin from it, plus a little. */
export function sceneExtent(points: readonly Vec3[], target: Vec3) {
  let far = 0
  for (const point of points) far = Math.max(far, Math.hypot(point[0] - target[0], point[1] - target[1], point[2] - target[2]))
  return far + 2
}

const ease = (t: number) => t * t * (3 - 2 * t)

/** The view a fraction `t` (0..1) of the way from `a` to `b`, the shorter way round. */
export function between(a: OrbitView, b: OrbitView, t: number): OrbitView {
  const e = ease(Math.min(1, Math.max(0, t)))
  const mix = (x: number, y: number) => x + (y - x) * e
  return {
    target: [mix(a.target[0], b.target[0]), mix(a.target[1], b.target[1]), mix(a.target[2], b.target[2])],
    // Distances change by ratios: a zoom looks even.
    distance: Math.exp(mix(Math.log(Math.max(1e-6, a.distance)), Math.log(Math.max(1e-6, b.distance)))),
    tilt: mix(a.tilt, b.tilt),
    turn: (((a.turn + angleBetween(b.turn, a.turn) * e) % TAU) + TAU) % TAU,
    // Orthographic from the start when the view goes to an axis, perspective at once when it leaves one.
    perspective: b.perspective,
    lens: mix(a.lens, b.lens),
  }
}

/** Whether two views are the same to the eye (a transition between them would show nothing). */
export function sameView(a: OrbitView, b: OrbitView) {
  return (
    a.perspective === b.perspective &&
    Math.abs(a.distance - b.distance) < 1e-4 * Math.max(1, a.distance) &&
    same(a.tilt, b.tilt) &&
    same(a.turn, b.turn) &&
    a.target.every((value, index) => Math.abs(value - b.target[index]) < 1e-5)
  )
}

/** Where the eye is and how it is turned, as a camera object's location and rotation (degrees). */
export function cameraPlacement(view: OrbitView): { location: Vec3; rotation: Vec3 } {
  return { location: eye(view), rotation: [view.tilt / DEGREE, 0, view.turn / DEGREE] }
}

/** The view's right, up and back directions in the world (the columns of its rotation). */
export function basisOf(view: Pick<OrbitView, 'tilt' | 'turn'>): [Vec3, Vec3, Vec3] {
  const [sx, cx, sz, cz] = [Math.sin(view.tilt), Math.cos(view.tilt), Math.sin(view.turn), Math.cos(view.turn)]
  return [
    [cz, sz, 0],
    [-sz * cx, cz * cx, sx],
    [sz * sx, -cz * sx, cx],
  ]
}

/**
 * Whether a wheel event comes from a trackpad's two fingers rather than a mouse wheel: pixel
 * deltas with a sideways part or a fraction of a pixel are a trackpad's, and so is the rest of the
 * same gesture (a straight scroll on a trackpad can give whole pixels). Anything else is a wheel:
 * a plain vertical wheel in whole steps stays zoom.
 */
export class WheelSource {
  private lastTrackpad = Number.NEGATIVE_INFINITY

  fromTrackpad(event: Pick<WheelEvent, 'deltaMode' | 'deltaX' | 'deltaY'>, now = performance.now()) {
    if (event.deltaMode !== 0) return false
    const sideways = event.deltaX !== 0
    const fractional = !Number.isInteger(event.deltaX) || !Number.isInteger(event.deltaY)
    if (sideways || fractional) {
      this.lastTrackpad = now
      return true
    }
    if (now - this.lastTrackpad < 120) {
      this.lastTrackpad = now
      return true
    }
    return false
  }
}
