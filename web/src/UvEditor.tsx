import { useCallback, useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent, type WheelEvent as ReactWheelEvent } from 'react'
import { Checkbox, IconButton, useKitEditor, useReadOnly } from '@kition/kit-editor'
import { PinIcon, PinOffIcon, RotateCcwIcon, RotateCwIcon, ScissorsIcon, XIcon } from '@kition/tessera-icons/react'
import { useImageUrl } from './TextureSlot'
import type { SceneInfo } from './scene'

/** What `uv.layout` answers. */
type Layout = {
  source: string
  liveUnwrap: boolean
  method: string
  iterations: number
  object: number
  hasUvs: boolean
  faces: Array<{ face: number; vertices: number[]; uvs: Array<[number, number]>; selected: boolean }>
  selectedVertices: number[]
  seams: Array<[number, number]>
  pinned: number[]
  image: string | null
}

type Box = { x: number; y: number; size: number }
type Drag = { kind: 'move'; source: string; from: [number, number]; to: [number, number] } | { kind: 'box'; from: [number, number]; to: [number, number] } | { kind: 'pan'; from: [number, number]; box: Box }

/** UV space shown: a margin round the unit square. */
const HOME: Box = { x: -0.05, y: -0.05, size: 1.1 }
/** How close (in UV units per shown square) a click must be to a vertex to pick it. */
const PICK = 0.02

/**
 * The UV editor: the edited mesh's texture coordinates over its base colour image (or a
 * checker), drawn as SVG from the engine's `uv.layout`. It works on the mesh selection: a click
 * on a coordinate selects its vertex, a click inside a face selects its island, a drag on empty
 * space draws a box, and a drag on the selection moves it (`uv.translate`, one step).
 */
export function UvEditor({ info, onClose }: { info: SceneInfo; onClose: () => void }) {
  const { session, run, t, phrase } = useKitEditor()
  const readOnly = useReadOnly()
  const doc = session.doc
  const [layout, setLayout] = useState<Layout | null>(null)
  const [box, setBox] = useState<Box>(HOME)
  const [checker, setChecker] = useState(false)
  const [preview, setPreview] = useState<Layout | null>(null)
  const previewSerial = useRef(0)
  const previewPending = useRef<{ doc: NonNullable<typeof doc>; source: string; serial: number; offset: [number, number] } | null>(null)
  const previewBusy = useRef(false)
  const previewLastStart = useRef(0)
  const source = useRef<{ doc: typeof doc; token: string } | null>(null)
  const mounted = useRef(true)
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; previewSerial.current += 1; previewPending.current = null } }, [])
  const invalidatePreview = () => { previewSerial.current += 1; previewPending.current = null; setPreview(null) }
  // One worker request at a time, at most every 40 ms. A reply for an older pointer position,
  // ended gesture, or unmounted panel never replaces the current layout.
  const requestPreview = (offset: [number, number], token: string) => {
    if (!doc || !layout?.liveUnwrap) return
    previewPending.current = { doc, source: token, serial: ++previewSerial.current, offset }
    setPreview(null)
    if (previewBusy.current) return
    previewBusy.current = true
    const pump = async () => {
      const request = previewPending.current
      previewPending.current = null
      if (!request || !mounted.current) { previewBusy.current = false; return }
      previewLastStart.current = performance.now()
      try {
        const answer = JSON.parse(await request.doc.execute('uv.preview', { offset: request.offset, source: request.source })) as Layout
        // Recheck after the solve: a queued peer edit or selection can have changed the mesh
        // while its answer travelled back from the worker. Viewport sampling keeps this token.
        const current = JSON.parse(await request.doc.execute('uv.layout', { all: true })) as Layout
        if (mounted.current && previewSerial.current === request.serial && dragNow.current?.kind === 'move' && answer.source === current.source && source.current?.token === answer.source) setPreview(answer)
      } catch { /* The commit reports a rejected solve; a preview never changes the document. */ }
      if (previewPending.current && mounted.current) window.setTimeout(() => void pump(), 40)
      else previewBusy.current = false
    }
    window.setTimeout(() => void pump(), Math.max(0, 40 - (performance.now() - previewLastStart.current)))
  }
  const [drag, setDragState] = useState<Drag | null>(null)
  // The pointer handlers read the drag from here: a quick click can deliver its pointerup before
  // React renders the pointerdown's state, and the handler then would see no drag at all.
  const dragNow = useRef<Drag | null>(null)
  const setDrag = useCallback((next: Drag | null) => {
    dragNow.current = next
    setDragState(next)
  }, [])
  const svg = useRef<SVGSVGElement>(null)

  // The layout again after every change (selection included).
  useEffect(() => {
    // A selection, remote edit, or document replacement invalidates the source of this drag.
    if (source.current?.doc !== doc) {
      previewSerial.current += 1
      previewPending.current = null
      setPreview(null)
      setDrag(null)
    }
    if (!doc || info.mode !== 'edit') {
      setLayout(null)
      return
    }
    let live = true
    void doc
      .execute('uv.layout', { all: true })
      .then((answer) => {
        if (live) {
          const next = JSON.parse(answer) as Layout
          if (source.current?.token !== next.source || source.current?.doc !== doc) {
            previewSerial.current += 1
            previewPending.current = null
            setPreview(null)
            setDrag(null)
          }
          source.current = { doc, token: next.source }
          setLayout(next)
        }
      })
      .catch(() => {
        if (live) setLayout(null)
      })
    return () => {
      live = false
    }
  }, [doc, info])

  const image = useImageUrl(checker ? null : (layout?.image ?? null))
  const selected = useMemo(() => new Set(layout?.selectedVertices ?? []), [layout])
  const seams = useMemo(() => new Set((layout?.seams ?? []).map(([a, b]) => `${a}:${b}`)), [layout])
  const pinned = useMemo(() => new Set(layout?.pinned ?? []), [layout])

  /** The UV point under a pointer event. */
  const toUv = useCallback(
    (event: { clientX: number; clientY: number }): [number, number] => {
      const rect = svg.current?.getBoundingClientRect()
      if (!rect || rect.width === 0) return [0, 0]
      const side = Math.min(rect.width, rect.height)
      const left = rect.left + (rect.width - side) / 2
      const top = rect.top + (rect.height - side) / 2
      const u = box.x + ((event.clientX - left) / side) * box.size
      const v = box.y + (1 - (event.clientY - top) / side) * box.size
      return [u, v]
    },
    [box],
  )

  /** The vertex whose coordinate is nearest `at`, within the pick distance. */
  const vertexAt = (at: [number, number]) => {
    let best: { vertex: number; distance: number } | null = null
    for (const face of layout?.faces ?? []) {
      face.uvs.forEach(([u, v], corner) => {
        const distance = Math.hypot(u - at[0], v - at[1])
        if (distance < PICK * box.size && (!best || distance < best.distance)) best = { vertex: face.vertices[corner], distance }
      })
    }
    return (best as { vertex: number } | null)?.vertex ?? null
  }

  /** The face containing `at`. */
  const faceAt = (at: [number, number]) =>
    layout?.faces.find((face) => {
      let inside = false
      for (let i = 0, j = face.uvs.length - 1; i < face.uvs.length; j = i++) {
        const [ui, vi] = face.uvs[i]
        const [uj, vj] = face.uvs[j]
        if (vi > at[1] !== vj > at[1] && at[0] < ((uj - ui) * (at[1] - vi)) / (vj - vi) + ui) inside = !inside
      }
      return inside
    }) ?? null

  /** The faces of the island `start` is on: joined through corners sharing a vertex and a coordinate. */
  const island = (start: number) => {
    const faces = layout?.faces ?? []
    const key = (vertex: number, [u, v]: [number, number]) => `${vertex}@${u.toFixed(5)},${v.toFixed(5)}`
    const byCorner = new Map<string, number[]>()
    faces.forEach((face, index) => face.vertices.forEach((vertex, corner) => byCorner.set(key(vertex, face.uvs[corner]), [...(byCorner.get(key(vertex, face.uvs[corner])) ?? []), index])))
    const seen = new Set<number>([start])
    const queue = [start]
    while (queue.length) {
      const index = queue.pop()!
      const face = faces[index]
      face.vertices.forEach((vertex, corner) => {
        for (const other of byCorner.get(key(vertex, face.uvs[corner])) ?? []) {
          if (!seen.has(other)) {
            seen.add(other)
            queue.push(other)
          }
        }
      })
    }
    return [...seen].map((index) => faces[index].face)
  }

  const down = (event: ReactPointerEvent<SVGSVGElement>) => {
    if (!layout) return
    invalidatePreview()
    event.currentTarget.setPointerCapture(event.pointerId)
    const at = toUv(event)
    if (event.button === 1 || event.altKey) {
      setDrag({ kind: 'pan', from: [event.clientX, event.clientY], box })
      return
    }
    const vertex = vertexAt(at)
    const onSelection = (vertex != null && selected.has(vertex)) || (() => {
      const face = faceAt(at)
      return face != null && face.vertices.every((other) => selected.has(other))
    })()
    if (onSelection && !event.shiftKey && !readOnly) {
      setDrag({ kind: 'move', source: layout.source, from: at, to: at })
      return
    }
    setDrag({ kind: 'box', from: at, to: at })
  }

  const move = (event: ReactPointerEvent<SVGSVGElement>) => {
    const drag = dragNow.current
    if (!drag) return
    if (drag.kind === 'pan') {
      const rect = svg.current?.getBoundingClientRect()
      const side = rect ? Math.min(rect.width, rect.height) : 1
      setBox({ ...drag.box, x: drag.box.x - ((event.clientX - drag.from[0]) / side) * drag.box.size, y: drag.box.y + ((event.clientY - drag.from[1]) / side) * drag.box.size })
      return
    }
    const to = toUv(event)
    setDrag({ ...drag, to })
    if (drag.kind === 'move') requestPreview([to[0] - drag.from[0], to[1] - drag.from[1]], drag.source)
  }

  const up = (event: ReactPointerEvent<SVGSVGElement>) => {
    const current = dragNow.current
    invalidatePreview()
    setDrag(null)
    if (!current || !layout || current.kind === 'pan') return
    const [u0, v0] = current.from
    const [u1, v1] = current.to
    if (current.kind === 'move') {
      if (Math.hypot(u1 - u0, v1 - v0) > 1e-6) void run('uv.translate', { offset: [u1 - u0, v1 - v0], source: current.source })
      return
    }
    const extend = event.shiftKey
    if (Math.hypot(u1 - u0, v1 - v0) < PICK * box.size * 0.5) {
      const vertex = vertexAt(current.from)
      if (vertex != null) {
        void run('mesh.select', { vertices: [vertex], extend })
        return
      }
      const face = faceAt(current.from)
      if (face) {
        const index = layout.faces.indexOf(face)
        void run('mesh.select', { faces: island(index), extend })
        return
      }
      if (!extend) void run('mesh.select', { vertices: [] })
      return
    }
    const [low, high] = [[Math.min(u0, u1), Math.min(v0, v1)], [Math.max(u0, u1), Math.max(v0, v1)]]
    const inside = new Set<number>()
    for (const face of layout.faces) {
      face.uvs.forEach(([u, v], corner) => {
        if (u >= low[0] && u <= high[0] && v >= low[1] && v <= high[1]) inside.add(face.vertices[corner])
      })
    }
    void run('mesh.select', { vertices: [...inside], extend })
  }

  const wheel = (event: ReactWheelEvent<SVGSVGElement>) => {
    const at = toUv(event)
    const factor = Math.exp(Math.max(-200, Math.min(200, event.deltaY)) * 0.002)
    const size = Math.min(20, Math.max(0.02, box.size * factor))
    setBox({ x: at[0] - ((at[0] - box.x) * size) / box.size, y: at[1] - ((at[1] - box.y) * size) / box.size, size })
  }

  // Coordinates are drawn in UV units, V up: the SVG's y is -v.
  const viewBox = `${box.x} ${-(box.y + box.size)} ${box.size} ${box.size}`
  const offset: [number, number] = drag?.kind === 'move' && !preview ? [drag.to[0] - drag.from[0], drag.to[1] - drag.from[1]] : [0, 0]
  const at = (vertex: number, [u, v]: [number, number]): [number, number] => (selected.has(vertex) ? [u + offset[0], v + offset[1]] : [u, v])
  const stroke = box.size / 400

  const shown = preview ?? layout
  const busy = readOnly || !layout
  return (
    <section className="scene-uv" data-testid="scene-uv-editor" aria-label={t('uvEditor')}>
      <header className="scene-uv-bar">
        <span className="scene-uv-title">{t('uvEditor')}</span>
        <IconButton icon={ScissorsIcon} label={phrase('Mark Seam')} disabled={busy || selected.size < 2} onClick={() => void run('mesh.markSeam', {})} data-testid="scene-uv-mark-seam" />
        <button type="button" className="scene-uv-action" disabled={busy} onClick={() => void run('uv.unwrap', { method: layout?.method ?? 'angleBased', iterations: layout?.iterations ?? 30 })} data-testid="scene-uv-unwrap">
          {phrase('Unwrap')}
        </button>
        <IconButton icon={RotateCcwIcon} label={t('uvRotateLeft')} disabled={busy || selected.size === 0} onClick={() => void run('uv.rotate', { angle: 90 })} />
        <IconButton icon={RotateCwIcon} label={t('uvRotateRight')} disabled={busy || selected.size === 0} onClick={() => void run('uv.rotate', { angle: -90 })} />
        <IconButton icon={PinIcon} label={phrase('Pin')} disabled={busy || selected.size === 0} onClick={() => void run('uv.pin', {})} data-testid="scene-uv-pin" />
        <IconButton icon={PinOffIcon} label={phrase('Unpin')} disabled={busy || selected.size === 0} onClick={() => void run('uv.unpin', {})} />
        <span className="scene-uv-spacer" />
        <Checkbox label={t('checker')} checked={checker} onChange={setChecker} testId="scene-uv-checker" />
        <IconButton
          icon={XIcon}
          label={t('close')}
          onClick={() => {
            onClose()
            // The button goes away with the panel: the editor keeps the keyboard.
            document.querySelector<HTMLElement>('.kui-app')?.focus({ preventScroll: true })
          }}
          data-testid="scene-uv-close"
        />
      </header>
      <div className="scene-uv-options">
        <select className="scene-uv-action" aria-label={t('uvMethod')} value={layout?.method ?? 'angleBased'} disabled={busy} onChange={(event) => void run('uv.liveUnwrap', { method: event.target.value })} data-testid="scene-uv-method">
          <option value="angleBased">{t('uvAngleBased')}</option>
          <option value="conformal">{t('uvConformal')}</option>
          <option value="minimumStretch">{t('uvMinimumStretch')}</option>
        </select>
        <Checkbox label={t('uvLiveUnwrap')} checked={layout?.liveUnwrap ?? false} disabled={busy} onChange={(on) => void run('uv.liveUnwrap', { on })} testId="scene-uv-live" />
      </div>
      <svg
        ref={svg}
        className="scene-uv-canvas"
        viewBox={viewBox}
        preserveAspectRatio="xMidYMid meet"
        onPointerDown={down}
        onPointerMove={move}
        onPointerUp={up}
        onPointerCancel={() => { invalidatePreview(); setDrag(null) }}
        onWheel={wheel}
        onDoubleClick={() => setBox(HOME)}
        data-testid="scene-uv-canvas"
      >
        <defs>
          <pattern id="scene-uv-checker" width={0.125} height={0.125} patternUnits="userSpaceOnUse">
            <rect width={0.125} height={0.125} fill="#d9d9de" />
            <rect width={0.0625} height={0.0625} fill="#f4f4f6" />
            <rect x={0.0625} y={0.0625} width={0.0625} height={0.0625} fill="#f4f4f6" />
          </pattern>
        </defs>
        <rect x={0} y={-1} width={1} height={1} fill={checker || !image ? 'url(#scene-uv-checker)' : '#fff'} />
        {image && !checker ? <image href={image} x={0} y={-1} width={1} height={1} preserveAspectRatio="none" opacity={0.85} /> : null}
        <rect x={0} y={-1} width={1} height={1} fill="none" stroke="#9a9aa2" strokeWidth={stroke} />
        {shown?.faces.map((face) => (
          <polygon
            key={face.face}
            points={face.uvs.map((uv, corner) => at(face.vertices[corner], uv)).map(([u, v]) => `${u},${-v}`).join(' ')}
            className="scene-uv-face"
            data-selected={face.selected || undefined}
            strokeWidth={stroke}
          />
        ))}
        {shown?.faces.flatMap((face) =>
          face.vertices.map((vertex, corner) => {
            const next = face.vertices[(corner + 1) % face.vertices.length]
            const key = vertex < next ? `${vertex}:${next}` : `${next}:${vertex}`
            if (!seams.has(key)) return null
            const [a, b] = [at(vertex, face.uvs[corner]), at(next, face.uvs[(corner + 1) % face.uvs.length])]
            return <line key={`${face.face}-${corner}`} x1={a[0]} y1={-a[1]} x2={b[0]} y2={-b[1]} className="scene-uv-seam" strokeWidth={stroke * 2.5} />
          }),
        )}
        {shown?.faces.flatMap((face) =>
          face.vertices.map((vertex, corner) => {
            const [u, v] = at(vertex, face.uvs[corner])
            const size = stroke * (selected.has(vertex) ? 4.5 : 3)
            return (
              <rect
                key={`${face.face}-${corner}-v`}
                x={u - size / 2}
                y={-v - size / 2}
                width={size}
                height={size}
                className="scene-uv-vertex"
                data-selected={selected.has(vertex) || undefined}
                data-pinned={pinned.has(vertex) || undefined}
                data-vertex={vertex}
              />
            )
          }),
        )}
        {drag?.kind === 'box' ? (
          <rect
            x={Math.min(drag.from[0], drag.to[0])}
            y={-Math.max(drag.from[1], drag.to[1])}
            width={Math.abs(drag.to[0] - drag.from[0])}
            height={Math.abs(drag.to[1] - drag.from[1])}
            className="scene-uv-box"
            strokeWidth={stroke}
          />
        ) : null}
      </svg>
      {layout && !layout.hasUvs ? <p className="scene-uv-empty">{t('uvNone')}</p> : null}
      {layout && layout.hasUvs && layout.faces.length === 0 ? <p className="scene-uv-empty">{t('uvSelectFaces')}</p> : null}
    </section>
  )
}
