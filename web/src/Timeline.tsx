import { useEffect, useRef, useState } from 'react'
import { KeyframeIcon, KeyframeNextIcon, KeyframePreviousIcon, PauseIcon, PlayIcon, SkipBackIcon, SkipForwardIcon, TrashIcon } from '@kition/tessera/icons/react'
import { IconButton, NumberField, useKitEditor, useReadOnly } from '@kition/sdk/editor'
import type { AnimationInfo } from './animation'
import { KeyingSetChoice } from './AnimationPanel'
import type { SceneInfo } from './scene'

/** Pixels between the ruler's edges and its first and last frames. */
const INSET = 8

/** Ruler ticks at a step that leaves room for their numbers. */
function tickStep(frames: number, width: number) {
  const steps = [1, 2, 5, 10, 20, 25, 50, 100, 200, 250, 500, 1000, 2500, 5000, 10000]
  return steps.find((step) => (width / frames) * step >= 44) ?? 10000
}

/**
 * The timeline under the viewport: play and pause, stepping through frames and keys, the
 * frame range, inserting and deleting keys on the selection, and a ruler that shows the
 * selection's keys and scrubs. Playback asks the engine for one frame at a time and skips
 * frames it cannot keep up with, as a player does.
 */
export function Timeline({ animation, hasSelection, info }: { animation: AnimationInfo; hasSelection: boolean; info: SceneInfo | null }) {
  const editor = useKitEditor()
  const { t } = editor
  const readOnly = useReadOnly()
  const ruler = useRef<HTMLDivElement>(null)
  const [width, setWidth] = useState(600)
  const [playing, setPlaying] = useState(false)
  const latest = useRef({ animation, editor })
  latest.current = { animation, editor }
  const busy = useRef(false)
  const wanted = useRef<number | null>(null)

  // One frame request in flight; a newer request replaces the one waiting.
  const goTo = (frame: number) => {
    wanted.current = Math.round(frame)
    if (busy.current) return
    busy.current = true
    void (async () => {
      try {
        while (wanted.current != null) {
          const next = wanted.current
          wanted.current = null
          if (next !== latest.current.animation.frame) await latest.current.editor.run('frame.set', { frame: next })
        }
      } finally {
        busy.current = false
      }
    })()
  }

  useEffect(() => {
    const element = ruler.current
    if (!element) return
    const observer = new ResizeObserver(() => setWidth(element.clientWidth || 600))
    observer.observe(element)
    setWidth(element.clientWidth || 600)
    return () => observer.disconnect()
  }, [])

  useEffect(() => {
    if (!playing) return
    let handle = 0
    const begin = performance.now()
    const from = latest.current.animation.frame
    const tick = (now: number) => {
      const { start, end, fps } = latest.current.animation
      const span = Math.max(1, end - start + 1)
      const elapsed = Math.floor(((now - begin) / 1000) * fps)
      const frame = start + ((from - start + elapsed) % span + span) % span
      if (!busy.current) goTo(frame)
      handle = requestAnimationFrame(tick)
    }
    handle = requestAnimationFrame(tick)
    return () => cancelAnimationFrame(handle)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [playing])

  const { start, end, frame, fps } = animation
  const frames = Math.max(1, end - start)
  // The range sits inside an inset, so the first and last keys are not cut off.
  const inner = Math.max(1, width - 2 * INSET)
  const x = (value: number) => INSET + ((value - start) / frames) * inner
  const frameAt = (clientX: number) => {
    const bounds = ruler.current?.getBoundingClientRect()
    if (!bounds) return frame
    return Math.min(end, Math.max(start, Math.round(start + ((clientX - bounds.left - INSET) / inner) * frames)))
  }
  const step = tickStep(frames, inner)
  const ticks: number[] = []
  for (let tick = Math.ceil(start / step) * step; tick <= end; tick += step) ticks.push(tick)
  const keyed = animation.selectedKeyFrames.includes(frame)

  return (
    <div className="scene-timeline" data-testid="scene-timeline">
      <div className="scene-timeline-controls">
        <IconButton icon={SkipBackIcon} label={t('jumpStart')} onClick={() => void editor.run('frame.start', {})} />
        <IconButton icon={KeyframePreviousIcon} label={t('previousKey')} disabled={animation.keyFrames.length === 0} onClick={() => void editor.run('anim.previousKey', {})} />
        <IconButton icon={playing ? PauseIcon : PlayIcon} label={playing ? t('pause') : t('play')} onClick={() => setPlaying((on) => !on)} data-testid="scene-timeline-play" />
        <IconButton icon={KeyframeNextIcon} label={t('nextKey')} disabled={animation.keyFrames.length === 0} onClick={() => void editor.run('anim.nextKey', {})} />
        <IconButton icon={SkipForwardIcon} label={t('jumpEnd')} onClick={() => void editor.run('frame.end', {})} />
        <span className="scene-timeline-field scene-timeline-frame">
          <NumberField label={t('frame')} value={frame} step={1} min={-1048574} max={1048574} testId="scene-timeline-frame" onChange={(value, final) => final && goTo(value)} />
        </span>
        <IconButton
          icon={KeyframeIcon}
          label={`${t('insertKey')} (I)`}
          disabled={readOnly || !hasSelection}
          onClick={() => void editor.run('anim.insertKey', {})}
          data-testid="scene-timeline-insert"
          data-keyed={keyed || undefined}
        />
        {info ? (
          <span className="scene-timeline-keying">
            <KeyingSetChoice info={info} />
          </span>
        ) : null}
        <IconButton icon={TrashIcon} label={`${t('deleteKey')} (Alt+I)`} disabled={readOnly || !hasSelection || !keyed} onClick={() => void editor.run('anim.deleteKey', {})} data-testid="scene-timeline-delete" />
      </div>
      <div
        ref={ruler}
        className="scene-timeline-ruler"
        data-testid="scene-timeline-ruler"
        onPointerDown={(event) => {
          event.currentTarget.setPointerCapture(event.pointerId)
          setPlaying(false)
          goTo(frameAt(event.clientX))
        }}
        onPointerMove={(event) => {
          if (event.buttons & 1) goTo(frameAt(event.clientX))
        }}
      >
        {ticks.map((tick) => (
          <span key={tick} className="scene-timeline-tick" style={{ left: x(tick) }}>
            {tick}
          </span>
        ))}
        {animation.selectedKeyFrames
          .filter((key) => key >= start && key <= end)
          .map((key) => (
            <span key={key} className="scene-timeline-key" data-current={Math.abs(key - frame) < 1e-4 || undefined} style={{ left: x(key) }} title={`${t('frame')} ${key}`} />
          ))}
        <span className="scene-timeline-playhead" style={{ left: x(Math.min(end, Math.max(start, frame))) }} data-testid="scene-timeline-playhead">
          <span className="scene-timeline-playhead-label">{frame}</span>
        </span>
      </div>
      <div className="scene-timeline-range">
        <span className="scene-timeline-field">
          <NumberField label={t('start')} value={start} step={1} min={-1048574} max={end} testId="scene-timeline-start" onChange={(value, final) => final && value !== start && void editor.run('frame.range', { start: value })} />
        </span>
        <span className="scene-timeline-field">
          <NumberField label={t('end')} value={end} step={1} min={start} max={1048574} testId="scene-timeline-end" onChange={(value, final) => final && value !== end && void editor.run('frame.range', { end: value })} />
        </span>
        <span className="scene-timeline-fps kui-muted">{t('fpsValue', { fps: Math.round(fps * 100) / 100 })}</span>
      </div>
    </div>
  )
}
