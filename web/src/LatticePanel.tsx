import { Button, PanelSection, Segmented, useKitEditor, useReadOnly } from '@kition/kit-editor'
import { NumberInput, Row, useRun } from './SidePanel'
import type { SceneInfo, SceneObject } from './scene'

/** A lattice's grid, how it blends, its edit mode (Tab), and putting its points back. */
export function LatticeSection({ object, info }: { object: SceneObject; info: SceneInfo }) {
  const { phrase: p, t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const lattice = object.lattice
  if (!lattice) return null
  const editing = info.mode === 'latticeEdit' && info.editing === object.id
  const set = (params: Record<string, unknown>) => run('lattice.set', { object: object.id, ...params })
  const side = (slot: number, value: number) => set({ resolution: lattice.resolution.map((current, other) => (other === slot ? value : current)) })
  return (
    <PanelSection title={p('Lattice')} testId="scene-lattice">
      <Row label={t('shapeMode')}>
        <Segmented
          label={t('shapeMode')}
          value={editing ? 'edit' : 'object'}
          disabled={readOnly || (info.mode !== 'object' && !editing)}
          segments={[
            { value: 'object', label: t('modeObject') },
            { value: 'edit', label: t('modeEdit') },
          ]}
          onChange={(value) => {
            if ((value === 'edit') !== editing) run('mode.shape', {})
          }}
        />
      </Row>
      <Row label={t('latticeResolution')}>
        <div className="scene-vector">
          {(['U', 'V', 'W'] as const).map((axis, slot) => (
            <NumberInput key={axis} label={axis} value={lattice.resolution[slot]} min={1} max={64} step={1} onCommit={(value) => side(slot, value)} testId={`scene-lattice-${axis.toLowerCase()}`} />
          ))}
        </div>
      </Row>
      <Row label={t('latticeInterpolation')}>
        <Segmented
          label={t('latticeInterpolation')}
          value={lattice.interpolation}
          disabled={readOnly}
          segments={[
            { value: 'bspline', label: t('interpolation_bspline') },
            { value: 'linear', label: t('interpolation_linear') },
          ]}
          onChange={(interpolation) => set({ interpolation })}
        />
      </Row>
      <Row label={t('curveShape')}>
        <span className="scene-readout tabular-nums" data-testid="scene-lattice-stats">
          {t('latticeStats', { points: lattice.pointCount, moved: lattice.moved.length })}
        </span>
      </Row>
      {editing ? <p className="kui-muted scene-text-hint">{t('latticeEditHint')}</p> : null}
      <div className="scene-bone-actions">
        <Button size="sm" disabled={readOnly || lattice.moved.length === 0} onClick={() => set({ reset: true })} data-testid="scene-lattice-reset">
          {t('latticeReset')}
        </Button>
      </div>
    </PanelSection>
  )
}
