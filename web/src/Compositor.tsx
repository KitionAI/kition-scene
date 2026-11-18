import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import { Checkbox, Choice, useKitEditor, useReadOnly } from '@kition/kit-editor'
import { linearHex, type SceneInfo } from './scene'
import type { NodeGroupInfo, NodeInfo } from './NodeEditorTypes'
import { Row } from './SidePanel'
import './Compositor.css'

/** The compositing groups of the scene. */
export function compositorGroups(info: SceneInfo) {
  return info.nodeGroups.filter((group) => group.tree === 'compositor')
}

function parsed(result: unknown): any {
  if (typeof result === 'string') {
    try {
      return JSON.parse(result)
    } catch {
      return null
    }
  }
  return result
}

/** The Scene panel's compositing rows: whether renders pass through a group, and which. */
export function CompositingRows({ info }: { info: SceneInfo }) {
  const { run, t } = useKitEditor()
  const readOnly = useReadOnly()
  const groups = compositorGroups(info)
  return (
    <>
      <Row label={t('compositing')}>
        <span className="scene-compositing">
          <Choice
            label={t('compositing')}
            quiet
            value={info.render.compositor != null ? String(info.render.compositor) : ''}
            disabled={readOnly}
            choices={[['', t('noneChoice')] as const, ...groups.map((group) => [String(group.id), group.name] as const)]}
            onChange={(value) => void run('compositor.set', { group: value === '' ? null : Number(value) })}
            testId="scene-compositor-group"
          />
          <button type="button" className="kui-button" data-size="sm" disabled={readOnly} onClick={() => void run('compositor.new', {})} data-testid="scene-compositor-new">
            {t('newNodeGroup')}
          </button>
        </span>
      </Row>
      {info.render.compositor != null ? (
        <Row label="">
          <Checkbox label={t('useCompositing')} checked={info.render.compositing} disabled={readOnly} onChange={(on) => void run('compositor.set', { compositing: on })} testId="scene-compositing" />
        </Row>
      ) : null}
    </>
  )
}

/** An sRGB `#rrggbb` as linear RGB. */
function hexLinear(hex: string): [number, number, number] {
  const byte = (at: number) => Number.parseInt(hex.slice(at, at + 2), 16) / 255
  const linear = (value: number) => (value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4)
  return [linear(byte(1)), linear(byte(3)), linear(byte(5))]
}

/** A colour input's value on a node: a swatch (sRGB on screen, linear in the node). */
export function NodeColor({ node, id, value, disabled, onSet }: { node: NodeInfo; id: string; value: unknown; disabled: boolean; onSet: (value: unknown) => void }) {
  const color = Array.isArray(value) ? (value as number[]) : [1, 1, 1, 1]
  return (
    <input
      type="color"
      className="scene-node-color"
      value={linearHex(color)}
      disabled={disabled}
      onChange={(event) => onSet([...hexLinear(event.target.value), color[3] ?? 1])}
      onKeyDown={(event) => event.stopPropagation()}
      data-testid={`scene-node-${node.id}-${id}`}
    />
  )
}

/** An Image node's picture: one of the scene's images, or a file read in. */
export function NodeImage({ node, group, info, disabled }: { node: NodeInfo; group: NodeGroupInfo; info: SceneInfo; disabled: boolean }) {
  const { run, t } = useKitEditor()
  const file = useRef<HTMLInputElement>(null)
  const value = typeof node.values.Image === 'string' ? node.values.Image : ''
  const read = async (chosen: File) => {
    const bytes = new Uint8Array(await chosen.arrayBuffer())
    let binary = ''
    for (let index = 0; index < bytes.length; index += 0x8000) binary += String.fromCharCode(...bytes.subarray(index, index + 0x8000))
    void run('compositor.setImage', { group: group.id, node: node.id, base64: btoa(binary), name: chosen.name })
  }
  return (
    <span className="scene-node-image">
      <select
        className="scene-node-input"
        value={value}
        disabled={disabled}
        onChange={(event) => void run('compositor.setImage', event.target.value ? { group: group.id, node: node.id, image: event.target.value } : { group: group.id, node: node.id, clear: true })}
        onKeyDown={(event) => event.stopPropagation()}
        data-testid={`scene-node-${node.id}-image`}
      >
        <option value="">{t('noneChoice')}</option>
        {info.images.map((image) => (
          <option key={image.hash} value={image.hash}>
            {image.name}
          </option>
        ))}
      </select>
      <button type="button" className="kui-button" data-size="sm" disabled={disabled} onClick={() => file.current?.click()} data-testid={`scene-node-${node.id}-load`}>
        {t('loadImage')}
      </button>
      <input
        ref={file}
        type="file"
        accept="image/png,image/jpeg,.hdr"
        hidden
        onChange={(event) => {
          const chosen = event.target.files?.[0]
          event.target.value = ''
          if (chosen) void read(chosen)
        }}
      />
    </span>
  )
}

type Curves = Partial<Record<'c' | 'r' | 'g' | 'b', [number, number][]>>
const SIDE = 132
const STRAIGHT: [number, number][] = [
  [0, 0],
  [1, 1],
]

/** RGB Curves' curves: a channel's points on a square, dragged to move, double-click to add,
 *  right-click to remove (the ends stay). The line drawn joins the points; the node bends it
 *  smoothly as the reference does. */
export function NodeCurves({ node, disabled, onSet }: { node: NodeInfo; disabled: boolean; onSet: (value: unknown) => void }) {
  const { t } = useKitEditor()
  const stored = (node.values.curves as Curves | undefined) ?? {}
  const [channel, setChannel] = useState<keyof Curves>('c')
  const [draft, setDraft] = useState<[number, number][] | null>(null)
  const dragging = useRef<number | null>(null)
  const box = useRef<SVGSVGElement>(null)
  const points = draft ?? stored[channel] ?? STRAIGHT
  useEffect(() => setDraft(null), [node.values.curves])
  const commit = (next: [number, number][]) => onSet({ ...stored, [channel]: [...next].sort((a, b) => a[0] - b[0]) })
  const at = (event: { clientX: number; clientY: number }): [number, number] => {
    const rect = box.current?.getBoundingClientRect()
    if (!rect) return [0, 0]
    const clamp = (value: number) => Math.min(1, Math.max(0, value))
    return [clamp((event.clientX - rect.left) / rect.width), clamp(1 - (event.clientY - rect.top) / rect.height)]
  }
  const down = (event: ReactPointerEvent, index: number) => {
    event.stopPropagation()
    if (disabled) return
    if (event.button === 2) {
      event.preventDefault()
      if (index > 0 && index < points.length - 1) commit(points.filter((_, other) => other !== index))
      return
    }
    dragging.current = index
    box.current?.setPointerCapture(event.pointerId)
  }
  const move = (event: ReactPointerEvent) => {
    const index = dragging.current
    if (index == null) return
    const [x, y] = at(event)
    const low = index > 0 ? points[index - 1][0] + 0.01 : 0
    const high = index < points.length - 1 ? points[index + 1][0] - 0.01 : 1
    const next = points.map((point, other) => (other === index ? [Math.min(high, Math.max(low, x)), y] : point) as [number, number])
    setDraft(next)
  }
  const up = () => {
    if (dragging.current == null) return
    dragging.current = null
    if (draft) commit(draft)
  }
  const path = points.map(([x, y], index) => `${index ? 'L' : 'M'}${x * SIDE},${(1 - y) * SIDE}`).join(' ')
  return (
    <span className="scene-node-curves" onPointerDown={(event) => event.stopPropagation()}>
      <span className="scene-node-curves-tabs">
        {(['c', 'r', 'g', 'b'] as const).map((item) => (
          <button key={item} type="button" data-active={item === channel || undefined} onClick={() => setChannel(item)} data-testid={`scene-node-${node.id}-curve-${item}`}>
            {item.toUpperCase()}
          </button>
        ))}
      </span>
      <svg
        ref={box}
        className="scene-node-curves-box"
        width={SIDE}
        height={SIDE}
        data-channel={channel}
        onPointerMove={move}
        onPointerUp={up}
        onContextMenu={(event) => event.preventDefault()}
        onDoubleClick={(event) => !disabled && commit([...points, at(event)])}
        aria-label={t('curves')}
        data-testid={`scene-node-${node.id}-curves`}
      >
        <path d={`M0,${SIDE} L${SIDE},0`} className="scene-node-curves-diagonal" />
        <path d={path} className="scene-node-curves-line" />
        {points.map(([x, y], index) => (
          <circle key={index} cx={x * SIDE} cy={(1 - y) * SIDE} r={4} onPointerDown={(event) => down(event, index)} data-testid={`scene-node-${node.id}-curve-point-${index}`} />
        ))}
      </svg>
    </span>
  )
}

/** The event a Cryptomatte node's Pick button sends the backdrop: the node to pick for, or
 *  `null` to stop picking. */
const PICK_EVENT = 'scene-matte-pick'
type PickTarget = { group: number; node: number } | null

/** A Cryptomatte node's matte: the names it keeps (separated by commas), and a button that
 *  picks one by clicking the backdrop (Shift-click takes one away). */
export function NodeMatte({ node, group, value, disabled, onSet }: { node: NodeInfo; group: NodeGroupInfo; value: string; disabled: boolean; onSet: (value: unknown) => void }) {
  const { t } = useKitEditor()
  const [picking, setPicking] = useState(false)
  useEffect(() => {
    const listen = (event: Event) => {
      const target = (event as CustomEvent<PickTarget>).detail
      setPicking(target?.node === node.id && target.group === group.id)
    }
    window.addEventListener(PICK_EVENT, listen)
    return () => window.removeEventListener(PICK_EVENT, listen)
  }, [node.id, group.id])
  const toggle = () => window.dispatchEvent(new CustomEvent<PickTarget>(PICK_EVENT, { detail: picking ? null : { group: group.id, node: node.id } }))
  return (
    <span className="scene-node-matte">
      <input
        key={value}
        className="scene-node-input"
        defaultValue={value}
        disabled={disabled}
        onKeyDown={(event) => {
          event.stopPropagation()
          if (event.key === 'Enter') (event.target as HTMLInputElement).blur()
        }}
        onBlur={(event) => event.target.value !== value && onSet(event.target.value)}
        data-testid={`scene-node-${node.id}-matte`}
      />
      <button type="button" className="kui-button" data-size="sm" data-active={picking || undefined} aria-pressed={picking} disabled={disabled} onClick={toggle} title={t('pickMatteHint')} data-testid={`scene-node-${node.id}-pick`}>
        {t('pickMatte')}
      </button>
    </span>
  )
}

/** What the compositing group makes of a small render (its Viewer when it has one), behind the
 *  node editor's canvas; made again a moment after each change. */
export function CompositorBackdrop({ group, revision }: { group: NodeGroupInfo; revision: string }) {
  const { run, t } = useKitEditor()
  const [url, setUrl] = useState<string | null>(null)
  const [failed, setFailed] = useState<string | null>(null)
  const [pick, setPick] = useState<PickTarget>(null)
  useEffect(() => {
    const listen = (event: Event) => setPick((event as CustomEvent<PickTarget>).detail)
    window.addEventListener(PICK_EVENT, listen)
    return () => window.removeEventListener(PICK_EVENT, listen)
  }, [])
  const picked = (event: ReactPointerEvent<HTMLImageElement>) => {
    if (!pick) return
    event.stopPropagation()
    const rect = event.currentTarget.getBoundingClientRect()
    const x = (event.clientX - rect.left) / Math.max(rect.width, 1)
    const y = (event.clientY - rect.top) / Math.max(rect.height, 1)
    void run('compositor.pickMatte', { group: pick.group, node: pick.node, x, y, remove: event.shiftKey })
  }
  useEffect(() => {
    let live = true
    const timer = window.setTimeout(() => {
      void run('compositor.preview', { group: group.id, size: 360, viewer: true })
        .then((result) => {
          const answer = parsed(result)
          if (!live || typeof answer?.base64 !== 'string') return
          setFailed(null)
          setUrl(`data:image/png;base64,${answer.base64}`)
        })
        .catch((error: unknown) => live && setFailed(error instanceof Error ? error.message : String(error)))
    }, 250)
    return () => {
      live = false
      window.clearTimeout(timer)
    }
  }, [run, group.id, revision])
  return (
    <div className="scene-compositor-backdrop" data-testid="scene-compositor-backdrop" data-ready={url ? true : undefined} data-picking={pick ? true : undefined}>
      {url ? <img src={url} alt={t('compositing')} onPointerDown={picked} data-testid="scene-compositor-backdrop-image" /> : <span className="kui-muted">{failed ?? t('rendering')}</span>}
    </div>
  )
}
