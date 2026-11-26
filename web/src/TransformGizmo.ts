import { useEffect, useRef, type RefObject } from 'react'
import { useKitEditor, useKitTools } from '@kition/sdk/editor'
import type { GpuViewport } from './GlViewport'
import type { ModalTransform } from './modal'
import type { SceneInfo, Vec3 } from './scene'
import { basisOf } from './navigation'
import { TRANSFORM_MODES } from './gesture'

/**
 * The 3D cursor, the transform tools' gizmos and the point a move snapped to, drawn on a canvas of
 * the viewport's own over the picture (the GPU one or the software one), projected with the view
 * drawn now so they stay put while the view moves. The page tells which gizmo handle is under the
 * pointer (hover makes it bright) and starts the transform a handle's drag gives; the engine runs
 * the transform. The shapes are the editor's own drawing.
 *
 * Move: three arrows, three squares between pairs of axes (the plane across the third), a circle
 * in the middle. Rotate: three rings across the axes (their near halves) and a ring facing the
 * view. Scale: three axes ending in boxes, the three squares, and a ring in the middle. The middle
 * (and Rotate's view ring) is the tool's own press: a free move, a turn about the view, an even
 * scale, as without a gizmo; the axes and squares keep the transform to their axis or plane.
 */

/** The gizmo's size in CSS pixels (Blender's default gizmo size, 75). */
export const GIZMO_SIZE = 75
/** How near a handle, in CSS pixels, the pointer must be. */
const HIT = 7
/** The cursor ring's radius in CSS pixels. */
const CURSOR_RADIUS = 10
/** The axes' colours (the default theme's X, Y and Z) and the bright hover colour. */
const AXIS_COLORS = ['#ff3352', '#8bdc00', '#2890ff']
const AXIS_NAMES = ['x', 'y', 'z'] as const

export type GizmoKind = 'move' | 'rotate' | 'scale'
/** A handle that keeps the transform to an axis, or (`plane`) to the plane across it. */
export type GizmoHandle = { kind: GizmoKind; axis: 'x' | 'y' | 'z'; plane: boolean }

type Point = [number, number]
type Projector = (point: Vec3) => Point | null

/** What the layer drew, for tests: each handle's screen point (document pixels), the hovered one, the cursor's place. */
type GizmoProbe = { kind: GizmoKind | null; handles: Record<string, Point>; hovered: string | null; cursor: Point | null; snapped: Point | null; draws: number }

declare global {
  interface Window {
    sceneGizmo?: GizmoProbe
  }
}

const add = (a: Vec3, b: Vec3, scale = 1): Vec3 => [a[0] + b[0] * scale, a[1] + b[1] * scale, a[2] + b[2] * scale]
const dot = (a: Vec3, b: Vec3) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2]

export const handleName = (handle: GizmoHandle) => `${handle.kind}-${handle.plane ? 'plane-' : ''}${handle.axis}`

/** The distance from `p` to the segment `a`–`b`. */
function toSegment(p: Point, a: Point, b: Point) {
  const [dx, dy] = [b[0] - a[0], b[1] - a[1]]
  const length = dx * dx + dy * dy
  const t = length > 1e-9 ? Math.min(1, Math.max(0, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length)) : 0
  return Math.hypot(p[0] - (a[0] + dx * t), p[1] - (a[1] + dy * t))
}

/** Whether `p` is inside the polygon `points`. */
function inside(p: Point, points: Point[]) {
  let within = false
  for (let index = 0, last = points.length - 1; index < points.length; last = index, index += 1) {
    const [a, b] = [points[index], points[last]]
    if (a[1] > p[1] !== b[1] > p[1] && p[0] < ((b[0] - a[0]) * (p[1] - a[1])) / (b[1] - a[1] || 1e-9) + a[0]) within = !within
  }
  return within
}

/** One handle's shape on the screen: what to draw, how to tell the pointer is on it, its own point. */
type Shape = {
  handle: GizmoHandle
  /** 0 (hidden: the axis runs along the view) to 1. */
  alpha: number
  /** A polyline, a filled polygon, or both. */
  line: Point[]
  fill: Point[] | null
  /** The arrow's head or the box at the axis's end. */
  tip: Point[] | null
  at: Point
  color: string
}

/** The gizmo of `kind` about `centre` with `axes`, on the screen. */
function shapes(kind: GizmoKind, centre: Vec3, axes: [Vec3, Vec3, Vec3], project: Projector, back: Vec3, perUnit: number, dpr: number): { shapes: Shape[]; middle: Point | null; ring: number } {
  const middle = project(centre)
  const size = GIZMO_SIZE * dpr
  const reach = size / Math.max(perUnit, 1e-9)
  const out: Shape[] = []
  if (!middle) return { shapes: out, middle, ring: 0 }
  // How much of an axis shows across the view (an axis along the view fades out).
  const facing = axes.map((axis) => {
    const end = project(add(centre, axis, reach))
    return end ? Math.hypot(end[0] - middle[0], end[1] - middle[1]) / size : 0
  })
  const fade = (amount: number) => Math.min(1, Math.max(0, (amount - 0.15) / 0.2))
  if (kind === 'rotate') {
    axes.forEach((axis, index) => {
      const [u, v] = [axes[(index + 1) % 3], axes[(index + 2) % 3]]
      const line: Point[] = []
      const radius = reach * 0.8
      for (let step = 0; step <= 64; step += 1) {
        const angle = (step / 64) * Math.PI * 2
        const offset = add(add([0, 0, 0], u, Math.cos(angle) * radius), v, Math.sin(angle) * radius)
        // The near half only (a ring seen edge-on is a line, all of it near).
        if (dot(offset, back) < -radius * 0.02) {
          line.push([Number.NaN, Number.NaN])
          continue
        }
        const point = project(add(centre, offset))
        line.push(point ?? [Number.NaN, Number.NaN])
      }
      void axis
      // The handle's own point: the middle of the longest run of the ring that shows.
      let [best, start, run] = [0, -1, 0]
      for (let step = 0; step <= line.length; step += 1) {
        const shows = step < line.length && Number.isFinite(line[step][0])
        if (shows && start < 0) start = step
        if (!shows && start >= 0) {
          if (step - start > run) [best, run] = [start + Math.floor((step - start) / 2), step - start]
          start = -1
        }
      }
      out.push({ handle: { kind, axis: AXIS_NAMES[index], plane: false }, alpha: run > 2 ? 1 : 0, line, fill: null, tip: null, at: line[best] ?? middle, color: AXIS_COLORS[index] })
    })
    return { shapes: out, middle, ring: size }
  }
  axes.forEach((axis, index) => {
    const alpha = fade(facing[index])
    const from = project(add(centre, axis, reach * 0.22))
    const to = project(add(centre, axis, reach))
    if (!from || !to) return
    const [dx, dy] = [to[0] - from[0], to[1] - from[1]]
    const length = Math.hypot(dx, dy) || 1
    const [ux, uy] = [dx / length, dy / length]
    const tip: Point[] =
      kind === 'move'
        ? [[to[0] + ux * 12 * dpr, to[1] + uy * 12 * dpr], [to[0] - uy * 4.5 * dpr, to[1] + ux * 4.5 * dpr], [to[0] + uy * 4.5 * dpr, to[1] - ux * 4.5 * dpr]]
        : [[to[0] - 4 * dpr, to[1] - 4 * dpr], [to[0] + 4 * dpr, to[1] - 4 * dpr], [to[0] + 4 * dpr, to[1] + 4 * dpr], [to[0] - 4 * dpr, to[1] + 4 * dpr]]
    out.push({ handle: { kind, axis: AXIS_NAMES[index], plane: false }, alpha, line: [from, to], fill: null, tip, at: to, color: AXIS_COLORS[index] })
  })
  // The squares between two axes keep to the plane across the third.
  axes.forEach((_, index) => {
    const [u, v] = [axes[(index + 1) % 3], axes[(index + 2) % 3]]
    const corners = [[0.28, 0.28], [0.44, 0.28], [0.44, 0.44], [0.28, 0.44]].map(([a, b]) => project(add(add(centre, u, reach * a), v, reach * b)))
    if (corners.some((corner) => !corner)) return
    const fill = corners as Point[]
    const at: Point = [(fill[0][0] + fill[2][0]) / 2, (fill[0][1] + fill[2][1]) / 2]
    // Seen edge-on a square is a sliver: it fades out.
    const area = Math.abs((fill[1][0] - fill[0][0]) * (fill[3][1] - fill[0][1]) - (fill[1][1] - fill[0][1]) * (fill[3][0] - fill[0][0]))
    const alpha = fade(Math.sqrt(area) / (size * 0.16) * 0.35)
    out.push({ handle: { kind, axis: AXIS_NAMES[index], plane: true }, alpha, line: [...fill, fill[0]], fill, tip: null, at, color: AXIS_COLORS[index] })
  })
  return { shapes: out, middle, ring: kind === 'scale' ? 0.2 * size : 0.12 * size }
}

/** The handle under `at` of `drawn` (squares before axes before rings), or null. */
function hitOf(drawn: Shape[], at: Point, dpr: number): Shape | null {
  const reach = HIT * dpr
  const near = (shape: Shape) => {
    if (shape.alpha < 0.3) return Number.POSITIVE_INFINITY
    if (shape.fill && inside(at, shape.fill)) return 0
    if (shape.tip && inside(at, shape.tip)) return 0
    let best = Number.POSITIVE_INFINITY
    for (let index = 1; index < shape.line.length; index += 1) {
      const [a, b] = [shape.line[index - 1], shape.line[index]]
      if (!Number.isFinite(a[0]) || !Number.isFinite(b[0])) continue
      best = Math.min(best, toSegment(at, a, b))
    }
    return best
  }
  let found: Shape | null = null
  let distance = reach
  for (const shape of drawn) {
    const gap = near(shape) - (shape.handle.plane ? 1 : 0)
    if (gap <= distance) {
      found = shape
      distance = gap
    }
  }
  return found
}

/**
 * The layer: draws the cursor always, the active transform tool's gizmo when there is something
 * to transform and no transform is under way, and the snapped point during one. Answers
 * `handleAt(point)`, the handle under a document point (null over none), and `hover(point)`.
 */
export function useTransformLayer(region: RefObject<HTMLDivElement | null>, gpu: GpuViewport, info: SceneInfo | null, modal: ModalTransform, readOnly: boolean) {
  const tools = useKitTools()
  const tool = tools?.active ?? null
  const canvas = useRef<HTMLCanvasElement | null>(null)
  const latest = useRef({ gpu, info, tool, readOnly })
  latest.current = { gpu, info, tool, readOnly }
  const hovered = useRef<string | null>(null)
  const drawnShapes = useRef<Shape[]>([])
  const probe = useRef<GizmoProbe>({ kind: null, handles: {}, hovered: null, cursor: null, snapped: null, draws: 0 })

  /** The projection of the picture drawn now. */
  const projector = (): Projector | null => {
    const { gpu: viewport, info: shown } = latest.current
    if (viewport.active) return (point) => viewport.project(point)
    const matrix = shown?.viewMatrix
    if (!matrix || !shown) return null
    const [width, height] = [shown.viewWidth, shown.viewHeight]
    return ([x, y, z]) => {
      const clip = [0, 1, 2, 3].map((row) => matrix[row] * x + matrix[4 + row] * y + matrix[8 + row] * z + matrix[12 + row])
      if (!(clip[3] > 1e-9)) return null
      return [((clip[0] / clip[3]) * 0.5 + 0.5) * width, (0.5 - (clip[1] / clip[3]) * 0.5) * height]
    }
  }

  /** The gizmo the active tool shows now (none during a transform, or with nothing to transform). */
  const gizmoKind = (): GizmoKind | null => {
    const { info: shown, tool: active, readOnly: locked } = latest.current
    if (!shown || locked || modal.active || !shown.transform.gizmo || !TRANSFORM_MODES.includes(shown.mode)) return null
    return active === 'move' || active === 'rotate' || active === 'scale' ? active : null
  }

  const draw = useRef(() => {})
  draw.current = () => {
    const element = canvas.current
    if (!element) return
    const dpr = window.devicePixelRatio || 1
    const width = Math.max(1, Math.round(element.clientWidth * dpr))
    const height = Math.max(1, Math.round(element.clientHeight * dpr))
    if (element.width !== width || element.height !== height) {
      element.width = width
      element.height = height
    }
    const context = element.getContext('2d')
    if (!context) return
    context.setTransform(1, 0, 0, 1, 0, 0)
    context.clearRect(0, 0, width, height)
    probe.current.draws += 1
    probe.current.handles = {}
    probe.current.kind = null
    probe.current.cursor = null
    probe.current.snapped = null
    drawnShapes.current = []
    const { info: shown, gpu: viewport } = latest.current
    const project = projector()
    if (!shown || !project || shown.shading === 'rendered') return
    // The 3D cursor: a red and white ring with a cross, the same size at any distance.
    const cursor = project(shown.transform.cursor)
    if (cursor) {
      probe.current.cursor = cursor
      drawCursor(context, cursor, dpr)
    }
    // The point a move snapped to.
    const snapped = modal.answer.active ? modal.answer.snap?.point : null
    const snappedAt = snapped ? project(snapped) : null
    if (snappedAt) {
      probe.current.snapped = snappedAt
      context.strokeStyle = '#ffffff'
      context.lineWidth = 1.5 * dpr
      context.setLineDash([])
      context.beginPath()
      context.arc(snappedAt[0], snappedAt[1], 6 * dpr, 0, Math.PI * 2)
      context.stroke()
    }
    const kind = gizmoKind()
    const gizmo = shown.transform.gizmo
    if (!kind || !gizmo) return
    const view = viewport.active ? viewport.view() : null
    const basis = viewport.cameraBasis() ?? basisOf(view ?? shown.orbit)
    const [right, , back] = basis
    const middle = project(gizmo.centre)
    const side = project(add(gizmo.centre, right, 1))
    if (!middle || !side) return
    // Screen pixels per metre at the gizmo (it keeps one size on the screen).
    const perUnit = Math.hypot(side[0] - middle[0], side[1] - middle[1])
    const axes: [Vec3, Vec3, Vec3] = shown.transform.orientation === 'view' ? [basis[0], basis[1], basis[2]] : gizmo.axes
    const { shapes: drawn, ring } = shapes(kind, gizmo.centre, axes, project, back, perUnit, dpr)
    drawnShapes.current = drawn
    probe.current.kind = kind
    const hover = hovered.current
    for (const shape of drawn) {
      if (shape.alpha <= 0) continue
      const name = handleName(shape.handle)
      probe.current.handles[name] = shape.at
      const bright = hover === name
      context.globalAlpha = shape.alpha
      context.strokeStyle = bright ? '#ffffff' : shape.color
      context.fillStyle = bright ? 'rgba(255,255,255,0.9)' : shape.color
      context.lineWidth = (bright ? 3 : 2) * dpr
      context.setLineDash([])
      if (shape.fill) {
        context.globalAlpha = shape.alpha * (bright ? 0.9 : 0.45)
        context.beginPath()
        shape.fill.forEach(([x, y], index) => (index ? context.lineTo(x, y) : context.moveTo(x, y)))
        context.closePath()
        context.fill()
        context.globalAlpha = shape.alpha
      }
      context.beginPath()
      let pen = false
      for (const [x, y] of shape.line) {
        if (!Number.isFinite(x)) {
          pen = false
          continue
        }
        if (pen) context.lineTo(x, y)
        else context.moveTo(x, y)
        pen = true
      }
      context.stroke()
      if (shape.tip) {
        context.beginPath()
        shape.tip.forEach(([x, y], index) => (index ? context.lineTo(x, y) : context.moveTo(x, y)))
        context.closePath()
        context.fill()
      }
    }
    context.globalAlpha = 1
    // The middle (the tool's own press): a white ring; Rotate's ring facing the view.
    context.strokeStyle = 'rgba(255,255,255,0.85)'
    context.lineWidth = 1.5 * dpr
    context.beginPath()
    context.arc(middle[0], middle[1], ring, 0, Math.PI * 2)
    context.stroke()
  }

  // The layer, over kit-editor's canvases and the tools' overlay (it takes no pointer input).
  const { doc } = useKitEditor().session
  useEffect(() => {
    const element = region.current?.querySelector('.kui-canvas')
    if (!element) return undefined
    const layer = document.createElement('canvas')
    layer.className = 'scene-tool-overlay scene-transform-layer'
    layer.setAttribute('aria-hidden', 'true')
    layer.dataset.testid = 'scene-transform-layer'
    element.append(layer)
    canvas.current = layer
    if (import.meta.env.DEV) window.sceneGizmo = probe.current
    const resize = new ResizeObserver(() => draw.current())
    resize.observe(layer)
    draw.current()
    return () => {
      resize.disconnect()
      layer.remove()
      canvas.current = null
    }
  }, [region, doc])

  useEffect(() => draw.current(), [info, tool, readOnly])
  useEffect(() => modal.subscribe(() => draw.current()), [modal])
  const onDraw = gpu.onDraw
  useEffect(() => onDraw(() => draw.current()), [onDraw])

  return {
    /** The handle under document point `at`, or null. */
    handleAt: (at: Point): GizmoHandle | null => {
      if (!gizmoKind()) return null
      return hitOf(drawnShapes.current, at, window.devicePixelRatio || 1)?.handle ?? null
    },
    /** The pointer is over document point `at` (null: it left): the handle there is bright. */
    hover: (at: Point | null) => {
      const shape = at && gizmoKind() ? hitOf(drawnShapes.current, at, window.devicePixelRatio || 1) : null
      const name = shape ? handleName(shape.handle) : null
      if (name === hovered.current) return
      hovered.current = name
      probe.current.hovered = name
      draw.current()
    },
  }
}

/** The 3D cursor at `at`: a ring of red and white dashes, and a cross through it with a gap. */
function drawCursor(context: CanvasRenderingContext2D, at: Point, dpr: number) {
  const radius = CURSOR_RADIUS * dpr
  context.setLineDash([])
  context.lineWidth = 2 * dpr
  context.strokeStyle = '#ffffff'
  context.beginPath()
  context.arc(at[0], at[1], radius, 0, Math.PI * 2)
  context.stroke()
  context.strokeStyle = '#e8243c'
  for (let part = 0; part < 8; part += 2) {
    context.beginPath()
    context.arc(at[0], at[1], radius, (part / 8) * Math.PI * 2, ((part + 1) / 8) * Math.PI * 2)
    context.stroke()
  }
  context.strokeStyle = 'rgba(20,20,20,0.9)'
  context.lineWidth = 1 * dpr
  context.beginPath()
  for (const [dx, dy] of [[1, 0], [-1, 0], [0, 1], [0, -1]]) {
    context.moveTo(at[0] + dx * radius * 0.45, at[1] + dy * radius * 0.45)
    context.lineTo(at[0] + dx * radius * 1.6, at[1] + dy * radius * 1.6)
  }
  context.stroke()
}
