import { useEffect, useRef, type RefObject } from 'react'
import { useKitEditor, useKitTools } from '@kition/sdk/editor'

/** For tests: how often the layer drew and whether it showed anything the last time. */
type OverlayProbe = { draws: number; drawn: boolean }

declare global {
  interface Window {
    sceneToolOverlay?: OverlayProbe
  }
}

/**
 * The engine tools' overlays (the box being dragged, the rotate and scale readouts and their
 * dashed line, the loop cut's loop, the knife's lines and points, proportional editing's circle)
 * drawn on a canvas of the viewport's own over the GPU picture. kit-editor draws them on its
 * software picture, which is hidden while the GPU one shows (`opacity: 0`), so they were never
 * seen there. The same drawing (kit-editor's style, its `decorate`) goes onto this canvas, in
 * document pixels one to one with device pixels, as the viewport keeps its document; with the GPU
 * viewport off the layer stays empty and the software picture shows them itself.
 */
export function useToolOverlay(region: RefObject<HTMLDivElement | null>, active: boolean) {
  const tools = useKitTools()
  const { doc } = useKitEditor().session
  const decorate = tools?.decorate ?? null
  const canvas = useRef<HTMLCanvasElement | null>(null)
  const latest = useRef({ decorate, active })
  latest.current = { decorate, active }
  const probe = useRef<OverlayProbe>({ draws: 0, drawn: false })

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
    const { decorate: paint, active: on } = latest.current
    probe.current.draws += 1
    probe.current.drawn = false
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
}
