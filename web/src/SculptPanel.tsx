import { Button, Checkbox, Choice, PanelSection, useKitEditor, useReadOnly } from '@kition/sdk/editor'
import { useState } from 'react'
import { NumberInput, Row, useRun } from './SidePanel'
import type { Modifier, SceneInfo, SceneObject } from './scene'
import './SculptPanel.css'

const DETAIL_MODES = ['relative', 'constant', 'brush'] as const
const REFINES = ['subdivideCollapse', 'subdivide', 'collapse'] as const

/** Sculpt mode's side panel sections: dynamic topology, and face sets with hiding. */
export function SculptSections({ info }: { info: SceneInfo }) {
  const { phrase: p, t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const sculpt = info.sculpt
  if (info.mode !== 'sculpt' || !sculpt) return null
  const dyntopo = sculpt.dyntopo
  const set = (values: Record<string, unknown>) => run('sculpt.dyntopo', values)
  const atLevel = sculpt.level > 0
  return (
    <>
      <PanelSection title={t('dyntopo')} testId="scene-sculpt-dyntopo-section">
        <div className="scene-sculpt-actions">
          <Checkbox label={t('dyntopo')} checked={dyntopo.enabled} disabled={readOnly || sculpt.levels > 0} onChange={(enabled) => set({ enabled })} testId="scene-sculpt-dyntopo" />
        </div>
        {sculpt.levels > 0 ? <p className="kui-muted scene-sculpt-note">{t('dyntopoMultires')}</p> : null}
        <Row label={t('detailMode')}>
          <Choice label={t('detailMode')} quiet value={dyntopo.mode} disabled={readOnly} choices={DETAIL_MODES.map((mode) => [mode, t(`detail_${mode}`)] as const)} onChange={(mode) => set({ mode })} testId="scene-sculpt-detail-mode" />
        </Row>
        {dyntopo.mode === 'relative' ? (
          <Row label={t('detailSize')}>
            <NumberInput label={t('detailSize')} value={dyntopo.detailSize} min={0.5} max={40} step={0.5} unit="px" onCommit={(detailSize) => set({ detailSize })} testId="scene-sculpt-detail-size" />
          </Row>
        ) : dyntopo.mode === 'constant' ? (
          <Row label={t('detailResolution')}>
            <NumberInput label={t('detailResolution')} value={dyntopo.resolution} min={1} max={10000} step={1} onCommit={(resolution) => set({ resolution })} testId="scene-sculpt-detail-resolution" />
          </Row>
        ) : (
          <Row label={t('detailPercent')}>
            <NumberInput label={t('detailPercent')} value={dyntopo.percent} min={0.5} max={100} step={1} unit="%" onCommit={(percent) => set({ percent })} testId="scene-sculpt-detail-percent" />
          </Row>
        )}
        <Row label={t('refine')}>
          <Choice label={t('refine')} quiet value={dyntopo.refine} disabled={readOnly} choices={REFINES.map((refine) => [refine, t(`refine_${refine}`)] as const)} onChange={(refine) => set({ refine })} />
        </Row>
        <div className="scene-sculpt-actions">
          <Button size="sm" disabled={readOnly || !dyntopo.enabled} onClick={() => run('sculpt.detailFlood', {})} data-testid="scene-sculpt-detail-flood">
            {p('Detail Flood Fill')}
          </Button>
        </div>
      </PanelSection>
      <PanelSection title={t('faceSets')} testId="scene-sculpt-face-sets">
        <p className="kui-muted scene-sculpt-note" data-testid="scene-sculpt-face-set-count">
          {t('faceSetsCount', { count: sculpt.faceSets.length, hidden: sculpt.hiddenFaces })}
        </p>
        <div className="scene-sculpt-actions">
          <Button size="sm" disabled={readOnly || sculpt.masked === 0} onClick={() => run('sculpt.faceSetsFromMask', {})} data-testid="scene-sculpt-face-set-mask">
            {t('faceSetFromMask')}
          </Button>
          <Button size="sm" disabled={readOnly} onClick={() => run('sculpt.faceSetsFromVisible', {})} data-testid="scene-sculpt-face-set-visible">
            {t('faceSetFromVisible')}
          </Button>
          <Button size="sm" disabled={readOnly} onClick={() => run('sculpt.faceSetsFromLooseParts', {})}>
            {t('faceSetLooseParts')}
          </Button>
          <Button size="sm" disabled={readOnly || sculpt.faceSets.length === 0} onClick={() => run('sculpt.faceSetsClear', {})} data-testid="scene-sculpt-face-sets-clear">
            {t('faceSetsClear')}
          </Button>
          <Button size="sm" disabled={readOnly || sculpt.hiddenFaces === 0} onClick={() => run('sculpt.faceSetVisibility', { action: 'showAll' })} data-testid="scene-sculpt-show-all">
            {t('showAllFaces')}
          </Button>
        </div>
        <p className="kui-muted scene-sculpt-note">{t(atLevel ? 'faceSetsAtLevel' : 'faceSetsHint')}</p>
      </PanelSection>
    </>
  )
}

/** The Multires modifier's card body: the levels shown and sculpted, and its operators. */
export function MultiresFields({ object, info, modifier, index }: { object: SceneObject; info: SceneInfo; modifier: Modifier; index: number }) {
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const total = object.mesh?.multiresLevels ?? 0
  const others = info.objects.filter((other) => other.id !== object.id && other.type === 'mesh')
  const [from, setFrom] = useState<string>('')
  const set = (values: Record<string, unknown>) => run('modifier.set', { object: object.id, index, ...values })
  const busy = readOnly || (info.mode !== 'object' && info.mode !== 'sculpt')
  return (
    <>
      <p className="kui-muted scene-sculpt-note" data-testid={`scene-modifier-${index}-total`}>
        {t('multiresTotal', { count: total })}
      </p>
      <Row label={t('multiresViewport')}>
        <NumberInput label={t('multiresViewport')} value={Number(modifier.levels ?? 0)} min={0} max={total} step={1} onCommit={(levels) => set({ levels: Math.round(levels) })} testId={`scene-modifier-${index}-levels`} />
      </Row>
      <Row label={t('multiresSculpt')}>
        <NumberInput label={t('multiresSculpt')} value={Number(modifier.sculptLevels ?? 0)} min={0} max={total} step={1} onCommit={(sculptLevels) => set({ sculptLevels: Math.round(sculptLevels) })} testId={`scene-modifier-${index}-sculptLevels`} />
      </Row>
      <div className="scene-sculpt-actions">
        <Button size="sm" disabled={busy} onClick={() => run('multires.subdivide', { object: object.id })} data-testid="scene-multires-subdivide">
          {t('multiresSubdivide')}
        </Button>
        <Button size="sm" disabled={busy || total >= 6} onClick={() => run('multires.unsubdivide', { object: object.id })} data-testid="scene-multires-unsubdivide">
          {t('multiresUnsubdivide')}
        </Button>
        <Button size="sm" disabled={busy || total !== 0} onClick={() => run('multires.rebuildSubdivisions', { object: object.id })} data-testid="scene-multires-rebuild">
          {t('multiresRebuild')}
        </Button>
        <Button size="sm" disabled={busy || total === 0} onClick={() => run('multires.deleteHigher', { object: object.id })} data-testid="scene-multires-delete-higher">
          {t('multiresDeleteHigher')}
        </Button>
        <Button size="sm" disabled={busy || total === 0} onClick={() => run('multires.applyBase', { object: object.id })} data-testid="scene-multires-apply-base">
          {t('multiresApplyBase')}
        </Button>
      </div>
      <p className="kui-muted scene-sculpt-note">{t('multiresRecoveryHint')}</p>
      <Row label={t('multiresFrom')}>
        <Choice label={t('multiresFrom')} quiet value={from} disabled={busy} choices={[['', '—'], ...others.map((other) => [String(other.id), other.name] as const)]} onChange={setFrom} />
      </Row>
      <div className="scene-sculpt-actions">
        <Button size="sm" disabled={busy || total === 0 || from === ''} onClick={() => run('multires.reshape', { object: object.id, from: Number(from) })}>
          {t('multiresReshape')}
        </Button>
      </div>
    </>
  )
}
