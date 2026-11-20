import { useEffect, useRef, useState, type RefObject } from 'react'
import { CameraIcon, FocusIcon, Grid2x2Icon } from '@kition/tessera-icons/react'
import { ContextMenu, IconButton, KitDocumentCanvas, MenuItem, MenuSeparator, useKitEditor, type KitCanvasHandle, type KitCanvasView, type RasterView } from '@kition/kit-editor'
import type { SceneInfo } from './scene'
import { WeightPaintControls } from './WeightPaint'
import { SculptControls } from './Sculpt'
import { ParticleEditControls } from './Particles'
import { PresenceLayer, type PlacedPeer } from './Presence'
import { useGpuViewport } from './GlViewport'
import { RenderedViewport } from './RenderedViewport'
import { orbit, pan, zoom } from './gl/camera'
import { useColorScheme } from './colorScheme'

/** Degrees the view turns per CSS pixel of an orbit drag. */
const ORBIT_SPEED = 0.4
/** The wheel's zoom rate per pixel of scroll. */
const WHEEL_SPEED = 0.0015

type Pending = { orbit: [number, number]; pan: [number, number]; zoom: number }

/**
 * The 3D viewport: the engine's picture shown one document pixel to one device pixel (the
 * document is the viewport, `view.resize` keeps its size the canvas's), with a modeller's
 * navigation: the wheel zooms, the middle button (or Alt with the left) orbits, Shift pans. Every
 * move is an engine view command, coalesced to one per frame with one in flight.
 *
 * A document opens at the size it was saved with. Until it is first the viewport's size, the
 * viewport is not ready (`data-ready="false"`): it shows no picture and takes no pointer input, as
 * the engine would pick in a picture framed for another size, one nobody saw.
 */
export function SceneViewport({
  canvas,
  info,
  onRender,
  presence,
}: {
  canvas: RefObject<KitCanvasHandle | null>
  info: SceneInfo | null
  onRender: () => void
  presence?: { available: boolean; peers: readonly PlacedPeer[]; movePointer: (at: [number, number] | null) => void }
}) {
  const editor = useKitEditor()
  const { doc } = editor.session
  const region = useRef<HTMLDivElement>(null)
  const viewMemory = useRef<RasterView | null>(null)
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null)
  const latest = useRef({ editor, doc, info })
  latest.current = { editor, doc, info }
  // A drag of the engine's tools under way (left button on the canvas, not navigating): the wheel
  // then resizes proportional editing's radius instead of zooming.
  const toolDrag = useRef(false)
  // The GPU viewport, when it draws: navigation then moves its view at once and hands the engine
  // the result when the gesture ends.
  const gpu = useGpuViewport(region, info?.shading !== 'rendered')
  const gpuRef = useRef(gpu)
  gpuRef.current = gpu
  const wheelCommit = useRef(0)
  // The document that was the viewport's size once: until then, the viewport is not ready.
  const [readyDoc, setReadyDoc] = useState<typeof doc>(null)
  // The document the engine paints in the host's color scheme, both pictures drawn since: until
  // then, the viewport is not ready either (an engine opens in the light scheme).
  const [schemedDoc, setSchemedDoc] = useState<typeof doc>(null)
  const ready = doc != null && readyDoc === doc && schemedDoc === doc
  const readyRef = useRef(ready)
  readyRef.current = ready
  /** The canvas's size in device pixels, as last measured. */
  const measured = useRef<{ width: number; height: number } | null>(null)
  const checkReady = useRef(() => {})
  checkReady.current = () => {
    const open = latest.current.doc
    const size = measured.current
    if (open && size && open.width === size.width && open.height === size.height) setReadyDoc(open)
  }

  // View commands, coalesced: one per animation frame, never two at once.
  const pending = useRef<Pending>({ orbit: [0, 0], pan: [0, 0], zoom: 1 })
  const resize = useRef<{ width: number; height: number } | null>(null)
  const busy = useRef(false)
  const frame = useRef(0)
  const flush = useRef<() => void>(() => undefined)
  flush.current = () => {
    frame.current = 0
    if (busy.current) return
    const { editor: current } = latest.current
    const work = pending.current
    const size = resize.current
    pending.current = { orbit: [0, 0], pan: [0, 0], zoom: 1 }
    resize.current = null
    const steps: Array<[string, Record<string, unknown>]> = []
    if (size) steps.push(['view.resize', size])
    if (work.orbit[0] || work.orbit[1]) steps.push(['view.orbit', { turn: work.orbit[0], tilt: work.orbit[1] }])
    if (work.pan[0] || work.pan[1]) steps.push(['view.pan', { right: work.pan[0], up: work.pan[1] }])
    if (work.zoom !== 1) steps.push(['view.zoom', { factor: work.zoom }])
    if (!steps.length) return
    busy.current = true
    void (async () => {
      for (const [command, params] of steps) {
        await current.session.run(command, params).catch((error) => {
          current.report('View', error)
          // A document the engine would not resize still shows, as it is.
          if (command === 'view.resize' && latest.current.doc) setReadyDoc(latest.current.doc)
        })
      }
      checkReady.current()
      busy.current = false
      const next = pending.current
      if (resize.current || next.orbit[0] || next.orbit[1] || next.pan[0] || next.pan[1] || next.zoom !== 1) schedule()
    })()
  }
  const schedule = () => {
    if (!frame.current) frame.current = requestAnimationFrame(() => flush.current())
  }

  /** Keeps the picture 1:1 at the top-left, and the document the canvas's device size. */
  const synchronize = (view: KitCanvasView) => {
    const handle = canvas.current
    if (!handle || view.width <= 0 || view.height <= 0) return
    const dpr = window.devicePixelRatio || 1
    const zoom = 1 / dpr
    if (Math.abs(view.zoom - zoom) > 1e-6) {
      handle.zoomTo(zoom)
      handle.panTo(0, 0)
      return
    }
    if (Math.abs(view.x) > 1e-6 || Math.abs(view.y) > 1e-6) handle.panTo(0, 0)
    const width = Math.max(1, Math.round(view.width * dpr))
    const height = Math.max(1, Math.round(view.height * dpr))
    measured.current = { width, height }
    checkReady.current()
    const open = latest.current.doc
    if (open && (open.width !== width || open.height !== height) && (resize.current?.width !== width || resize.current?.height !== height)) {
      resize.current = { width, height }
      schedule()
    }
  }

  // The document's size changed (the engine answered `view.resize`): the viewport may be ready.
  useEffect(() => checkReady.current(), [doc, info])

  // The host's color scheme: the engine paints the viewport's background, grid and frame shade in
  // it (the editor's colours, as a modeller's theme; never the scene's, never saved), at open and
  // whenever the host changes it.
  // The viewport is ready only once the software picture and the GPU's have been drawn after the
  // scheme reached the engine, so its first frame shown is in the host's scheme.
  const scheme = useColorScheme()
  useEffect(() => {
    if (!doc) return undefined
    let current = true
    void latest.current.editor.session
      .run('view.scheme', { scheme })
      .catch((error) => latest.current.editor.report('View', error))
      .then(() => (current ? Promise.all([canvas.current?.retryRender().catch(() => false), gpuRef.current.settled()]) : null))
      .then(() => {
        if (current) setSchemedDoc(doc)
      })
    return () => {
      current = false
    }
  }, [doc, scheme])

  // Navigation, ahead of the canvas's own handlers (capture phase): its wheel and middle button
  // would scroll the 2D picture. Before the viewport is ready, the canvas takes no pointer input.
  useEffect(() => {
    const element = region.current
    if (!element) return undefined
    const height = () => Math.max(1, element.clientHeight)
    const unready = (event: Event) => {
      if (readyRef.current || !(event.target instanceof HTMLCanvasElement)) return false
      event.preventDefault()
      event.stopPropagation()
      return true
    }
    const wheel = (event: WheelEvent) => {
      if (!(event.target instanceof HTMLCanvasElement)) return
      event.preventDefault()
      event.stopPropagation()
      if (!readyRef.current) return
      const delta = event.deltaY * (event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? height() : 1)
      const proportional = latest.current.info?.proportional
      if (toolDrag.current && proportional) {
        const radius = Math.max(1e-3, proportional.radius * Math.exp(-Math.max(-400, Math.min(400, delta)) * WHEEL_SPEED))
        void latest.current.editor.session.run('mesh.proportional', { on: true, radius }).catch(() => undefined)
        return
      }
      const factor = Math.exp(Math.max(-400, Math.min(400, delta)) * WHEEL_SPEED * (event.ctrlKey ? 4 : 1))
      if (gpuRef.current.active) {
        gpuRef.current.move((view) => zoom(view, factor))
        window.clearTimeout(wheelCommit.current)
        wheelCommit.current = window.setTimeout(() => gpuRef.current.commit(), 180)
        return
      }
      pending.current.zoom *= factor
      schedule()
    }
    let drag: { x: number; y: number; pan: boolean; id: number } | null = null
    const down = (event: PointerEvent) => {
      if (unready(event) || !(event.target instanceof HTMLCanvasElement)) return
      const orbit = event.button === 1 || (event.button === 0 && event.altKey)
      if (!orbit) {
        if (event.button === 0) toolDrag.current = true
        return
      }
      event.preventDefault()
      event.stopPropagation()
      drag = { x: event.clientX, y: event.clientY, pan: event.shiftKey, id: event.pointerId }
      element.setPointerCapture(event.pointerId)
      element.dataset.navigating = drag.pan ? 'pan' : 'orbit'
    }
    const move = (event: PointerEvent) => {
      if (!drag && unready(event)) return
      if (!drag || event.pointerId !== drag.id) return
      event.stopPropagation()
      const dx = event.clientX - drag.x
      const dy = event.clientY - drag.y
      drag.x = event.clientX
      drag.y = event.clientY
      if (gpuRef.current.active) {
        const panning = drag.pan || event.shiftKey
        const degrees = Math.PI / 180
        gpuRef.current.move((view) => (panning ? pan(view, dx / height(), -dy / height()) : orbit(view, -dx * ORBIT_SPEED * degrees, -dy * ORBIT_SPEED * degrees)))
        return
      }
      if (drag.pan || event.shiftKey) {
        pending.current.pan[0] += dx / height()
        pending.current.pan[1] -= dy / height()
      } else {
        pending.current.orbit[0] -= dx * ORBIT_SPEED
        pending.current.orbit[1] -= dy * ORBIT_SPEED
      }
      schedule()
    }
    const up = (event: PointerEvent) => {
      if (event.button === 0) toolDrag.current = false
      if (!drag && unready(event)) return
      if (!drag || event.pointerId !== drag.id) return
      event.stopPropagation()
      drag = null
      delete element.dataset.navigating
      if (gpuRef.current.active) gpuRef.current.commit()
      if (element.hasPointerCapture(event.pointerId)) element.releasePointerCapture(event.pointerId)
    }
    // The middle button's autoscroll.
    const mouseDown = (event: MouseEvent) => {
      if (event.button === 1) event.preventDefault()
    }
    element.addEventListener('wheel', wheel, { capture: true, passive: false })
    element.addEventListener('pointerdown', down, true)
    element.addEventListener('pointermove', move, true)
    element.addEventListener('pointerup', up, true)
    element.addEventListener('pointercancel', up, true)
    element.addEventListener('mousedown', mouseDown, true)
    // A tool drag can end outside the viewport.
    const release = () => {
      toolDrag.current = false
    }
    window.addEventListener('pointerup', release)
    window.addEventListener('pointercancel', release)
    return () => {
      element.removeEventListener('wheel', wheel, true)
      element.removeEventListener('pointerdown', down, true)
      element.removeEventListener('pointermove', move, true)
      element.removeEventListener('pointerup', up, true)
      element.removeEventListener('pointercancel', up, true)
      element.removeEventListener('mousedown', mouseDown, true)
      window.removeEventListener('pointerup', release)
      window.removeEventListener('pointercancel', release)
    }
  }, [])

  useEffect(() => () => cancelAnimationFrame(frame.current), [])

  // The pointer, shared as presence in device pixels from the canvas's top left.
  const sharePointer = useRef(presence?.movePointer)
  sharePointer.current = presence?.available ? presence.movePointer : undefined
  useEffect(() => {
    const element = region.current
    if (!element) return undefined
    const move = (event: PointerEvent) => {
      const canvasElement = element.querySelector('canvas')
      if (!sharePointer.current || !canvasElement) return
      const box = canvasElement.getBoundingClientRect()
      const dpr = window.devicePixelRatio || 1
      sharePointer.current([(event.clientX - box.left) * dpr, (event.clientY - box.top) * dpr])
    }
    const leave = () => sharePointer.current?.(null)
    element.addEventListener('pointermove', move, { passive: true })
    element.addEventListener('pointerleave', leave)
    return () => {
      element.removeEventListener('pointermove', move)
      element.removeEventListener('pointerleave', leave)
    }
  }, [])

  const { phrase: p, enabled, run } = editor
  const command = (id: string, label: string) => <MenuItem key={id} label={p(label)} disabled={!enabled(id)} onSelect={() => void run(id, {})} testId={`scene-context-${id}`} />
  const nav = (id: string, label: string, short: string) => (
    <button key={id} type="button" className="scene-nav-button" title={p(label)} aria-label={p(label)} onClick={() => void run(id, {})} data-testid={`scene-nav-${id}`}>
      {short}
    </button>
  )
  return (
    <div
      ref={region}
      className="kui-row scene-viewport"
      data-testid="scene-viewport"
      data-ready={ready}
      aria-busy={!ready || undefined}
      onContextMenu={(event) => {
        if (!(event.target instanceof HTMLCanvasElement)) return
        event.preventDefault()
        setMenu({ x: event.clientX, y: event.clientY })
      }}
    >
      <KitDocumentCanvas canvas={canvas} viewMemory={viewMemory} framed={false} onView={synchronize} testId="scene-canvas" />
      {info ? (
        <div className="scene-nav" role="toolbar" aria-label={p('Viewpoint')} data-testid="scene-nav">
          {nav('view.front', 'Front', p('Front'))}
          {nav('view.right', 'Right', p('Right'))}
          {nav('view.top', 'Top', p('Top'))}
          <span className="scene-nav-divider" />
          <IconButton icon={CameraIcon} label={p('Camera')} pressed={info.throughCamera} disabled={!enabled('view.camera')} onClick={() => void run('view.camera', {})} data-testid="scene-nav-view.camera" />
          <IconButton icon={Grid2x2Icon} label={p('Perspective/Orthographic')} pressed={!info.perspective} onClick={() => void run('view.perspective', {})} data-testid="scene-nav-view.perspective" />
          <IconButton icon={FocusIcon} label={p('Frame All')} onClick={() => void run('view.frameAll', {})} data-testid="scene-nav-view.frameAll" />
        </div>
      ) : null}
      <RenderedViewport active={info?.shading === 'rendered'} />
      {info ? <WeightPaintControls info={info} /> : null}
      {info ? <SculptControls info={info} /> : null}
      {info ? <ParticleEditControls info={info} /> : null}
      {presence ? <PresenceLayer peers={presence.peers} region={region} /> : null}
      <ContextMenu at={menu} onClose={() => setMenu(null)} label={p('Scene')} testId="scene-context-menu">
        {command('edit.undo', 'Undo')}
        {command('edit.redo', 'Redo')}
        <MenuSeparator />
        {command('edit.duplicate', 'Duplicate')}
        {command('edit.delete', 'Delete')}
        {command('mode.toggle', 'Toggle Edit Mode')}
        <MenuSeparator />
        {command('object.shadeSmooth', 'Shade Smooth')}
        {command('object.shadeSmoothByAngle', 'Shade Smooth by Angle')}
        {command('object.shadeFlat', 'Shade Flat')}
        <MenuSeparator />
        {command('view.frameSelected', 'Frame Selected')}
        {command('view.frameAll', 'Frame All')}
        <MenuSeparator />
        <MenuItem label={p('Render Image')} disabled={!enabled('render.image')} onSelect={onRender} />
      </ContextMenu>
    </div>
  )
}
