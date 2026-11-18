import { useCallback, useEffect, useRef, useState } from 'react'
import { Button, Checkbox, Dialog, NumberField, Segmented, toast, useKitEditor, type HostConnection } from '@kition/kit-editor'
import { engineManifest } from './engine'
import { MAX_TRACE_WORKERS, TracePool, autoTraceWorkers, traceWorkersAvailable } from './trace/tracePool'

/** How a render is made: the rasterizer at once, or the path tracer a little at a time. */
export type RenderEngine = 'fast' | 'path'

type Picture = { url: string; bytes: Uint8Array }
type Status = {
  state: 'idle' | 'running' | 'done' | 'failed' | 'cancelled'
  fraction: number
  samples: number
  target: number
  message?: string
  /** How long the render took, once done (milliseconds), and the workers that made it (1: the engine's own). */
  elapsed?: number
  workers?: number
}
type TraceProgress = { samples: number; target: number; fraction: number; done: boolean }

/** The worker time one path tracing step takes, and how often the picture is refreshed. */
const STEP_MS = 60
const PREVIEW_MS = 400

function decodeBase64(text: string) {
  const binary = atob(text)
  const bytes = new Uint8Array(binary.length)
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index)
  return bytes
}

/**
 * The render dialog: a picture through the scene camera, with the fast rasterizer (shadows and
 * ambient occlusion) or the path tracer (bounced light, soft shadows), which renders in short
 * steps and shows the picture as it clears: split across workers (each with the engine and the
 * scene, see `trace/`) when the machine has the cores, else in the engine's own worker.
 */
export function RenderDialog({
  host,
  base,
  aspect,
  compositing,
  onClose,
}: {
  host: HostConnection
  base: string
  aspect: string | undefined
  /** Whether renders pass through the scene's compositing group; null when it has none. */
  compositing: boolean | null
  onClose: () => void
}) {
  const editor = useKitEditor()
  const { t } = editor
  const [engine, setEngine] = useState<RenderEngine>('fast')
  const [samples, setSamples] = useState(64)
  const [denoise, setDenoise] = useState(true)
  const [workers, setWorkers] = useState(autoTraceWorkers)
  const [picture, setPicture] = useState<Picture | null>(null)
  const [status, setStatus] = useState<Status>({ state: 'idle', fraction: 0, samples: 0, target: 0 })
  // The render under way; a newer render or closing the dialog stops it.
  const run = useRef(0)
  const pool = useRef<TracePool | null>(null)
  const latest = useRef(editor)
  latest.current = editor

  const show = useCallback((bytes: Uint8Array) => {
    const url = URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type: 'image/png' }))
    setPicture((current) => {
      if (current) URL.revokeObjectURL(current.url)
      return { url, bytes }
    })
  }, [])

  const render = useCallback(
    async (how: RenderEngine, count: number, smooth: boolean, split = 1) => {
      const mine = ++run.current
      pool.current?.stop()
      pool.current = null
      const doc = latest.current.session.doc
      if (!doc) return
      const started = performance.now()
      const report = (progress: TraceProgress, used: number) =>
        setStatus({
          state: progress.done ? 'done' : 'running',
          fraction: progress.fraction,
          samples: progress.samples,
          target: progress.target,
          workers: used,
          elapsed: progress.done ? Math.round(performance.now() - started) : undefined,
        })
      setStatus({ state: 'running', fraction: 0, samples: 0, target: how === 'path' ? count : 1 })
      try {
        if (how === 'fast') {
          const answer = JSON.parse(await doc.execute('render.image', { samples: 2 })) as { base64: string }
          if (mine !== run.current) return
          show(decodeBase64(answer.base64))
          setStatus({ state: 'done', fraction: 1, samples: 1, target: 1 })
          return
        }
        if (split > 1 && traceWorkersAvailable()) {
          try {
            // Workers open the scene as it is now and trace interleaved rows; the engine merges.
            const snapshot = await doc.invoke<Uint8Array>('traceSnapshot')
            await doc.execute('render.trace.start', { samples: count, denoise: smooth, collect: true })
            if (mine !== run.current) return
            const workersPool = new TracePool(split)
            pool.current = workersPool
            const manifest = engineManifest()
            const kitEngine = { base: manifest.base ?? '', module: manifest.module ?? 'kition_scene_kit', documentClass: manifest.documentClass ?? 'SceneDocument' }
            let shown = 0
            await workersPool.run(kitEngine, snapshot, { samples: count, denoise: smooth }, PREVIEW_MS, async (sums) => {
              if (mine !== run.current) return
              const progress = JSON.parse(await doc.invoke<string>('traceMerge', sums)) as TraceProgress
              if (mine !== run.current) return
              // The picture first, so "done" never shows beside an older one.
              if (progress.done || performance.now() - shown > PREVIEW_MS) {
                shown = performance.now()
                const bytes = await doc.invoke<Uint8Array>('traceImage')
                if (mine !== run.current) return
                show(bytes)
              }
              report(progress, split)
            })
            return
          } catch (error) {
            if (mine !== run.current) return
            // Workers could not run here: the engine's own tracer renders instead.
            console.warn('trace workers failed; rendering in the engine', error)
            pool.current?.stop()
            pool.current = null
          }
        }
        await doc.execute('render.trace.start', { samples: count, denoise: smooth })
        let shown = 0
        for (;;) {
          if (mine !== run.current) return
          const progress = JSON.parse(await doc.invoke<string>('traceStep', STEP_MS)) as TraceProgress
          if (mine !== run.current) return
          if (progress.done || performance.now() - shown > PREVIEW_MS) {
            shown = performance.now()
            const bytes = await doc.invoke<Uint8Array>('traceImage')
            if (mine !== run.current) return
            show(bytes)
          }
          report(progress, 1)
          if (progress.done) return
        }
      } catch (error) {
        if (mine === run.current) setStatus({ state: 'failed', fraction: 0, samples: 0, target: 0, message: error instanceof Error ? error.message : String(error) })
      }
    },
    [show],
  )

  const cancel = useCallback(() => {
    run.current += 1
    pool.current?.stop()
    pool.current = null
    setStatus((current) => (current.state === 'running' ? { ...current, state: 'cancelled' } : current))
    void latest.current.session.doc?.execute('render.trace.cancel', {}).catch(() => {})
  }, [])

  // A fast render as the dialog opens; nothing keeps running once it closes.
  useEffect(() => {
    void render('fast', 1, false)
    return () => {
      run.current += 1
      pool.current?.stop()
      pool.current = null
      void latest.current.session.doc?.execute('render.trace.cancel', {}).catch(() => {})
    }
  }, [render])
  useEffect(() => () => {
    if (picture) URL.revokeObjectURL(picture.url)
  }, [picture])

  const running = status.state === 'running'
  return (
    <Dialog
      title={t('renderTitle')}
      testId="scene-render-dialog"
      onClose={onClose}
      actions={
        <>
          {running ? (
            <Button onClick={cancel} data-testid="scene-render-cancel">
              {t('cancel')}
            </Button>
          ) : (
            <Button onClick={() => void render(engine, samples, denoise, workers)} data-testid="scene-render-start">
              {t('renderAgain')}
            </Button>
          )}
          <Button
            variant="primary"
            disabled={!picture || running}
            onClick={async () => {
              if (!picture) return
              try {
                const written = await host.exportFile(`${base}.png`, picture.bytes)
                toast('success', t('saved', { name: written.split('/').pop() ?? written }))
              } catch (error) {
                editor.report(t('exportFailed'), error)
              }
            }}
            data-testid="scene-render-save"
          >
            {t('saveImage')}
          </Button>
          <Button onClick={onClose}>{t('close')}</Button>
        </>
      }
    >
      <div className="scene-render-options">
        <Segmented
          label={t('renderEngine')}
          value={engine}
          segments={[
            { value: 'fast', label: t('engineFast') },
            { value: 'path', label: t('enginePath') },
          ]}
          onChange={(value) => {
            const next = value as RenderEngine
            setEngine(next)
            void render(next, samples, denoise, workers)
          }}
          testId="scene-render-engine"
        />
        {engine === 'path' ? (
          <>
            <span className="scene-render-samples">
              <NumberField label={t('traceSamples')} value={samples} min={1} max={4096} step={1} onChange={(value, final) => final && setSamples(Math.round(value))} testId="scene-render-samples" />
            </span>
            <span className="scene-render-samples">
              <NumberField
                label={t('traceWorkers')}
                value={workers}
                min={1}
                max={MAX_TRACE_WORKERS}
                step={1}
                onChange={(value, final) => final && setWorkers(Math.max(1, Math.min(MAX_TRACE_WORKERS, Math.round(value))))}
                testId="scene-render-workers"
              />
            </span>
            <Checkbox label={t('denoise')} checked={denoise} onChange={setDenoise} />
          </>
        ) : null}
        {compositing != null ? (
          <Checkbox
            label={t('compositing')}
            checked={compositing}
            disabled={running}
            onChange={(on) => void editor.run('compositor.set', { compositing: on }).then(() => render(engine, samples, denoise, workers))}
            testId="scene-render-compositing"
          />
        ) : null}
      </div>
      <div className="scene-render" style={aspect ? { aspectRatio: aspect } : undefined}>
        {picture ? (
          <img src={picture.url} alt={t('renderTitle')} data-testid="scene-render-image" />
        ) : (
          <span className="kui-muted">{status.state === 'failed' ? `${t('renderFailed')}: ${status.message ?? ''}` : t('rendering')}</span>
        )}
      </div>
      <div
        className="scene-render-progress"
        data-state={status.state}
        data-elapsed-ms={status.elapsed}
        data-workers={status.workers}
        data-testid="scene-render-progress"
      >
        <div className="scene-render-bar">
          <div style={{ width: `${Math.round(status.fraction * 100)}%` }} />
        </div>
        <span className="tabular-nums">
          {status.state === 'failed'
            ? t('renderFailed')
            : status.state === 'cancelled'
              ? t('renderCancelled')
              : engine === 'path'
                ? t('samplesProgress', { done: status.samples, target: status.target })
                : status.state === 'done'
                  ? t('renderDone')
                  : t('rendering')}
        </span>
      </div>
    </Dialog>
  )
}
