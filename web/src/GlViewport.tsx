import { useEffect, useRef, useState, type RefObject } from 'react'
import { useKitEditor } from '@kition/sdk/editor'
import { GlRenderer } from './gl/renderer'
import { decodeGeometry, decodeWorld, type GlFrame } from './gl/protocol'
import { multiply, projection, viewMatrix, type OrbitView, type Vec3 } from './gl/camera'

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
  /**
   * Drops the gesture's view without handing it over (only while it is still `view`, when given):
   * the engine's own view shows again.
   */
  release: (view?: OrbitView) => void
  /**
   * Resolves once the GPU viewport has asked the engine for its frame after this call and drawn
   * it (at once when it draws nothing): what it shows then is the engine's state now.
   */
  settled: () => Promise<void>
  /** Draws a frame the engine answered some other way (a transform's step). */
  adopt: (frame: GlFrame) => void
  /** Where a world point is in the picture as drawn now, in document pixels (null behind the eye). */
  project: (point: Vec3) => [number, number] | null
  /** Calls `listener` after each frame drawn (the view may have moved); answers how to stop. */
  onDraw: (listener: () => void) => () => void
  /** The orbit view drawn now (a gesture's own while it lasts), or null before the first frame. */
  view: () => OrbitView | null
  /**
   * Looking through a camera (and no gesture moving the view): the camera's right, up and back
   * directions in the world, from the engine's view matrix. Null otherwise.
   */
  cameraBasis: () => [Vec3, Vec3, Vec3] | null
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
    /**
     * How many of the gestures' views the engine has taken: a frame asked for before the last of
     * them was taken shows an older view, and does not replace the gesture's own.
     */
    taken: number
    fetching: boolean
    again: boolean
    raf: number
    failed: boolean
    /** Callers of `settled` waiting for the next frame asked for, then for its drawing. */
    asking: Array<() => void>
    drawing: Array<() => void>
    /** Who wants to know when a frame was drawn. */
    drawn: Set<() => void>
  }>({ renderer: null, canvas: null, frame: null, local: null, holding: false, taken: 0, fetching: false, again: false, raf: 0, failed: !wanted(), asking: [], drawing: [], drawn: new Set() })
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

  /** Lets every `settled` caller in `waiting` go. */
  const release = (waiting: Array<() => void>) => {
    for (const done of waiting.splice(0)) done()
  }
  const draw = () => {
    const s = state.current
    s.raf = 0
    const { renderer, frame, canvas } = s
    const drawn = s.drawing.splice(0)
    try {
      paint(s, renderer, frame, canvas)
    } finally {
      release(drawn)
    }
  }
  const paint = (s: typeof state.current, renderer: GlRenderer | null, frame: GlFrame | null, canvas: HTMLCanvasElement | null) => {
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
    for (const listener of s.drawn) listener()
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
    release(s.asking)
    release(s.drawing)
    setOn(false)
    latest.current.editor.report('GPU viewport', error)
  }

  /** Asks the engine for the frame, and for the geometry whose key changed. */
  const refresh = async () => {
    const s = state.current
    const open = latest.current.doc
    if (!latest.current.enabled) {
      setOn(false)
      release(s.asking)
      return
    }
    if (!open || !s.renderer || s.failed) {
      release(s.asking)
      return
    }
    if (s.fetching) {
      s.again = true
      return
    }
    s.fetching = true
    const asked: Array<() => void> = []
    try {
      do {
        s.again = false
        // Who asked before this frame was asked for: let go once it is drawn.
        asked.push(...s.asking.splice(0))
        const taken = s.taken
        const frame = JSON.parse(await open.invoke<string>('glScene')) as GlFrame
        if (!(await show(open, frame, taken))) {
          release(asked)
          return
        }
        s.drawing.push(...asked.splice(0))
        schedule()
      } while (s.again)
    } catch (error) {
      release(asked)
      fail(error)
    } finally {
      s.fetching = false
    }
  }
  /**
   * Takes `frame` for drawing: fetches the geometry whose key changed and the world image, then
   * schedules the drawing. Answers false when the GPU viewport stopped meanwhile.
   */
  const show = async (open: NonNullable<typeof doc>, frame: GlFrame, taken = state.current.taken) => {
    const s = state.current
    const renderer = s.renderer
    if (!renderer || !latest.current.enabled) {
      setOn(false)
      return false
    }
    const held = renderer.heldKeys()
    for (const object of frame.objects) {
      if (held.get(object.id) === object.key) continue
      const bytes = await open.invoke<Uint8Array>('glMesh', object.id, object.tangents).catch(() => null)
      const geometry = bytes ? decodeGeometry(bytes) : null
      if (geometry) renderer.setGeometry(object.id, object.key, geometry)
    }
    const environment = frame.shading === 'material' ? frame.environment : null
    if (environment && renderer.environmentKey() !== environment.key) {
      const bytes = await open.invoke<Uint8Array>('glWorld').catch(() => null)
      renderer.setEnvironment(environment.key, bytes ? decodeWorld(bytes) : null)
    }
    return settle(frame, taken)
  }

  /** Whether drawing `frame` needs nothing more from the engine (no geometry, no world image). */
  const ready = (frame: GlFrame) => {
    const renderer = state.current.renderer
    if (!renderer) return false
    const held = renderer.heldKeys()
    const environment = frame.shading === 'material' ? frame.environment : null
    return frame.objects.every((object) => held.get(object.id) === object.key) && (!environment || renderer.environmentKey() === environment.key)
  }

  /**
   * Makes `frame` the one drawn (its geometry is in the renderer); asked for when the engine had
   * taken `taken` of the gestures' views.
   */
  const settle = (frame: GlFrame, taken = state.current.taken) => {
    const s = state.current
    const renderer = s.renderer
    // Geometry and environment requests can finish after rendered shading takes over.
    if (!renderer || !latest.current.enabled) {
      setOn(false)
      return false
    }
    renderer.retain(new Set(frame.objects.map((object) => object.id)))
    s.frame = frame
    // Shadows need float render targets; without them the software picture shows them.
    const missing = renderer.shadowsMissing(frame)
    probe.current.reason = missing ? 'shadows need float render targets' : frame.reason
    if (!s.holding && taken === s.taken) s.local = null
    setOn(frame.supported && !missing)
    return true
  }

  /**
   * A frame answered some other way (a transform's step): drawn at once, not at the next animation
   * frame, which would make the picture a frame later than the pointer; while a refresh is under
   * way that refresh asks again instead.
   */
  const adopt = (frame: GlFrame) => {
    const s = state.current
    const open = latest.current.doc
    if (!open || !s.renderer || s.failed) return
    if (s.fetching) {
      s.again = true
      return
    }
    // Only matrices moved (an object's transform): drawn now, before anything else runs.
    if (ready(frame)) {
      if (settle(frame)) {
        cancelAnimationFrame(s.raf)
        draw()
      }
      return
    }
    s.fetching = true
    void show(open, frame)
      .then((shown) => {
        if (!shown) return
        cancelAnimationFrame(s.raf)
        draw()
      })
      .catch((error) => fail(error))
      .finally(() => {
        s.fetching = false
        if (s.again) void refreshRef.current()
      })
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
      release(s.asking)
      release(s.drawing)
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
    settled: () =>
      new Promise<void>((done) => {
        state.current.asking.push(done)
        void refreshRef.current()
      }),
    adopt,
    project: (point) => {
      const s = state.current
      const { frame, canvas } = s
      if (!frame || !canvas) return null
      const dpr = window.devicePixelRatio || 1
      const width = Math.max(1, Math.round(canvas.clientWidth * dpr))
      const height = Math.max(1, Math.round(canvas.clientHeight * dpr))
      const view = s.local ?? frame.view
      const camera = s.local == null ? frame.camera : null
      const matrix = camera
        ? multiply(Float32Array.from(camera.projection), Float32Array.from(camera.viewMatrix))
        : multiply(projection(view, width, height), viewMatrix(view))
      const [x, y, z] = point
      const clip = [0, 1, 2, 3].map((row) => matrix[row] * x + matrix[4 + row] * y + matrix[8 + row] * z + matrix[12 + row])
      if (!(clip[3] > 1e-9)) return null
      return [((clip[0] / clip[3]) * 0.5 + 0.5) * width, (0.5 - (clip[1] / clip[3]) * 0.5) * height]
    },
    view: () => {
      const s = state.current
      const view = s.local ?? s.frame?.view
      return view ? { ...view, target: [...view.target] as Vec3 } : null
    },
    cameraBasis: () => {
      const s = state.current
      const camera = s.local == null ? s.frame?.camera : null
      if (!camera) return null
      const m = camera.viewMatrix
      const row = (index: number): Vec3 => [m[index] ?? 0, m[4 + index] ?? 0, m[8 + index] ?? 0]
      return [row(0), row(1), row(2)]
    },
    onDraw: (listener) => {
      state.current.drawn.add(listener)
      return () => {
        state.current.drawn.delete(listener)
      }
    },
    release: (view) => {
      const s = state.current
      if (view && s.local !== view) return
      s.holding = false
      s.local = null
      schedule()
    },
    commit: () => {
      const s = state.current
      const view = s.local
      // Nothing moved since the engine took the last view (a view still shown until the frame
      // after it arrives is not sent again: the engine may have moved on, into a camera view).
      if (!view || !s.holding) {
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
          state.current.taken += 1
          if (state.current.local === view) state.current.holding = false
          void refreshRef.current()
        })
    },
  }
}
