import { useEffect, useRef, type RefObject } from 'react'
import { useKitEditor, useKitTools } from '@kition/sdk/editor'
import type { GpuViewport } from './GlViewport'
import type { ModalTransform } from './modal'

/** For tests: how often the layer drew and whether it showed anything the last time. */
type OverlayProbe = { draws: number; drawn: boolean; modal: boolean }

declare global {
  interface Window {
    sceneToolOverlay?: OverlayProbe
  }
}

/** The axes' colours (the default theme's X, Y and Z). */
const AXIS_COLORS = ['#ff3352', '#8bdc00', '#2890ff']

/**
 * The engine tools' overlays (the box being dragged, the loop cut's loop, the knife's lines and
 * points) and the transform under way (its axis lines through the pivot, the dashed line from the
 * pivot to the pointer, proportional editing's circle) drawn on a canvas of the viewport's own
 * over the picture. kit-editor draws the tools' overlays on its software picture, which is hidden
 * while the GPU one shows (`opacity: 0`), so they were never seen there; the same drawing (its
 * `decorate`) goes onto this canvas, in document pixels one to one with device pixels, as the
 * viewport keeps its document. With the GPU viewport off the software picture shows the tools'
 * overlays itself, and this layer only the transform.
 *
 * The transform is drawn from world points projected with the view drawn now, so the lines stay
 * on the pivot while the view moves during the transform.
 */
export function useToolOverlay(region: RefObject<HTMLDivElement | null>, gpu: GpuViewport, modal: ModalTransform) {
  const tools = useKitTools()
  const { doc } = useKitEditor().session
  const decorate = tools?.decorate ?? null
  const active = gpu.active
  const canvas = useRef<HTMLCanvasElement | null>(null)
  const latest = useRef({ decorate, active, gpu })
  latest.current = { decorate, active, gpu }
  const probe = useRef<OverlayProbe>({ draws: 0, drawn: false, modal: false })

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
    const { decorate: paint, active: on, gpu: viewport } = latest.current
    probe.current.draws += 1
    probe.current.drawn = false
    probe.current.modal = false
    const answer = modal.answer
    if (answer.active) {
      // Document pixels (the engine's, one per device pixel) to the canvas.
      context.setTransform(1, 0, 0, 1, 0, 0)
      drawModal(context, answer, viewport, dpr, modal.pointer)
      probe.current.drawn = true
      probe.current.modal = true
      return
    }
    if (!on || !paint) return
    // The document is the viewport at one document pixel per device pixel: a CSS pixel is `dpr`
    // document pixels.
    const zoom = 1 / dpr
    context.setTransform(dpr, 0, 0, dpr, 0, 0)
    context.save()
    paint(context, (x, y) => ({ x: x * zoom, y: y * zoom }), { zoom, x: 0, y: 0 })
    context.restore()
    probe.current.drawn = true
  }

  // The layer, over kit-editor's canvases (it takes no pointer input).
  useEffect(() => {
    const element = region.current?.querySelector('.kui-canvas')
    if (!element) return undefined
    const layer = document.createElement('canvas')
    layer.className = 'scene-tool-overlay'
    layer.setAttribute('aria-hidden', 'true')
    layer.dataset.testid = 'scene-tool-overlay'
    element.append(layer)
    canvas.current = layer
    window.sceneToolOverlay = probe.current
    const resize = new ResizeObserver(() => draw.current())
    resize.observe(layer)
    draw.current()
    return () => {
      resize.disconnect()
      layer.remove()
      canvas.current = null
    }
    // The canvas region exists once the document is open.
  }, [region, doc])

  useEffect(() => draw.current(), [decorate, active])
  // The transform's answers, and every frame the GPU draws (the view moves under the lines).
  useEffect(() => modal.subscribe(() => draw.current()), [modal])
  const onDraw = gpu.onDraw
  useEffect(() => onDraw(() => {
    if (modal.answer.active) draw.current()
  }), [onDraw, modal])
}

/** The transform's lines, in document pixels. */
function drawModal(
  context: CanvasRenderingContext2D,
  answer: ModalTransform['answer'],
  gpu: GpuViewport,
  dpr: number,
  pointer: [number, number] | null,
) {
  const pivot = answer.pivot
  const projected = gpu.active && pivot ? gpu.project(pivot) : null
  const centre = projected ?? answer.pivotScreen ?? null
  if (!centre) return
  const reach = Math.max(context.canvas.width, context.canvas.height) * 2
  context.lineCap = 'round'
  for (const line of answer.lines ?? []) {
    let ends: [number, number, number, number] | null = line.screen
    if (gpu.active && pivot) {
      // The axis's direction on the screen as drawn now: a point a little way along it.
      const step = gpu.project([pivot[0] + line.direction[0] * 0.01, pivot[1] + line.direction[1] * 0.01, pivot[2] + line.direction[2] * 0.01])
      const [dx, dy] = step ? [step[0] - centre[0], step[1] - centre[1]] : [0, 0]
      const length = Math.hypot(dx, dy)
      ends = length > 1e-6 ? [centre[0] - (dx / length) * reach, centre[1] - (dy / length) * reach, centre[0] + (dx / length) * reach, centre[1] + (dy / length) * reach] : null
    }
    if (!ends) continue
    context.strokeStyle = AXIS_COLORS[line.axis] ?? '#ffffff'
    context.lineWidth = 1.5 * dpr
    context.setLineDash([])
    context.beginPath()
    context.moveTo(ends[0], ends[1])
    context.lineTo(ends[2], ends[3])
    context.stroke()
  }
  const at = pointer ?? answer.pointer ?? null
  if (answer.dashed && at) {
    // The dashed line from the pivot to the pointer (rotate, scale), light under dark.
    for (const [color, width] of [['rgba(255,255,255,0.85)', 3], ['rgba(20,20,20,0.95)', 1.5]] as const) {
      context.strokeStyle = color
      context.lineWidth = width * dpr
      context.setLineDash([6 * dpr, 4 * dpr])
      context.beginPath()
      context.moveTo(centre[0], centre[1])
      context.lineTo(at[0], at[1])
      context.stroke()
    }
    context.setLineDash([])
    // A double arrow at the pointer, across the line for a rotation, along it for a scale.
    const [dx, dy] = [at[0] - centre[0], at[1] - centre[1]]
    const length = Math.hypot(dx, dy) || 1
    const [ux, uy] = answer.kind === 'rotate' ? [-dy / length, dx / length] : [dx / length, dy / length]
    const size = 9 * dpr
    context.strokeStyle = 'rgba(20,20,20,0.95)'
    context.lineWidth = 1.5 * dpr
    context.beginPath()
    for (const sign of [1, -1]) {
      const [tx, ty] = [at[0] + ux * size * sign, at[1] + uy * size * sign]
      context.moveTo(at[0], at[1])
      context.lineTo(tx, ty)
      context.moveTo(tx, ty)
      context.lineTo(tx - (ux * sign + uy * 0.6) * size * 0.5, ty - (uy * sign - ux * 0.6) * size * 0.5)
      context.moveTo(tx, ty)
      context.lineTo(tx - (ux * sign - uy * 0.6) * size * 0.5, ty - (uy * sign + ux * 0.6) * size * 0.5)
    }
    context.stroke()
  }
  // Proportional editing's reach, round the pivot.
  if (answer.proportional != null && pivot) {
    let radius: number | null = answer.proportionalPixels ?? null
    if (gpu.active) {
      const edge = gpu.project([pivot[0] + answer.proportional, pivot[1], pivot[2]])
      const edgeY = gpu.project([pivot[0], pivot[1] + answer.proportional, pivot[2]])
      const edgeZ = gpu.project([pivot[0], pivot[1], pivot[2] + answer.proportional])
      const lengths = [edge, edgeY, edgeZ].filter((point): point is [number, number] => point != null).map((point) => Math.hypot(point[0] - centre[0], point[1] - centre[1]))
      radius = lengths.length ? Math.max(...lengths) : null
    }
    if (radius != null && Number.isFinite(radius) && radius < 1e5) {
      context.strokeStyle = 'rgba(60,60,66,0.8)'
      context.lineWidth = 1 * dpr
      context.setLineDash([4 * dpr, 4 * dpr])
      context.beginPath()
      context.arc(centre[0], centre[1], radius, 0, Math.PI * 2)
      context.stroke()
      context.setLineDash([])
    }
  }
  // The pivot.
  context.fillStyle = 'rgba(255,255,255,0.9)'
  context.strokeStyle = 'rgba(20,20,20,0.9)'
  context.lineWidth = 1 * dpr
  context.beginPath()
  context.arc(centre[0], centre[1], 2.5 * dpr, 0, Math.PI * 2)
  context.fill()
  context.stroke()
}
