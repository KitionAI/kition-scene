import { useEffect, useRef, useState, type RefObject } from 'react'
import { ContextMenu, KitDocumentCanvas, MenuItem, MenuSeparator, useKitEditor, useKitTools, type KitCanvasHandle, type KitCanvasView, type RasterView } from '@kition/sdk/editor'
import type { SceneInfo } from './scene'
import { WeightPaintControls } from './WeightPaint'
import { SculptControls } from './Sculpt'
import { ParticleEditControls } from './Particles'
import { PresenceLayer, type PlacedPeer } from './Presence'
import { useGpuViewport } from './GlViewport'
import { RenderedViewport } from './RenderedViewport'
import { orbit, pan, type OrbitView, type Vec3 } from './gl/camera'
import { DRAG_ZOOM_SPEED, ORBIT_SPEED, SMOOTH_MS, WHEEL_SPEED, WheelSource, basisOf, between, cameraPlacement, leavingAxis, sameView, sceneExtent, zoomWithin } from './navigation'
import { viewNavigator } from './navigator'
import { NavGizmo, type NavDrag } from './NavGizmo'
import { useColorScheme } from './colorScheme'
import { TRANSFORM_MODES, TRANSFORM_TOOLS, toolGesture } from './gesture'
import { useToolOverlay } from './ToolOverlay'
import { modalTransform } from './modal'
import { ModalHeader, useModalActive } from './ModalUi'
import { AdjustLast } from './AdjustLast'
import { useTransformLayer } from './TransformGizmo'
import { TransformStrip } from './TransformControls'

type Pending = {
  orbit: [number, number]
  pan: [number, number]
  /** The distance to zoom to (the software viewport's zoom keeps within limits it can check). */
  distance: number | null
  /** An orbit away from an orthographic axis view turns perspective first (automatic perspective). */
  perspective: boolean
  /** The camera's frame as it is to be (looking through a camera). */
  cameraFrame: { zoom: number; offset: [number, number] } | null
}

const RESTING: () => Pending = () => ({ orbit: [0, 0], pan: [0, 0], distance: null, perspective: false, cameraFrame: null })

const toDegrees = (radians: number) => (radians * 180) / Math.PI

/** `view.set`'s parameters for an orbit view. */
const viewParams = (view: OrbitView) => ({ target: view.target, distance: view.distance, tilt: toDegrees(view.tilt), turn: toDegrees(view.turn), perspective: view.perspective })

/** Whether the person asked for less motion: a change of view then jumps. */
const reducedMotion = () => typeof window.matchMedia === 'function' && window.matchMedia('(prefers-reduced-motion: reduce)').matches

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
  t,
  readOnly = false,
}: {
  canvas: RefObject<KitCanvasHandle | null>
  info: SceneInfo | null
  onRender: () => void
  presence?: { available: boolean; peers: readonly PlacedPeer[]; movePointer: (at: [number, number] | null) => void }
  /** The editor's strings (Adjust Last Operation's, the navigation's). */
  t?: (key: string) => string
  /** The document cannot be changed: no gizmo, no transform settings. */
  readOnly?: boolean
}) {
  const editor = useKitEditor()
  const { doc } = editor.session
  const region = useRef<HTMLDivElement>(null)
  const viewMemory = useRef<RasterView | null>(null)
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null)
  const tools = useKitTools()
  const latest = useRef({ editor, doc, info, tool: tools?.active ?? null })
  latest.current = { editor, doc, info, tool: tools?.active ?? null }
  // The GPU viewport, when it draws: navigation then moves its view at once and hands the engine
  // the result when the gesture ends.
  const gpu = useGpuViewport(region, info?.shading !== 'rendered')
  const gpuRef = useRef(gpu)
  gpuRef.current = gpu
  // The transform under way: the engine runs it; the page gives it the pointer, the right button
  // and the wheel, draws its lines and shows its header.
  const modal = modalTransform
  modal.hooks = {
    run: async (command, params, refresh) => {
      const result = await latest.current.editor.session.run(command, params, null, { refresh })
      return typeof result === 'string' ? result : null
    },
    invoke: async (name, ...args) => {
      const open = latest.current.doc
      return open ? open.invoke(name, ...args) : null
    },
    gpu: () => gpuRef.current.active,
    adopt: (frame) => gpuRef.current.adopt(frame),
  }
  // The tools' overlays and the transform's lines over the picture.
  useToolOverlay(region, gpu, modal)
  // The 3D cursor, the transform tools' gizmos and the snapped point over the picture.
  const layer = useTransformLayer(region, gpu, info, modal, readOnly)
  const layerRef = useRef(layer)
  layerRef.current = layer
  // A change of the document may be the end of the transform (the tools confirm and cancel too).
  const { change } = editor.session
  useEffect(() => modal.sync(), [change, modal])
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
  const pending = useRef<Pending>(RESTING())
  /** The distance the software viewport's zoom last sent, until the engine's answer shows it. */
  const sentDistance = useRef<number | null>(null)
  const sentAt = useRef(0)
  /** The view the software viewport's last key or click sent, until the engine's answer shows it. */
  const sentView = useRef<{ view: OrbitView; at: number } | null>(null)
  if (sentView.current && info && (performance.now() - sentView.current.at > 1000 || sameView(info.orbit, sentView.current.view))) sentView.current = null
  if (sentDistance.current != null && info && (Math.abs(info.orbit.distance - sentDistance.current) < 1e-3 || performance.now() - sentAt.current > 1000)) sentDistance.current = null
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
    pending.current = RESTING()
    resize.current = null
    const steps: Array<[string, Record<string, unknown>]> = []
    if (size) steps.push(['view.resize', size])
    if (work.cameraFrame) steps.push(['view.set', { cameraZoom: work.cameraFrame.zoom, cameraOffset: work.cameraFrame.offset }])
    if (work.perspective) steps.push(['view.set', { perspective: true }])
    if (work.orbit[0] || work.orbit[1]) steps.push(['view.orbit', { turn: work.orbit[0], tilt: work.orbit[1] }])
    if (work.pan[0] || work.pan[1]) steps.push(['view.pan', { right: work.pan[0], up: work.pan[1] }])
    if (work.distance != null) {
      steps.push(['view.set', { distance: work.distance }])
      sentDistance.current = work.distance
      sentAt.current = performance.now()
    }
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
      if (resize.current || next.orbit[0] || next.orbit[1] || next.pan[0] || next.pan[1] || next.distance != null || next.perspective || next.cameraFrame) schedule()
    })()
  }
  const schedule = () => {
    if (!frame.current) frame.current = requestAnimationFrame(() => flush.current())
  }

  // Navigation: the view is moved here (the GPU viewport's own view while a gesture lasts, or
  // coalesced view commands for the software one), changes of view are smooth, and the zoom keeps
  // within limits.
  /** The change of view under way, which a new gesture or key finishes at once. */
  const animation = useRef<{ finish: () => void } | null>(null)
  /** Counts the navigation's actions: one waiting on the engine gives way to a newer one. */
  const navSerial = useRef(0)
  const wheelSource = useRef(new WheelSource())
  /** Looking through a camera: its frame as last asked for, ahead of the engine's answer. */
  const cameraFrame = useRef<{ zoom: number; offset: [number, number] } | null>(null)
  if (!info?.throughCamera) cameraFrame.current = null
  const run = (command: string, params: Record<string, unknown>) => latest.current.editor.session.run(command, params).catch((error) => latest.current.editor.report('View', error))
  const nav = useRef({
    finish: () => {
      navSerial.current += 1
      animation.current?.finish()
    },
    current: (): OrbitView | null => null,
    drag: (_kind: NavDrag, _dx: number, _dy: number) => {},
    zoomBy: (_factor: number) => {},
    framedBy: (_factor: number, _offset: [number, number]) => {},
    end: () => {},
  })
  nav.current.current = () => {
    const gl = gpuRef.current
    if (gl.active) return gl.view()
    const sent = sentView.current && performance.now() - sentView.current.at < 1000 ? sentView.current.view : null
    const known = sent ?? latest.current.info?.orbit
    return known ? { ...known, target: [...known.target] as Vec3 } : null
  }
  const extent = (view: OrbitView) => sceneExtent(latest.current.info?.objects.map((object) => object.location) ?? [], view.target)
  nav.current.zoomBy = (factor) => {
    const gl = gpuRef.current
    if (gl.active) {
      gl.move((view) => zoomWithin(view, factor, extent(view)))
      return
    }
    sentView.current = null
    const known = latest.current.info?.orbit
    if (!known) return
    // From the distance last asked for: the engine's answer may not be back yet.
    const now = { ...known, distance: pending.current.distance ?? (performance.now() - sentAt.current < 1000 ? sentDistance.current : null) ?? known.distance }
    pending.current.distance = zoomWithin(now, factor, extent(now)).distance
    schedule()
  }
  nav.current.framedBy = (factor, offset) => {
    const shown = latest.current.info
    if (!shown) return
    const base = cameraFrame.current ?? { zoom: shown.cameraZoom, offset: shown.cameraOffset }
    const reach = (value: number) => Math.min(4, Math.max(-4, value))
    const next = { zoom: Math.min(16, Math.max(0.1, base.zoom * factor)), offset: [reach(base.offset[0] + offset[0]), reach(base.offset[1] + offset[1])] as [number, number] }
    cameraFrame.current = next
    pending.current.cameraFrame = next
    schedule()
  }
  nav.current.drag = (kind, dx, dy) => {
    sentView.current = null
    const element = region.current
    const [width, height] = [Math.max(1, element?.clientWidth ?? 1), Math.max(1, element?.clientHeight ?? 1)]
    const camera = latest.current.info?.throughCamera === true
    const gl = gpuRef.current
    if (kind === 'zoom') {
      const factor = Math.exp(-dy * DRAG_ZOOM_SPEED)
      if (camera) nav.current.framedBy(1 / factor, [0, 0])
      else nav.current.zoomBy(factor)
      return
    }
    if (kind === 'pan') {
      if (camera) nav.current.framedBy(1, [(2 * dx) / width, (-2 * dy) / height])
      else if (gl.active) gl.move((view) => pan(view, dx / height, -dy / height))
      else {
        pending.current.pan[0] += dx / height
        pending.current.pan[1] -= dy / height
        schedule()
      }
      return
    }
    const degrees = Math.PI / 180
    if (gl.active) {
      gl.move((view) => orbit(leavingAxis(view), -dx * ORBIT_SPEED * degrees, -dy * ORBIT_SPEED * degrees))
      return
    }
    const known = latest.current.info?.orbit
    if (known && leavingAxis(known) !== known) pending.current.perspective = true
    pending.current.orbit[0] -= dx * ORBIT_SPEED
    pending.current.orbit[1] -= dy * ORBIT_SPEED
    schedule()
  }
  nav.current.end = () => {
    if (gpuRef.current.active) gpuRef.current.commit()
  }
  /** Moves from `from` to `to` over the smooth view's time on the GPU viewport, then tells the engine. */
  const animate = (from: OrbitView, to: OrbitView) => {
    const gl = gpuRef.current
    const started = performance.now()
    let raf = 0
    const finish = () => {
      cancelAnimationFrame(raf)
      if (animation.current?.finish === finish) animation.current = null
      gl.move(() => to)
      gl.commit()
    }
    const tick = (now: number) => {
      const t = (now - started) / SMOOTH_MS
      if (t >= 1) {
        finish()
        return
      }
      gl.move(() => between(from, to, t))
      raf = requestAnimationFrame(tick)
    }
    animation.current = { finish }
    raf = requestAnimationFrame(tick)
  }
  const show = (to: OrbitView, smooth: boolean) => {
    nav.current.finish()
    const gl = gpuRef.current
    const from = gl.active ? gl.view() : null
    if (!from) {
      sentView.current = { view: to, at: performance.now() }
      void run('view.set', viewParams(to))
      return
    }
    if (!smooth || reducedMotion() || latest.current.info?.throughCamera || sameView(from, to)) {
      gl.move(() => to)
      gl.commit()
      return
    }
    animate(from, to)
  }
  /** Runs a view command; one that moves the view (`smooth`) changes to the view it leaves smoothly. */
  const viewCommand = async (id: string, smooth: boolean) => {
    nav.current.finish()
    const serial = navSerial.current
    const gl = gpuRef.current
    const from = smooth && gl.active && !latest.current.info?.throughCamera && !reducedMotion() ? gl.view() : null
    // The picture holds the view it had until the change to the new one starts.
    if (from) gl.move(() => from)
    await run(id, {})
    if (!from) return
    try {
      const answer = await latest.current.editor.session.run('view.state', {}, null, { refresh: 'none' })
      // A key or gesture came meanwhile: it starts from the engine's view, and this one stops.
      if (navSerial.current !== serial) {
        gl.release(from)
        return
      }
      const state = (typeof answer === 'string' ? JSON.parse(answer) : answer) as { state?: { view?: { target: Vec3; distance: number; tilt: number; turn: number; perspective: boolean; lens: number; camera?: number } } } | null
      const view = state?.state?.view
      if (!view || view.camera != null) {
        gl.release(from)
        return
      }
      animate(from, { target: [...view.target] as Vec3, distance: view.distance, tilt: view.tilt, turn: view.turn, perspective: view.perspective, lens: view.lens })
    } catch {
      gl.release(from)
    }
  }
  const alignCamera = async () => {
    const shown = latest.current.info
    const say = (key: string) => latest.current.editor.report('View', new Error(t ? t(key) : key))
    if (!shown || shown.camera == null) {
      say('noActiveCamera')
      return
    }
    if (shown.objects.find((object) => object.id === shown.camera)?.parent != null) {
      say('cameraParented')
      return
    }
    // Looking through the camera already: the view is the camera's.
    const view = shown.throughCamera ? null : nav.current.current()
    if (!view) return
    nav.current.finish()
    if (gpuRef.current.active) gpuRef.current.commit()
    const { location, rotation } = cameraPlacement(view)
    await latest.current.editor.run('object.transform', { objects: [shown.camera], location, rotation })
    await run('view.camera', {})
  }
  viewNavigator.hooks = {
    // A key during a change of view steps from where that change ends.
    current: () => {
      nav.current.finish()
      return nav.current.current()
    },
    throughCamera: () => latest.current.info?.throughCamera === true,
    show,
    command: (id, smooth) => void viewCommand(id, smooth),
    cameraFrame: (factor, offset) => nav.current.framedBy(factor, offset),
    resetCameraFrame: () => {
      cameraFrame.current = { zoom: 1, offset: [0, 0] }
      pending.current.cameraFrame = cameraFrame.current
      schedule()
    },
    zoom: (factor) => {
      nav.current.finish()
      nav.current.zoomBy(factor)
      nav.current.end()
    },
    height: () => Math.max(1, region.current?.clientHeight ?? 1),
    alignCamera: () => void alignCamera(),
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
    /** Where an event is in the document (device pixels from the canvas's top left). */
    const documentPoint = (event: MouseEvent): [number, number] => {
      const box = element.getBoundingClientRect()
      const dpr = window.devicePixelRatio || 1
      return [(event.clientX - box.left) * dpr, (event.clientY - box.top) * dpr]
    }
    /** The pointer goes to the transform: one is under way, or a transform tool's press is held. */
    const transforming = () => modal.active || (toolGesture.held && toolGesture.transform)
    const wheel = (event: WheelEvent) => {
      if (!(event.target instanceof HTMLCanvasElement)) return
      event.preventDefault()
      event.stopPropagation()
      if (!readyRef.current) return
      const delta = event.deltaY * (event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? height() : 1)
      // The transform takes the wheel when it has a use for it (proportional editing's radius).
      if (transforming()) {
        // A bevel's segments, a loop cut's cuts.
        if (modal.answer.wheel && modal.wheel(delta)) return
        // Proportional editing's radius: set as a setting, which the transform under way (if the
        // press started one; nothing may be selected) follows at once.
        const proportional = latest.current.info?.proportional
        if (proportional) {
          const radius = Math.max(1e-3, proportional.radius * Math.exp(-Math.max(-400, Math.min(400, delta)) * WHEEL_SPEED))
          void latest.current.editor.session.run('mesh.proportional', { on: true, radius }).catch(() => undefined)
          return
        }
        if (modal.wheel(delta)) return
      }
      nav.current.finish()
      // A trackpad's two fingers orbit (Shift: pan); pinching (Ctrl) or Ctrl/Cmd with two fingers
      // zooms, and so does the wheel. Looking through a camera, zooming and panning move its frame.
      const unit = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? height() : 1
      const trackpad = wheelSource.current.fromTrackpad(event)
      if (event.ctrlKey || event.metaKey || !trackpad) {
        const factor = Math.exp(Math.max(-400, Math.min(400, delta)) * WHEEL_SPEED * (event.ctrlKey ? 4 : 1))
        if (latest.current.info?.throughCamera) {
          // The camera's frame goes to the engine at once; there is no orbit view to hand over.
          nav.current.framedBy(1 / factor, [0, 0])
          return
        }
        nav.current.zoomBy(factor)
      } else {
        // The fingers move the scene the way a drag would: against the scroll's direction.
        nav.current.drag(event.shiftKey ? 'pan' : 'orbit', -event.deltaX * unit, -event.deltaY * unit)
      }
      if (gpuRef.current.active) {
        window.clearTimeout(wheelCommit.current)
        wheelCommit.current = window.setTimeout(() => gpuRef.current.commit(), 180)
      }
    }
    let drag: { x: number; y: number; mode: NavDrag; id: number } | null = null
    // Shift+right button: the 3D cursor goes under the pointer, and follows it while held (one
    // placement in flight, the latest point waiting).
    let cursorDrag: number | null = null
    let cursorBusy = false
    let cursorNext: [number, number] | null = null
    const placeCursor = (at: [number, number]) => {
      cursorNext = at
      if (cursorBusy) return
      cursorBusy = true
      void (async () => {
        while (cursorNext) {
          const point = cursorNext
          cursorNext = null
          await latest.current.editor.session.run('cursor.set', { at: point }).catch(() => undefined)
        }
        cursorBusy = false
      })()
    }
    // A gizmo handle's drag: the page started the transform, and its release confirms it.
    let handleDrag: number | null = null
    // The right button cancelled a transform: the context menu it brings stays closed.
    let cancelledMenu = false
    // The press that confirmed an operation: its release is the page's too.
    let swallowed: number | null = null
    const down = (event: PointerEvent) => {
      if (unready(event) || !(event.target instanceof HTMLCanvasElement)) return
      modal.pointer = documentPoint(event)
      if (event.button === 2 && event.shiftKey && !transforming()) {
        event.preventDefault()
        event.stopPropagation()
        cancelledMenu = true
        cursorDrag = event.pointerId
        element.setPointerCapture(event.pointerId)
        placeCursor(documentPoint(event))
        return
      }
      // A gizmo handle (not its middle, which is the tool's own press) starts the transform kept
      // to the handle's axis or plane.
      const handle = event.button === 0 && !event.altKey && !transforming() ? layerRef.current.handleAt(documentPoint(event)) : null
      if (handle && latest.current.info) {
        event.preventDefault()
        event.stopPropagation()
        handleDrag = event.pointerId
        toolGesture.held = true
        toolGesture.transform = true
        element.setPointerCapture(event.pointerId)
        modal.begin(handle.kind, { at: documentPoint(event), handle: { axis: handle.axis, plane: handle.plane, orientation: latest.current.info.transform.orientation } })
        layerRef.current.hover(null)
        return
      }
      if (event.button === 2 && transforming()) {
        // The right button cancels the transform (the canvas never gives it to the tools).
        event.preventDefault()
        event.stopPropagation()
        cancelledMenu = true
        modal.cancel()
        return
      }
      // The left button confirms an operation a key started (a loop cut's first click goes on to
      // its slide): the page sends it, and the tools hear neither the press nor its release.
      if (event.button === 0 && !event.altKey && modal.operates) {
        event.preventDefault()
        event.stopPropagation()
        swallowed = event.pointerId
        modal.confirm()
        return
      }
      const orbit = event.button === 1 || (event.button === 0 && event.altKey)
      if (!orbit) {
        if (event.button === 0) {
          const { info: shown, tool } = latest.current
          toolGesture.held = true
          toolGesture.transform = TRANSFORM_TOOLS.includes(tool ?? '') && TRANSFORM_MODES.includes(shown?.mode ?? 'object')
        }
        return
      }
      event.preventDefault()
      event.stopPropagation()
      nav.current.finish()
      // Shift pans, Ctrl/Cmd zooms (up: out, down: in), otherwise it orbits.
      drag = { x: event.clientX, y: event.clientY, mode: event.shiftKey ? 'pan' : event.ctrlKey || event.metaKey ? 'zoom' : 'orbit', id: event.pointerId }
      element.setPointerCapture(event.pointerId)
      element.dataset.navigating = drag.mode
    }
    const move = (event: PointerEvent) => {
      if (!drag && unready(event)) return
      if (cursorDrag === event.pointerId) {
        event.stopPropagation()
        placeCursor(documentPoint(event))
        return
      }
      // The right button pressed while the left holds a handle's drag (a chord comes as a move):
      // it cancels the transform.
      if (event.button === 2 && transforming()) {
        event.preventDefault()
        event.stopPropagation()
        cancelledMenu = true
        modal.cancel()
        return
      }
      if (!drag) {
        if (!(event.target instanceof HTMLCanvasElement) && !element.contains(event.target as Node)) return
        const at = documentPoint(event)
        modal.pointer = at
        // During a transform the pointer drives it; the tools hear nothing of it.
        if (transforming()) {
          event.stopPropagation()
          modal.move(at, event.shiftKey, event.ctrlKey || event.metaKey)
        } else if (event.buttons === 0) {
          layerRef.current.hover(at)
        }
        return
      }
      if (event.pointerId !== drag.id) return
      event.stopPropagation()
      const dx = event.clientX - drag.x
      const dy = event.clientY - drag.y
      drag.x = event.clientX
      drag.y = event.clientY
      // Shift pressed during an orbit pans from there on.
      nav.current.drag(drag.mode === 'orbit' && event.shiftKey ? 'pan' : drag.mode, dx, dy)
    }
    const up = (event: PointerEvent) => {
      if (cursorDrag === event.pointerId) {
        cursorDrag = null
        event.stopPropagation()
        if (element.hasPointerCapture(event.pointerId)) element.releasePointerCapture(event.pointerId)
        return
      }
      if (handleDrag === event.pointerId) {
        handleDrag = null
        event.stopPropagation()
        toolGesture.held = false
        toolGesture.transform = false
        if (element.hasPointerCapture(event.pointerId)) element.releasePointerCapture(event.pointerId)
        // The release's place counts, then it confirms (a cancel meanwhile leaves nothing to confirm).
        modal.move(documentPoint(event), event.shiftKey, event.ctrlKey || event.metaKey)
        if (modal.active) modal.confirm()
        return
      }
      if (swallowed === event.pointerId) {
        swallowed = null
        event.stopPropagation()
        return
      }
      if (event.button === 0) {
        toolGesture.held = false
        toolGesture.transform = false
      }
      if (!drag && unready(event)) return
      if (!drag || event.pointerId !== drag.id) return
      event.stopPropagation()
      drag = null
      delete element.dataset.navigating
      nav.current.end()
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
      toolGesture.held = false
      toolGesture.transform = false
    }
    // Outside the viewport the pointer still drives the transform a key started, and a press there
    // confirms it.
    const outsideMove = (event: PointerEvent) => {
      if (!modal.active || element.contains(event.target as Node)) return
      const at = documentPoint(event)
      modal.pointer = at
      modal.move(at, event.shiftKey, event.ctrlKey || event.metaKey)
    }
    const outsideDown = (event: PointerEvent) => {
      if (!modal.active || element.contains(event.target as Node)) return
      if (event.button === 2) modal.cancel()
      else modal.confirm()
    }
    window.addEventListener('pointermove', outsideMove, true)
    window.addEventListener('pointerdown', outsideDown, true)
    const menu = (event: MouseEvent) => {
      if (!cancelledMenu && !modal.active) return
      cancelledMenu = false
      event.preventDefault()
      event.stopPropagation()
    }
    element.addEventListener('contextmenu', menu, true)
    window.addEventListener('pointerup', release)
    window.addEventListener('pointercancel', release)
    return () => {
      element.removeEventListener('wheel', wheel, true)
      element.removeEventListener('pointerdown', down, true)
      element.removeEventListener('pointermove', move, true)
      element.removeEventListener('pointerup', up, true)
      element.removeEventListener('pointercancel', up, true)
      element.removeEventListener('mousedown', mouseDown, true)
      element.removeEventListener('contextmenu', menu, true)
      window.removeEventListener('pointermove', outsideMove, true)
      window.removeEventListener('pointerdown', outsideDown, true)
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

  /**
   * Before the context menu opens, the engine's tools hear where the pointer is and the menu key:
   * an object there that is not selected becomes the selection, so the menu acts on it (a
   * selected one keeps the selection). Tools with their own use of the pointer (Knife, Loop Cut,
   * a brush) are left alone.
   */
  const selectForMenu = async (target: HTMLCanvasElement, at: { x: number; y: number }) => {
    const { doc: open } = latest.current
    if (!open || !readyRef.current || !['select', 'move', 'rotate', 'scale'].includes(tools?.active ?? '')) return
    const box = target.getBoundingClientRect()
    const dpr = window.devicePixelRatio || 1
    const point = { action: 'move', x: (at.x - box.left) * dpr, y: (at.y - box.top) * dpr, mods: {}, pressure: 1, zoom: 1 / dpr }
    const tool = latest.current.editor.session.tool
    await tool('pointer', [JSON.stringify(point)]).catch(() => null)
    await tool('toolKey', ['ContextMenu', '{}', 1 / dpr], true).catch(() => null)
  }
  const modalActive = useModalActive()
  const { phrase: p, enabled } = editor
  const command = (id: string, label: string) => <MenuItem key={id} label={p(label)} disabled={!enabled(id)} onSelect={() => void editor.run(id, {})} testId={`scene-context-${id}`} />
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
        // Shift+right button places the 3D cursor: no menu.
        if (event.shiftKey) return
        const at = { x: event.clientX, y: event.clientY }
        void selectForMenu(event.target, at).finally(() => setMenu(at))
      }}
    >
      <KitDocumentCanvas canvas={canvas} viewMemory={viewMemory} framed={false} onView={synchronize} testId="scene-canvas" />
      {info ? (
        <GizmoHost
          gpu={gpuRef}
          info={info}
          label={t ?? ((key) => key)}
          cameraEnabled={enabled('view.camera')}
          onDrag={(kind, phase, dx, dy) => {
            if (phase === 'begin') nav.current.finish()
            else if (phase === 'move') nav.current.drag(kind, dx, dy)
            else nav.current.end()
          }}
        />
      ) : null}
      {info && !readOnly && TRANSFORM_MODES.includes(info.mode) ? <TransformStrip info={info} /> : null}
      <ModalHeader />
      {t ? <AdjustLast operation={info?.operation ?? null} hidden={modalActive} t={t} /> : null}
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

/**
 * The navigation gizmo, drawn again whenever the view moves (each GPU frame while a gesture lasts,
 * each engine answer otherwise), without drawing the rest of the viewport again.
 */
function GizmoHost({
  gpu,
  info,
  label,
  cameraEnabled,
  onDrag,
}: {
  gpu: RefObject<ReturnType<typeof useGpuViewport>>
  info: SceneInfo
  label: (key: string) => string
  cameraEnabled: boolean
  onDrag: (kind: NavDrag, phase: 'begin' | 'move' | 'end', dx: number, dy: number) => void
}) {
  const basis = () => gpu.current?.cameraBasis() ?? basisOf(gpu.current?.view() ?? info.orbit)
  const [, redraw] = useState('')
  useEffect(() => {
    let shown = ''
    return gpu.current?.onDraw(() => {
      const key = basis().flat().map((value) => value.toFixed(3)).join()
      if (key !== shown) {
        shown = key
        redraw(key)
      }
    })
    // The basis reads the refs it is given.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [gpu])
  return (
    <NavGizmo
      basis={basis()}
      onAxis={(view) => viewNavigator.toggleAxis(view)}
      onDrag={onDrag}
      camera={info.throughCamera}
      orthographic={!info.perspective}
      cameraEnabled={cameraEnabled}
      onCamera={() => viewNavigator.hooks?.command('view.camera', false)}
      onPerspective={() => viewNavigator.hooks?.command('view.perspective', false)}
      label={label}
    />
  )
}
