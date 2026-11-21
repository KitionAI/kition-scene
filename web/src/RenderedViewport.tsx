import { useEffect, useRef, useState } from 'react'
import { useKitEditor } from '@kition/sdk/editor'

type Progress = { samples: number; target: number; done: boolean; generation: number }

/** One worker request in flight. Cleanup cancels scheduling, never publishes a stale reply,
 * and lets queued edits/navigation run before the next cooperative tracing step. */
export function RenderedViewport({ active }: { active: boolean }) {
  const editor = useKitEditor()
  const { doc, change } = editor.session
  const [progress, setProgress] = useState<Progress | null>(null)
  const latest = useRef(editor)
  latest.current = editor
  const flight = useRef<Promise<unknown> | null>(null)
  useEffect(() => {
    if (!active || !doc) {
      setProgress(null)
      return
    }
    setProgress(null)
    let cancelled = false
    let timer = 0
    // Engine commands update this snapshot before React commits the new shading prop.
    // Treat that interval as cancellation too, so an obsolete step cannot publish or toast.
    const current = () => !cancelled && latest.current.session.doc === doc && (doc.info.inspected.view as { shading?: unknown } | undefined)?.shading === 'rendered'
    const step = async () => {
      await flight.current?.catch(() => undefined)
      if (!current()) return
      try {
        const request = latest.current.session.run('view.renderedStep', { budgetMs: 8 }, null, { refresh: 'repaint' })
        flight.current = request
        const answer = await request
        if (flight.current === request) flight.current = null
        if (!current() || answer == null) return
        const next = JSON.parse(answer) as Progress
        setProgress(next)
        if (!next.done) timer = window.setTimeout(() => void step(), 24)
      } catch (error) {
        flight.current = null
        if (current()) latest.current.report(latest.current.t('rendered'), error)
      }
    }
    timer = window.setTimeout(() => void step(), 100)
    return () => {
      cancelled = true
      window.clearTimeout(timer)
    }
  }, [active, doc, change])
  if (!active) return null
  return (
    <div className="scene-rendered-progress" role="status" data-testid="scene-rendered-progress" data-samples={progress?.samples ?? 0} data-done={progress?.done ?? false} data-generation={progress?.generation ?? 0}>
      {progress ? editor.t('viewportSamples', { samples: progress.samples, target: progress.target }) : editor.t('viewportPreparing')}
    </div>
  )
}
