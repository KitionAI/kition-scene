import { useEffect, useRef, type MutableRefObject } from 'react'
import { readViewState, type HostConnection, type useKitSession } from '@kition/sdk/editor'

type Session = ReturnType<typeof useKitSession>
/** The engine's `view.state` answer: the person's view, selection, current frame and UV options. */
export type SceneViewState = Record<string, unknown>

const parse = (answer: unknown) => (typeof answer === 'string' ? (JSON.parse(answer) as unknown) : answer)

/**
 * The person's view of the scene kept with the host (Kit UI protocol 7n, capability `view-state`),
 * never in the file: when a document opens, the view state the host kept for it is put back
 * (`view.restore`; a document reopened in this page keeps the view it had); after every change
 * `kept` follows the engine's `view.state`, which `useKitSession`'s `viewState` option writes to
 * the host after each save. Without the capability nothing is read or kept between sessions.
 */
export function useSceneViewState(host: HostConnection, session: Session, kept: MutableRefObject<SceneViewState | null>) {
  const { doc } = session
  const revision = doc?.info.revision ?? null
  const latest = useRef(session)
  latest.current = session
  const restored = useRef<unknown>(null)
  const path = host.init.document.path

  useEffect(() => {
    if (!doc || restored.current === doc) return
    let current = true
    void (async () => {
      const carried = kept.current
      const state = carried ?? (await readViewState(host.files, path))?.state ?? null
      try {
        if (current && state) await latest.current.run('view.restore', { state }, null, { refresh: 'repaint' })
      } catch (error) {
        console.warn('[scene] could not restore the view', error)
      } finally {
        if (current) restored.current = doc
      }
    })()
    return () => {
      current = false
    }
  }, [doc, host, kept, path])

  useEffect(() => {
    if (!doc) return
    let current = true
    void (async () => {
      try {
        const answer = parse(await latest.current.run('view.state', {}, null, { refresh: 'none' })) as { state?: SceneViewState } | null
        // Not before the open's view state is back: the default view would replace it.
        if (current && restored.current === doc && answer?.state) kept.current = answer.state
      } catch {
        // The next change asks again.
      }
    })()
    return () => {
      current = false
    }
  }, [doc, revision, kept])
}
