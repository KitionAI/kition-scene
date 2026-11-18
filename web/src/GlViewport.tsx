import { useEffect, useRef, useState, type RefObject } from 'react'
import { useKitEditor } from '@kition/kit-editor'
import { GlRenderer } from './gl/renderer'
import { decodeGeometry, decodeWorld, type GlFrame } from './gl/protocol'
import type { OrbitView } from './gl/camera'

/**
 * `localStorage['kition.scene.gl']`: `'off'` keeps the software viewport (for comparing, and tests);
 * `'on'` uses the GPU viewport even on a GPU emulated on the CPU.
 */
const SWITCH = 'kition.scene.gl'

function setting() {
  try {
    return window.localStorage.getItem(SWITCH)
  } catch {
    return null
  }
}

function wanted() {
  return setting() !== 'off'
}

/**
 * Whether WebGL runs on a CPU emulation of a GPU (SwiftShader, llvmpipe, Microsoft's basic
 * renderer): it would block the page for each frame, so the software viewport, which draws in the
 * engine's worker, serves better.
 */
function emulated(gl: WebGL2RenderingContext) {
  const info = gl.getExtension('WEBGL_debug_renderer_info')
  const name = String(info ? gl.getParameter(info.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER))
  return /swiftshader|llvmpipe|softpipe|basic render/i.test(name)
}

/** What the viewport's navigation does with the GPU viewport. */
export type GpuViewport = {
  /** The GPU draws the scene now (the software picture is hidden). */
  active: boolean
  /** Moves the view locally for the gesture under way; the engine is told at `commit`. */
  move: (step: (view: OrbitView) => OrbitView) => void
  /** Hands the engine the view the gesture ended with. */
  commit: () => void
}

/** For tests and measurement: frames drawn, and whether the GPU viewport is on. */
type Probe = {
  active: boolean
  frames: number
  reason: string | null
  lastFrameMs: number
  /** Draws `count` frames back to back, each waited for, turning the view a little each time;
   * answers milliseconds per frame (for the benchmark). */
  bench?: (count: number) => number
}

declare global {
  interface Window {
    sceneGl?: Probe
  }
}

/**
 * The GPU viewport (docs/design.md, section 4): a WebGL2 canvas under kit-editor's overlay
 * canvas, drawing what the engine describes (`glScene`, `glMesh`). The engine keeps the scene, the
 * picking, and the software picture (hidden while this one shows, and shown again whenever the
 * GPU cannot draw the state or WebGL2 fails). During navigation the view moves here, at once,
 * and the engine gets the result when the gesture ends.
 */
export function useGpuViewport(region: RefObject<HTMLDivElement | null>, enabled = true): GpuViewport {
  const editor = useKitEditor()
  const { doc } = editor.session
  const [active, setActive] = useState(false)
  const state = useRef<{
    renderer: GlRenderer | null
    canvas: HTMLCanvasElement | null
    frame: GlFrame | null
    local: OrbitView | null
    /** A gesture moved the view and the engine has not taken it yet. */
    holding: boolean
    fetching: boolean
    again: boolean
    raf: number
    failed: boolean
  }>({ renderer: null, canvas: null, frame: null, local: null, holding: false, fetching: false, again: false, raf: 0, failed: !wanted() })
  const probe = useRef<Probe>({ active: false, frames: 0, reason: null, lastFrameMs: 0 })
  const latest = useRef({ doc, editor, enabled })
  latest.current = { doc, editor, enabled }

  const setOn = (on: boolean) => {
    probe.current.active = on
    window.sceneGl = probe.current
    const element = region.current
    if (element) {
      if (on) element.dataset.gpu = 'on'
      else delete element.dataset.gpu
    }
    setActive(on)
  }

  const draw = () => {
    const s = state.current
    s.raf = 0
    const { renderer, frame, canvas } = s
    if (!latest.current.enabled || !renderer || !frame || !canvas || !frame.supported || s.failed) return
    const dpr = window.devicePixelRatio || 1
    const width = Math.max(1, Math.round(canvas.clientWidth * dpr))
    const height = Math.max(1, Math.round(canvas.clientHeight * dpr))
    const view = s.local ?? frame.view
    const started = performance.now()
    try {
      renderer.draw(frame, { view, width, height, camera: s.local == null })
    } catch (error) {
      fail(error)
      return
    }
    probe.current.frames += 1
    probe.current.lastFrameMs = performance.now() - started
  }
  const schedule = () => {
    if (!state.current.raf) state.current.raf = requestAnimationFrame(draw)
  }
  probe.current.bench = (count: number) => {
    const s = state.current
    const { renderer, frame, canvas } = s
    if (!renderer || !frame || !canvas) return Number.NaN
    const dpr = window.devicePixelRatio || 1
    const [width, height] = [Math.max(1, Math.round(canvas.clientWidth * dpr)), Math.max(1, Math.round(canvas.clientHeight * dpr))]
    const pixel = new Uint8Array(4)
    const started = performance.now()
    for (let index = 0; index < count; index += 1) {
      const view = { ...(s.local ?? frame.view), turn: (s.local ?? frame.view).turn + index * 0.01 }
      renderer.draw(frame, { view, width, height })
      // Reading a pixel waits for the frame to finish on the GPU.
      renderer.gl.readPixels(0, 0, 1, 1, renderer.gl.RGBA, renderer.gl.UNSIGNED_BYTE, pixel)
    }
    return (performance.now() - started) / count
  }

  const fail = (error: unknown) => {
    const s = state.current
    if (s.failed) return
    s.failed = true
    probe.current.reason = error instanceof Error ? error.message : String(error)
    s.renderer?.dispose()
    s.renderer = null
    s.canvas?.remove()
    s.canvas = null
    setOn(false)
    latest.current.editor.report('GPU viewport', error)
  }

  /** Asks the engine for the frame, and for the geometry whose key changed. */
  const refresh = async () => {
    const s = state.current
    const open = latest.current.doc
    if (!latest.current.enabled) {
      setOn(false)
      return
    }
    if (!open || !s.renderer || s.failed) return
    if (s.fetching) {
      s.again = true
      return
    }
    s.fetching = true
    try {
      do {
        s.again = false
        const frame = JSON.parse(await open.invoke<string>('glScene')) as GlFrame
        const renderer = s.renderer
        if (!renderer || !latest.current.enabled) {
          setOn(false)
          return
        }
        const held = renderer.heldKeys()
        for (const object of frame.objects) {
          if (held.get(object.id) === object.key) continue
          const bytes = await open.invoke<Uint8Array>('glMesh', object.id, object.tangents).catch(() => null)
          const geometry = bytes ? decodeGeometry(bytes) : null
          if (geometry) renderer.setGeometry(object.id, object.key, geometry)
        }
        renderer.retain(new Set(frame.objects.map((object) => object.id)))
        const environment = frame.shading === 'material' ? frame.environment : null
        if (environment && renderer.environmentKey() !== environment.key) {
          const bytes = await open.invoke<Uint8Array>('glWorld').catch(() => null)
          renderer.setEnvironment(environment.key, bytes ? decodeWorld(bytes) : null)
        }
        // Geometry and environment requests can finish after rendered shading takes over.
        if (!latest.current.enabled) {
          setOn(false)
          return
        }
        s.frame = frame
        // Shadows need float render targets; without them the software picture shows them.
        const missing = renderer.shadowsMissing(frame)
        probe.current.reason = missing ? 'shadows need float render targets' : frame.reason
        if (!s.holding) s.local = null
        setOn(frame.supported && !missing)
        schedule()
      } while (s.again)
    } catch (error) {
      fail(error)
    } finally {
      s.fetching = false
    }
  }
  const refreshRef = useRef(refresh)
  refreshRef.current = refresh

  // The canvas, under kit-editor's overlay canvas so the tools' overlays stay on top.
  useEffect(() => {
    const element = region.current
    const s = state.current
    if (!element || !doc || s.failed || s.renderer) return undefined
    const host = element.querySelector('.kui-canvas')
    if (!host) return undefined
    const canvas = document.createElement('canvas')
    canvas.className = 'scene-gl-canvas'
    canvas.setAttribute('aria-hidden', 'true')
    canvas.dataset.testid = 'scene-gl-canvas'
    try {
      s.renderer = new GlRenderer(canvas)
      if (emulated(s.renderer.gl) && setting() !== 'on') {
        s.renderer.dispose()
        s.renderer = null
        throw new Error('the GPU is emulated on the CPU')
      }
    } catch (error) {
      s.failed = true
      probe.current.reason = error instanceof Error ? error.message : String(error)
      window.sceneGl = probe.current
      return undefined
    }
    host.prepend(canvas)
    s.canvas = canvas
    s.renderer.onTextureNeeded = (hash, data) => {
      const open = latest.current.doc
      if (!open) return
      void open
        .invoke<Uint8Array>('imageBytes', hash)
        .then((bytes) => (bytes.byteLength ? createImageBitmap(new Blob([bytes.slice()]), { imageOrientation: 'flipY' }) : null))
        .catch(() => null)
        .then((image) => {
          const renderer = state.current.renderer
          if (!renderer) return
          if (data) renderer.setDataTexture(hash, image)
          else renderer.setTexture(hash, image)
          schedule()
        })
    }
    const lost = (event: Event) => {
      event.preventDefault()
      fail(new Error('the WebGL context was lost'))
    }
    canvas.addEventListener('webglcontextlost', lost)
    const resize = new ResizeObserver(() => schedule())
    resize.observe(canvas)
    void refreshRef.current()
    return () => {
      resize.disconnect()
      canvas.removeEventListener('webglcontextlost', lost)
      cancelAnimationFrame(s.raf)
      s.renderer?.dispose()
      s.renderer = null
      canvas.remove()
      s.canvas = null
    }
    // The canvas region exists once the document is open.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [doc])

  // Every change of the document (and every repaint mid-gesture) asks for the frame again.
  const { change, repaints } = editor.session
  useEffect(() => {
    void refreshRef.current()
  }, [change, enabled])
  useEffect(() => repaints.subscribe(() => void refreshRef.current()), [repaints])

  return {
    active,
    move: (step) => {
      const s = state.current
      const base = s.local ?? s.frame?.view
      if (!base) return
      s.holding = true
      s.local = step({ ...base, target: [...base.target] as OrbitView['target'] })
      schedule()
    },
    commit: () => {
      const s = state.current
      const view = s.local
      if (!view) {
        s.holding = false
        return
      }
      const toDegrees = (radians: number) => (radians * 180) / Math.PI
      const { editor: current } = latest.current
      void current.session
        .run('view.set', { target: view.target, distance: view.distance, tilt: toDegrees(view.tilt), turn: toDegrees(view.turn), perspective: view.perspective })
        .catch((error) => current.report('View', error))
        .finally(() => {
          // A newer gesture may have begun meanwhile: it keeps its own view.
          if (state.current.local === view) state.current.holding = false
          void refreshRef.current()
        })
    },
  }
}
