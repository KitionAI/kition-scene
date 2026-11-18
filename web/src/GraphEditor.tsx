import { useMemo, useRef, useState } from 'react'
import { Choice, useKitEditor, useReadOnly, type KitEngineInfo } from '@kition/kit-editor'
import { sceneToTweak, tweakToScene, type FModifierInfo, type NlaTweakInfo } from './AnimationData'
import { FModifierPanel } from './FModifierPanel'
import type { SceneInfo } from './scene'

/** One channel as the engine's `inspect()` lists it. */
type Channel = {
  property: string
  index: number
  path?: string
  object?: number
  material?: number
  keys: { frame: number; value: number; interpolation: string }[]
  /** F-curve modifiers, and the modified curve sampled over the timeline. */
  modifiers?: FModifierInfo[]
  samples?: [number, number][]
}

type Picked = { channel: number; frame: number }

const AXES = ['X', 'Y', 'Z', 'W']
const COLORS = ['#d9485f', '#3f9b4e', '#3b6fd9', '#8a63d2']
const HEIGHT = 168
const PAD = { left: 40, right: 12, top: 10, bottom: 18 }

function channelsOf(info: KitEngineInfo | null | undefined): Channel[] {
  const raw = (info?.inspected as Record<string, any> | undefined)?.animation?.channels
  return Array.isArray(raw) ? (raw as Channel[]) : []
}

function label(channel: Channel) {
  const axis = channel.property === 'shapeKey' || channel.property === 'power' || channel.property === 'lens' || channel.property === 'hidden' ? '' : ` ${AXES[channel.index] ?? channel.index}`
  return `${channel.path ? `${channel.path} · ` : ''}${channel.property}${axis}`
}

/** Linear, constant, or a Bézier between keys with flat-ish handles: a preview of the curve. */
function pathOf(channel: Channel, x: (frame: number) => number, y: (value: number) => number) {
  const keys = channel.keys.map((key) => ({ ...key, frame: x(key.frame) }))
  const ident = (frame: number) => frame
  return keyPath(keys, ident, y)
}

function keyPath(keys: Channel['keys'], x: (frame: number) => number, y: (value: number) => number) {
  if (!keys.length) return ''
  let d = `M${x(keys[0].frame)},${y(keys[0].value)}`
  for (let index = 1; index < keys.length; index += 1) {
    const from = keys[index - 1]
    const to = keys[index]
    if (from.interpolation === 'constant') d += ` H${x(to.frame)} V${y(to.value)}`
    else if (from.interpolation === 'linear') d += ` L${x(to.frame)},${y(to.value)}`
    else {
      const third = (to.frame - from.frame) / 3
      d += ` C${x(from.frame + third)},${y(from.value)} ${x(to.frame - third)},${y(to.value)} ${x(to.frame)},${y(to.value)}`
    }
  }
  return d
}

/**
 * The curves of the selected objects (and their materials): keys as dots, the current frame as a
 * line. Drag a key to move it in time and value; pick one to change its interpolation or delete it.
 */
export function GraphEditor({ info, engine }: { info: SceneInfo; engine: KitEngineInfo | null | undefined }) {
  const { run, t, phrase: p } = useKitEditor()
  const readOnly = useReadOnly()
  const svg = useRef<SVGSVGElement>(null)
  const [width, setWidth] = useState(640)
  const [picked, setPicked] = useState<Picked | null>(null)
  const [drag, setDrag] = useState<{ picked: Picked; frame: number; value: number } | null>(null)
  const animation = (engine?.inspected as Record<string, any> | undefined)?.animation ?? {}
  const materials = info.objects.filter((object) => info.selected.includes(object.id)).map((object) => object.material).filter((id): id is number => id != null)
  const channels = useMemo(
    () => channelsOf(engine).filter((channel) => (channel.object != null && info.selected.includes(channel.object)) || (channel.material != null && materials.includes(channel.material))),
    [engine, info.selected, materials],
  )
  const start = Number(animation.start ?? 1)
  const end = Math.max(start + 1, Number(animation.end ?? 250))
  const frames = channels.flatMap((channel) => channel.keys.map((key) => tweakToScene(channel.object != null && channel.property !== 'shapeKey' ? (info.objects.find((object) => object.id === channel.object)?.nla?.tweak ?? null) : null, key.frame)))
  const low = Math.min(start, ...frames)
  const high = Math.max(end, ...frames)
  const values = channels.flatMap((channel) => [...channel.keys.map((key) => key.value), ...(channel.samples ?? []).map((sample) => sample[1])])
  const min = values.length ? Math.min(...values) : 0
  const max = values.length ? Math.max(...values) : 1
  const span = max - min || 1
  const x = (frame: number) => PAD.left + ((frame - low) / (high - low || 1)) * (width - PAD.left - PAD.right)
  const y = (value: number) => PAD.top + (1 - (value - (min - span * 0.1)) / (span * 1.2)) * (HEIGHT - PAD.top - PAD.bottom)
  const toFrame = (px: number) => low + ((px - PAD.left) / (width - PAD.left - PAD.right)) * (high - low || 1)
  const toValue = (py: number) => min - span * 0.1 + (1 - (py - PAD.top) / (HEIGHT - PAD.top - PAD.bottom)) * span * 1.2
  const target = (channel: Channel) => channel.object ?? channel.material ?? 0
  // In tweak mode an object's own keys sit at their action's frames: shown at the scene's.
  const tweaks = new Map<number, NlaTweakInfo>(info.objects.filter((object) => object.nla?.tweak).map((object) => [object.id, object.nla!.tweak!]))
  const tweakOf = (channel: Channel) => (channel.object != null && channel.property !== 'shapeKey' ? (tweaks.get(channel.object) ?? null) : null)
  const shown = (channel: Channel, frame: number) => tweakToScene(tweakOf(channel), frame)
  const keyParams = (pick: Picked) => {
    const channel = channels[pick.channel]
    return { target: target(channel), property: channel.property, index: channel.index, path: channel.path ?? '', frame: pick.frame }
  }
  const point = (event: React.PointerEvent) => {
    const box = svg.current?.getBoundingClientRect()
    return box ? { px: event.clientX - box.left, py: event.clientY - box.top } : { px: 0, py: 0 }
  }
  const pickedKey = picked ? channels[picked.channel]?.keys.find((key) => Math.abs(key.frame - picked.frame) < 1e-3) : undefined
  return (
    <section className="scene-graph" data-testid="scene-graph">
      <div className="scene-graph-header">
        <strong>{p('Graph Editor')}</strong>
        <span className="kui-muted">{channels.length ? channels.map(label).slice(0, 6).join(' · ') : t('noCurves')}</span>
        <span style={{ flex: 1 }} />
        {pickedKey ? (
          <>
            <span className="tabular-nums kui-muted" data-testid="scene-graph-picked">
              {t('keyAt', { frame: Math.round(pickedKey.frame * 100) / 100, value: Math.round(pickedKey.value * 1000) / 1000 })}
            </span>
            <Choice
              label={t('interpolation')}
              quiet
              value={pickedKey.interpolation}
              disabled={readOnly}
              choices={[
                ['bezier', t('bezier')],
                ['linear', t('linear')],
                ['constant', t('constant')],
              ]}
              onChange={(interpolation) => picked && void run('graph.setInterpolation', { ...keyParams(picked), interpolation })}
              testId="scene-graph-interpolation"
            />
            <button
              type="button"
              className="kui-button"
              data-size="sm"
              disabled={readOnly}
              onClick={() => {
                if (!picked) return
                void run('graph.deleteKey', keyParams(picked))
                setPicked(null)
              }}
            >
              {p('Delete Keyframe')}
            </button>
          </>
        ) : null}
      </div>
      <svg
        ref={(element) => {
          svg.current = element
          if (element && Math.abs(element.clientWidth - width) > 2) setWidth(element.clientWidth)
        }}
        className="scene-graph-canvas"
        height={HEIGHT}
        role="img"
        aria-label={p('Graph Editor')}
        onPointerMove={(event) => {
          if (!drag) return
          const { px, py } = point(event)
          setDrag({ ...drag, frame: Math.round(toFrame(px)), value: toValue(py) })
        }}
        onPointerUp={() => {
          if (!drag) return
          const moved = drag.frame !== Math.round(shown(channels[drag.picked.channel], drag.picked.frame)) || Math.abs(drag.value - (channels[drag.picked.channel]?.keys.find((key) => key.frame === drag.picked.frame)?.value ?? drag.value)) > 1e-6
          const channel = channels[drag.picked.channel]
          const toFrame = channel ? sceneToTweak(tweakOf(channel), drag.frame) : drag.frame
          if (moved) void run('graph.moveKey', { ...keyParams(drag.picked), toFrame, value: drag.value })
          setPicked({ channel: drag.picked.channel, frame: Math.round(toFrame * 1000) / 1000 })
          setDrag(null)
        }}
      >
        <line x1={x(Number(animation.frame ?? start))} x2={x(Number(animation.frame ?? start))} y1={0} y2={HEIGHT} className="scene-graph-now" />
        <line x1={PAD.left} x2={width - PAD.right} y1={y(0)} y2={y(0)} className="scene-graph-zero" />
        {channels.map((channel, index) =>
          channel.samples?.length ? (
            <polyline
              key={`${label(channel)}-${index}`}
              points={channel.samples.map(([frame, value]) => `${x(shown(channel, frame))},${y(value)}`).join(' ')}
              fill="none"
              stroke={COLORS[channel.index % COLORS.length]}
              strokeWidth={1.5}
              className="scene-graph-modified"
              onPointerDown={() => setPicked({ channel: index, frame: Number.NaN })}
              data-testid="scene-graph-curve"
            />
          ) : (
            <path
              key={`${label(channel)}-${index}`}
              d={pathOf(channel, (frame) => x(shown(channel, frame)), y)}
              fill="none"
              stroke={COLORS[channel.index % COLORS.length]}
              strokeWidth={1.5}
              onPointerDown={() => setPicked({ channel: index, frame: Number.NaN })}
              data-testid="scene-graph-curve"
            />
          ),
        )}
        {channels.map((channel, index) =>
          channel.keys.map((key) => {
            const dragging = drag && drag.picked.channel === index && drag.picked.frame === key.frame
            const chosen = picked && picked.channel === index && Math.abs(picked.frame - key.frame) < 1e-3
            return (
              <circle
                key={`${index}-${key.frame}`}
                cx={x(dragging ? drag.frame : shown(channel, key.frame))}
                cy={y(dragging ? drag.value : key.value)}
                r={chosen ? 5 : 4}
                className="scene-graph-key"
                data-picked={chosen || undefined}
                onPointerDown={(event) => {
                  if (readOnly) return
                  event.stopPropagation()
                  ;(event.currentTarget.ownerSVGElement ?? event.currentTarget).setPointerCapture?.(event.pointerId)
                  const pick = { channel: index, frame: key.frame }
                  setPicked(pick)
                  setDrag({ picked: pick, frame: Math.round(shown(channel, key.frame)), value: key.value })
                }}
                data-testid="scene-graph-key"
                data-frame={key.frame}
              />
            )
          }),
        )}
        <text x={4} y={y(max) + 4} className="scene-graph-label">
          {Math.round(max * 100) / 100}
        </text>
        <text x={4} y={y(min) + 4} className="scene-graph-label">
          {Math.round(min * 100) / 100}
        </text>
      </svg>
      {picked && channels[picked.channel] ? (
        <FModifierPanel
          channel={{ target: target(channels[picked.channel]), property: channels[picked.channel].property, index: channels[picked.channel].index, path: channels[picked.channel].path ?? '' }}
          modifiers={channels[picked.channel].modifiers ?? []}
          frame={Math.round(sceneToTweak(tweakOf(channels[picked.channel]), Number(animation.frame ?? start)))}
          label={label(channels[picked.channel])}
        />
      ) : channels.length ? (
        <p className="kui-muted scene-nla-empty">{t('fmodNoChannel')}</p>
      ) : null}
    </section>
  )
}
