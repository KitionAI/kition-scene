import { useSyncExternalStore } from 'react'
import { modalTransform } from './modal'

/**
 * The transform's header (what it does, as Blender's viewport header shows it) and the keys it
 * takes, for the status bar. Each reads only what it shows from the transform, so an answer that
 * changes only the numbers re-renders the header alone, not the editor.
 */
export function ModalHeader() {
  const header = useSyncExternalStore(modalTransform.subscribe, () => (modalTransform.answer.active ? (modalTransform.answer.header ?? '') : ''))
  const kind = useSyncExternalStore(modalTransform.subscribe, () => modalTransform.answer.kind ?? '')
  if (!header) return null
  return (
    <div className="scene-modal-header" role="status" data-testid="scene-modal-header" data-kind={kind}>
      {header}
    </div>
  )
}

/** Whether a transform is under way (or starting). */
export function useModalActive() {
  return useSyncExternalStore(modalTransform.subscribe, () => modalTransform.active)
}

/** The keys the transform takes now. */
export function ModalKeys({ t }: { t: (key: string) => string }) {
  const shape = useSyncExternalStore(modalTransform.subscribe, () => {
    const answer = modalTransform.answer
    return [answer.kind ?? 'move', answer.constraint ? 'axis' : '', answer.numeric ? 'typed' : '', answer.proportional != null ? 'radius' : ''].join('|')
  })
  const [kind, axis, typed, radius] = shape.split('|')
  return (
    <span className="scene-modal-keys" data-testid="scene-modal-keys">
      <span><kbd>LMB ↵</kbd>{t('modalConfirm')}</span>
      <span><kbd>RMB Esc</kbd>{t('modalCancel')}</span>
      <span><kbd>X Y Z</kbd>{t('modalAxis')}</span>
      <span><kbd>⇧X ⇧Y ⇧Z</kbd>{t('modalPlane')}</span>
      {axis ? <span><kbd>C</kbd>{t('modalClear')}</span> : null}
      <span><kbd>0–9 . -</kbd>{t('modalType')}</span>
      {typed ? <span><kbd>Tab</kbd>{t('modalNextField')}</span> : null}
      <span><kbd>Ctrl</kbd>{t('modalSnap')}</span>
      <span><kbd>⇧</kbd>{t('modalPrecision')}</span>
      {kind !== 'move' ? <span><kbd>G</kbd>{t('modalMove')}</span> : null}
      {kind !== 'rotate' ? <span><kbd>R</kbd>{t('modalRotate')}</span> : null}
      {kind !== 'scale' ? <span><kbd>S</kbd>{t('modalScale')}</span> : null}
      {radius ? <span><kbd>Wheel</kbd>{t('modalRadius')}</span> : null}
      <span><kbd>MMB</kbd>{t('modalNavigate')}</span>
    </span>
  )
}
