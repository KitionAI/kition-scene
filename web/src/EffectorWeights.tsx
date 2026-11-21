import { useKitEditor, useReadOnly } from '@kition/sdk/editor'
import { NumberInput, Row } from './SidePanel'
import type { EffectorWeightsInfo } from './PhysicsInfo'

/** One simulated object's influence controls; updates merge with its other weights. */
export function EffectorWeightsControls({ weights, system, set }: { weights: EffectorWeightsInfo; system: 'cloth' | 'soft'; set: (weights: Partial<EffectorWeightsInfo>) => void }) {
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  return <fieldset className="scene-effector-weights" disabled={readOnly} data-testid={`scene-${system}-weights`}>
    <legend>{t('effectorWeights')}</legend>
    <p className="kui-muted">{t('effectorHint')}</p>
    {(['all', 'gravity', 'force', 'wind', 'vortex', 'turbulence', 'drag'] as const).map(key => {
      const label = t(key === 'all' ? 'effectorAll' : key === 'gravity' ? 'physicsGravity' : `field_${key}`)
      return <Row key={key} label={label}>
        <NumberInput label={label} value={weights[key]} min={-200} max={200} step={0.1} onCommit={value => set({ [key]: value })} testId={`scene-${system}-weight-${key}`} />
      </Row>
    })}
  </fieldset>
}
