import { useRef, useState } from 'react'
import { PlusIcon, TrashIcon } from '@kition/tessera/icons/react'
import { Button, Checkbox, Choice, IconButton, MenuButton, MenuItem, useKitEditor, useReadOnly } from '@kition/sdk/editor'
import type { NlaStripInfo, ObjectNla } from './AnimationData'
import type { AnimationInfo } from './animation'
import { NumberInput, Row } from './SidePanel'
import type { SceneInfo } from './scene'
import './NlaEditor.css'

const ROW = 28
const HEADER = 196
const PAD = 10
const MIN_WIDTH = 10
const BLENDS = ['replace', 'combine', 'add', 'subtract', 'multiply'] as const
const EXTRAPOLATIONS = ['hold', 'holdForward', 'nothing'] as const

type Picked = { track: number; strip: number }

/** The strip's outline: a ramp up over its blend in, flat, a ramp down over its blend out. */
function stripShape(strip: NlaStripInfo, x: (frame: number) => number, top: number) {
  const bottom = top + ROW - 6
  const high = top + 4
  const fixed = strip.influence != null || strip.kind !== 'action'
  // Short strips stay wide enough to pick.
  const left = x(strip.start)
  const right = Math.max(x(strip.end), left + MIN_WIDTH)
  const span = x(strip.end) - left || 1
  const at = (frames: number) => left + ((right - left) * (x(strip.start + frames) - left)) / span
  const inEnd = at(fixed ? 0 : strip.blendIn)
  const outStart = at(strip.end - strip.start - (fixed ? 0 : strip.blendOut))
  return `M${left},${bottom} L${inEnd},${high} L${outStart},${high} L${right},${bottom} Z`
}

/**
 * The active object's NLA under the viewport: its tracks (top track at the top) with their
 * action strips over the frames, the object's own keys above them, and the picked strip's
 * settings. Drag a strip to move it (one undo step), double-click it to tweak its action.
 */
export function NlaEditor({ info, animation }: { info: SceneInfo; animation: AnimationInfo }) {
  const { run, t, phrase: p } = useKitEditor()
  const readOnly = useReadOnly()
  const svg = useRef<SVGSVGElement>(null)
  const [width, setWidth] = useState(640)
  const [picked, setPicked] = useState<Picked | null>(null)
  const [drag, setDrag] = useState<{ picked: Picked; from: number; delta: number } | null>(null)
  const object = info.objects.find((candidate) => candidate.id === info.active)
  if (!object) {
    return (
      <section className="scene-nla" data-testid="scene-nla">
        <div className="scene-nla-header">
          <strong>{t('nlaEditor')}</strong>
          <span className="kui-muted">{t('nlaNoObject')}</span>
        </div>
      </section>
    )
  }
  const nla: ObjectNla = object.nla ?? { tracks: [], action: { blend: 'replace', influence: 1, extrapolation: 'hold' }, tweak: null }
  const tweak = nla.tweak
  const id = object.id
  const busy = readOnly || info.mode !== 'object'
  const strips = nla.tracks.flatMap((track) => track.strips)
  const low = Math.min(animation.start, ...strips.map((strip) => strip.start))
  const high = Math.max(animation.end, ...strips.map((strip) => strip.end))
  const lane = Math.max(80, width - HEADER)
  const x = (frame: number) => PAD + ((frame - low) / (high - low || 1)) * (lane - 2 * PAD)
  const perFrame = (lane - 2 * PAD) / (high - low || 1)
  const rows = [...nla.tracks.keys()].reverse()
  const height = (rows.length + 1) * ROW
  const pickedStrip = picked ? nla.tracks[picked.track]?.strips[picked.strip] : undefined
  const nextStrip = picked ? nla.tracks[picked.track]?.strips[picked.strip + 1] : undefined
  const groupLast = picked ? picked.strip + (nextStrip?.kind === 'transition' ? 2 : 1) : 0
  const canGroup = !!picked && !!nla.tracks[picked.track]?.strips[groupLast] && pickedStrip?.kind !== 'transition'
  const setStrip = (values: Record<string, unknown>) => picked && run('nla.setStrip', { object: id, track: picked.track, strip: picked.strip, ...values })
  const tweakedName = tweak ? nla.tracks[tweak.track]?.strips[tweak.strip]?.action : undefined
  return (
    <section className="scene-nla" data-testid="scene-nla">
      <div className="scene-nla-header">
        <strong>{t('nlaEditor')}</strong>
        <span className="kui-muted">{object.name}</span>
        <span style={{ flex: 1 }} />
        {tweak ? (
          <>
            <span className="kui-muted" data-testid="scene-nla-tweaking">
              {t('nlaTweaking', { action: tweakedName ?? '' })}
            </span>
            <Button size="sm" disabled={busy} onClick={() => run('nla.tweakExit', { object: id })} data-testid="scene-nla-tweak-exit">
              {t('nlaStopTweaking')}
            </Button>
          </>
        ) : (
          <>
            <Button size="sm" disabled={busy || object.animated.length === 0} onClick={() => run('nla.pushDown', { object: id })} data-testid="scene-nla-push-down">
              {p('Push Down')}
            </Button>
            <MenuButton
              align="end"
              trigger={({ open, toggle }) => (
                <Button size="sm" aria-expanded={open} disabled={busy} onClick={toggle} data-testid="scene-nla-add-strip">
                  {t('nlaAddStrip')}
                </Button>
              )}
            >
              {info.animation.actions.length ? (
                info.animation.actions.map((action) => (
                  <MenuItem key={action.name} label={action.name} onSelect={() => run('nla.addStrip', { object: id, action: action.name })} testId={`scene-nla-add-${action.name}`} />
                ))
              ) : (
                <MenuItem label={t('nlaNoActions')} disabled onSelect={() => undefined} />
              )}
            </MenuButton>
            <IconButton icon={PlusIcon} label={t('nlaAddTrack')} disabled={busy} onClick={() => run('nla.addTrack', { object: id })} data-testid="scene-nla-add-track" />
          </>
        )}
      </div>
      <div className="scene-nla-body">
        <div className="scene-nla-heads" style={{ width: HEADER }}>
          <div className="scene-nla-head" style={{ height: ROW }}>
            <span className="scene-nla-name">{t('nlaOwnBlend')}</span>
            <Choice label={t('nlaBlending')} quiet value={nla.action.blend} disabled={busy} choices={BLENDS.map((blend) => [blend, t(`nla_${blend}`)] as const)} onChange={(blend) => run('nla.setAction', { object: id, blend })} testId="scene-nla-own-blend" />
          </div>
          {rows.map((track) => {
            const found = nla.tracks[track]
            return (
              <div className="scene-nla-head" key={`${track}-${found.name}`} style={{ height: ROW }} data-testid={`scene-nla-track-${track}`} data-muted={found.muted || undefined}>
                <span className="scene-nla-name" title={found.name}>
                  {found.name}
                </span>
                <Checkbox label={t('nlaMute')} checked={found.muted} disabled={busy || tweak != null} onChange={(muted) => run('nla.setTrack', { object: id, track, muted })} testId={`scene-nla-track-${track}-mute`} />
                <Checkbox label={t('nlaSolo')} checked={found.solo} disabled={busy || tweak != null} onChange={(solo) => run('nla.setTrack', { object: id, track, solo })} testId={`scene-nla-track-${track}-solo`} />
                <IconButton icon={TrashIcon} label={t('nlaRemoveTrack')} disabled={busy || tweak != null} onClick={() => run('nla.removeTrack', { object: id, track })} />
              </div>
            )
          })}
        </div>
        <svg
          ref={(element) => {
            svg.current = element
            // SVG elements report no clientWidth in every browser: measure the laid-out box.
            const measured = element ? element.getBoundingClientRect().width + HEADER : width
            if (element && measured > HEADER && Math.abs(measured - width) > 2) setWidth(measured)
          }}
          className="scene-nla-lanes"
          height={height}
          role="img"
          aria-label={t('nlaEditor')}
          onPointerMove={(event) => {
            if (!drag) return
            setDrag({ ...drag, delta: Math.round((event.clientX - drag.from) / perFrame) })
          }}
          onPointerUp={() => {
            if (!drag) return
            const strip = nla.tracks[drag.picked.track]?.strips[drag.picked.strip]
            if (strip && drag.delta !== 0) void run('nla.setStrip', { object: id, track: drag.picked.track, strip: drag.picked.strip, start: strip.start + drag.delta })
            setDrag(null)
          }}
        >
          <rect x={0} y={0} width={lane} height={ROW} className="scene-nla-own" />
          {rows.map((track, row) => (
            <rect key={track} x={0} y={(row + 1) * ROW} width={lane} height={ROW} className="scene-nla-lane" data-odd={row % 2 === 1 || undefined} />
          ))}
          {rows.flatMap((track, row) =>
            nla.tracks[track].strips.map((strip, at) => {
              const moving = drag && drag.picked.track === track && drag.picked.strip === at ? drag.delta : 0
              const shifted = { ...strip, start: strip.start + moving, end: strip.end + moving }
              const chosen = picked?.track === track && picked.strip === at
              const tweaked = tweak?.track === track && tweak.strip === at
              const top = (row + 1) * ROW + 3
              return (
                <g
                  key={`${track}-${at}-${strip.name}`}
                  className="scene-nla-strip"
                  data-picked={chosen || undefined}
                  data-muted={strip.muted || undefined}
                  data-tweaked={tweaked || undefined}
                  data-testid={`scene-nla-strip-${track}-${at}`}
                  onPointerDown={(event) => {
                    if (busy) return
                    event.stopPropagation()
                    ;(event.currentTarget.ownerSVGElement ?? event.currentTarget).setPointerCapture?.(event.pointerId)
                    setPicked({ track, strip: at })
                    if (!tweak && strip.kind !== 'transition') setDrag({ picked: { track, strip: at }, from: event.clientX, delta: 0 })
                  }}
                  onDoubleClick={() => !busy && !tweak && strip.kind === 'action' && run('nla.tweak', { object: id, track, strip: at })}
                >
                  <path d={stripShape(shifted, x, top - 3)} className="scene-nla-strip-body" />
                  <text x={x(shifted.start) + 4} y={top + 15} className="scene-nla-strip-label">
                    {strip.name}
                  </text>
                </g>
              )
            }),
          )}
          <line x1={x(animation.frame)} x2={x(animation.frame)} y1={0} y2={height} className="scene-nla-now" />
        </svg>
      </div>
      {pickedStrip && picked ? (
        <div className="scene-nla-settings" data-testid="scene-nla-settings">
          {pickedStrip.kind !== 'transition' ? <Row label={t('nlaStripStart')}>
            <NumberInput label={t('nlaStripStart')} value={pickedStrip.start} step={1} onCommit={(start) => setStrip({ start })} testId="scene-nla-start" />
            {pickedStrip.kind === 'meta' ? <NumberInput label={t('nlaMetaEnd')} value={pickedStrip.end} step={1} onCommit={(end) => setStrip({ end })} testId="scene-nla-end" /> : <span className="kui-muted">{t('nlaStripEnd', { frame: Math.round(pickedStrip.end * 100) / 100 })}</span>}
          </Row> : <span className="kui-muted">{t('nlaTransitionHint')}</span>}
          {pickedStrip.kind === 'meta' ? <span className="kui-muted">{t('nlaChildren', { count: pickedStrip.children.length })}</span> : null}
          {pickedStrip.kind === 'action' ? <><Row label={t('nlaActionStart')}>
            <NumberInput label={t('nlaActionStart')} value={pickedStrip.actionStart} step={1} onCommit={(actionStart) => setStrip({ actionStart })} testId="scene-nla-action-start" />
            <NumberInput label={t('nlaActionEnd')} value={pickedStrip.actionEnd} step={1} onCommit={(actionEnd) => setStrip({ actionEnd })} testId="scene-nla-action-end" />
          </Row>
          <Row label={t('nlaScale')}>
            <NumberInput label={t('nlaScale')} value={pickedStrip.scale} min={0.0001} step={0.1} onCommit={(scale) => setStrip({ scale })} testId="scene-nla-scale" />
            <NumberInput label={t('nlaRepeat')} value={pickedStrip.repeat} min={0.01} step={0.1} onCommit={(repeat) => setStrip({ repeat })} testId="scene-nla-repeat" />
            <Checkbox label={t('nlaReversed')} checked={pickedStrip.reversed} disabled={busy} onChange={(reversed) => setStrip({ reversed })} testId="scene-nla-reversed" />
          </Row>
          <Row label={t('nlaBlendIn')}>
            <NumberInput label={t('nlaBlendIn')} value={pickedStrip.blendIn} min={0} step={1} onCommit={(blendIn) => setStrip({ blendIn })} testId="scene-nla-blend-in" />
            <NumberInput label={t('nlaBlendOut')} value={pickedStrip.blendOut} min={0} step={1} onCommit={(blendOut) => setStrip({ blendOut })} testId="scene-nla-blend-out" />
            <Checkbox label={t('nlaAutoBlend')} checked={pickedStrip.autoBlend} disabled={busy} onChange={(autoBlend) => setStrip({ autoBlend })} testId="scene-nla-auto-blend" />
          </Row>
          <Row label={t('nlaBlending')}>
            <Choice label={t('nlaBlending')} quiet value={pickedStrip.blend} disabled={busy} choices={BLENDS.map((blend) => [blend, t(`nla_${blend}`)] as const)} onChange={(blend) => setStrip({ blend })} testId="scene-nla-blend" />
            <Choice label={t('nlaExtrapolation')} quiet value={pickedStrip.extrapolation} disabled={busy} choices={EXTRAPOLATIONS.map((value) => [value, t(`nla_${value}`)] as const)} onChange={(extrapolation) => setStrip({ extrapolation })} testId="scene-nla-extrapolation" />
          </Row>
          <Row label={t('nlaInfluence')}>
            <Checkbox label={t('nlaFixedInfluence')} checked={pickedStrip.influence != null} disabled={busy} onChange={(fixed) => setStrip({ influence: fixed ? 1 : null })} testId="scene-nla-fixed-influence" />
            {pickedStrip.influence != null ? <NumberInput label={t('nlaInfluence')} value={pickedStrip.influence} min={0} max={1} step={0.05} onCommit={(influence) => setStrip({ influence })} testId="scene-nla-influence" /> : null}
            <Checkbox label={t('nlaMuted')} checked={pickedStrip.muted} disabled={busy} onChange={(muted) => setStrip({ muted })} testId="scene-nla-muted" />
          </Row>
          </> : <Checkbox label={t('nlaMuted')} checked={pickedStrip.muted} disabled={busy} onChange={(muted) => setStrip({ muted })} testId="scene-nla-muted" />}
          <div className="scene-nla-actions">
            {pickedStrip.kind === 'meta' ? <Button size="sm" disabled={busy || tweak != null} onClick={() => { void run('nla.dissolveMeta', { object: id, ...picked }); setPicked(null) }} data-testid="scene-nla-dissolve">{t('nlaDissolve')}</Button> : null}
            <Button size="sm" disabled={busy || tweak != null || !canGroup} onClick={() => { void run('nla.makeMeta', { object: id, track: picked.track, first: picked.strip, last: groupLast }); setPicked(null) }} data-testid="scene-nla-group">{t('nlaGroupNext')}</Button>
            <Button size="sm" disabled={busy || tweak != null || pickedStrip.kind === 'transition' || !nextStrip || nextStrip.kind === 'transition' || nextStrip.start <= pickedStrip.end} onClick={() => run('nla.addTransition', { object: id, ...picked })} data-testid="scene-nla-transition">{t('nlaAddTransition')}</Button>
            <Button size="sm" disabled={busy || tweak != null || pickedStrip.kind !== 'action'} onClick={() => run('nla.tweak', { object: id, track: picked.track, strip: picked.strip })} data-testid="scene-nla-tweak">
              {t('nlaTweak')}
            </Button>
            <Button size="sm" disabled={busy || tweak != null || pickedStrip.kind !== 'action'} onClick={() => run('nla.tweak', { object: id, track: picked.track, strip: picked.strip, upperStack: true })}>
              {t('nlaTweakUpper')}
            </Button>
            <IconButton
              icon={TrashIcon}
              label={t('nlaRemoveStrip')}
              disabled={busy || tweak != null}
              onClick={() => {
                void run('nla.removeStrip', { object: id, track: picked.track, strip: picked.strip })
                setPicked(null)
              }}
              data-testid="scene-nla-remove-strip"
            />
          </div>
        </div>
      ) : nla.tracks.length === 0 ? (
        <p className="kui-muted scene-nla-empty">{t('nlaEmpty')}</p>
      ) : (
        <div className="scene-nla-settings">
          <Row label={t('nlaOwnBlend')}>
            <NumberInput label={t('nlaInfluence')} value={nla.action.influence} min={0} max={1} step={0.05} onCommit={(influence) => run('nla.setAction', { object: id, influence })} testId="scene-nla-own-influence" />
            <Choice label={t('nlaExtrapolation')} quiet value={nla.action.extrapolation} disabled={busy} choices={EXTRAPOLATIONS.map((value) => [value, t(`nla_${value}`)] as const)} onChange={(extrapolation) => run('nla.setAction', { object: id, extrapolation })} testId="scene-nla-own-extrapolation" />
          </Row>
        </div>
      )}
    </section>
  )
}
