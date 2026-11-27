/**
 * Selecting by hand on the page: what the engine's tools cannot hear (the right button, the wheel,
 * Alt with the left button, which orbits) and the select modes keys start. C paints the selection
 * with a circle until the right button, Escape or Enter ends it (the wheel sizes it; Shift or the
 * middle button takes out); Ctrl/Cmd with the right button draws a lasso (Shift takes out); B
 * makes the next drag a box whatever the tool, then gives the tool back. The engine finds what
 * they take (`select.circle`, `select.lasso`) with rays.
 *
 * While the GPU viewport draws, selection commands owe the software picture nothing (an area
 * outside it): it is drawn when something asks for it.
 */

/** An area of no document pixel: the canvas repaints nothing for it. */
export const NO_PICTURE = { x0: -2, y0: -2, x1: -1, y1: -1 }

/** Whether the GPU viewport draws the picture now (the GPU viewport keeps it). */
export const gpuShown = { on: false }

/** Blender's circle: 25 points across, in CSS pixels. */
const CIRCLE_RADIUS = 25

type Circle = { radius: number; at: [number, number] | null; painting: 'extend' | 'subtract' | null }
type Lasso = { points: Array<[number, number]>; subtract: boolean }

/** The page's selection gestures, which the tool overlay layer draws. */
class SelectionGestures {
  /** C's circle, while it lasts (document pixels). */
  circle: Circle | null = null
  /** The lasso being drawn (document pixels). */
  lasso: Lasso | null = null
  /** B: the tool to give back after the next box, when B picked the box. */
  boxOnce: { restore: string | null } | null = null
  private listeners = new Set<() => void>()

  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  emit() {
    for (const listener of [...this.listeners]) listener()
  }

  startCircle(dpr: number) {
    this.circle = { radius: this.circle?.radius ?? CIRCLE_RADIUS * dpr, at: this.circle?.at ?? null, painting: null }
    this.emit()
  }

  endCircle() {
    if (!this.circle) return
    this.circle = null
    this.emit()
  }

  /** The wheel sizes the circle (up: bigger). */
  resize(delta: number) {
    if (!this.circle) return
    this.circle = { ...this.circle, radius: Math.min(2000, Math.max(2, this.circle.radius * Math.exp(-delta * 0.002))) }
    this.emit()
  }
}

export const selectionGestures = new SelectionGestures()

declare global {
  interface Window {
    sceneSelection?: SelectionGestures
  }
}

if (import.meta.env.DEV && typeof window !== 'undefined') window.sceneSelection = selectionGestures

/** Draws the circle and the lasso (document pixels, one per device pixel) on `context`. */
export function drawSelectionGestures(context: CanvasRenderingContext2D) {
  const { circle, lasso } = selectionGestures
  if (!circle?.at && !lasso) return false
  context.save()
  context.lineWidth = 1.5
  context.setLineDash([5, 4])
  if (circle?.at) {
    context.strokeStyle = 'rgba(0, 0, 0, 0.6)'
    context.beginPath()
    context.arc(circle.at[0], circle.at[1], circle.radius, 0, Math.PI * 2)
    context.stroke()
    context.lineDashOffset = 4.5
    context.strokeStyle = 'rgba(255, 255, 255, 0.9)'
    context.stroke()
  }
  if (lasso && lasso.points.length > 1) {
    context.beginPath()
    lasso.points.forEach(([x, y], index) => (index === 0 ? context.moveTo(x, y) : context.lineTo(x, y)))
    context.closePath()
    context.strokeStyle = 'rgba(0, 0, 0, 0.6)'
    context.stroke()
    context.lineDashOffset = 4.5
    context.strokeStyle = 'rgba(255, 255, 255, 0.9)'
    context.stroke()
  }
  context.restore()
  return true
}
