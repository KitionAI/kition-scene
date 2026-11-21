import { Button, Choice, IconButton, useKitEditor } from '@kition/sdk/editor'
import { TrashIcon } from '@kition/tessera/icons/react'
import { NumberInput, Row } from './SidePanel'
import type { BoidRule, BoidsInfo, ParticleSystemInfo, SceneInfo, SceneObject } from './scene'

/** Airborne averaged boid rules. The worker owns evaluation, validation and history. */
export function BoidSettings({ object, info, system, disabled, set }: { object: SceneObject; info: SceneInfo; system: ParticleSystemInfo; disabled: boolean; set: (values: Record<string, unknown>) => void }) {
  const { t } = useKitEditor()
  const boids = system.boids
  const rules = boids.rules
  const change = (patch: Partial<BoidsInfo>) => set({ boids: patch })
  const changeRule = (index: number, rule: BoidRule) => change({ rules: rules.map((old, at) => at === index ? rule : old) })
  const fields: [keyof Omit<BoidsInfo, 'rules'>, number][] = [['airSpeedMax', 100], ['airAccMax', 1], ['airAngularMax', 1], ['airPersonalSpace', 10]]
  return <fieldset disabled={disabled} className="scene-boids" data-testid="scene-boids">
    <p className="kui-muted scene-particles-note">{t('boidAverageHint')}</p>
    {fields.map(([key, max]) => <NumberInput key={key} label={t(`boid_${key}`)} value={boids[key]} min={0} max={max} step={key === 'airSpeedMax' ? 1 : 0.1} onCommit={value => change({ [key]: value })} testId={`scene-boids-${key}`} />)}
    {rules.map((rule, index) => <div key={index} className="scene-boid-rule" data-testid={`scene-boid-rule-${index}`}>
      <Row label={t('boidRule')}>
        <Choice label={t('boidRule')} quiet value={rule.kind} disabled={disabled} choices={(['goal', 'avoid', 'separate', 'flock'] as const).map(kind => [kind, t(`boid_${kind}`)] as const)} onChange={kind => {
          const nextKind = kind as BoidRule['kind']
          changeRule(index, nextKind === 'goal' || nextKind === 'avoid' ? { ...rule, kind: nextKind } : { kind: nextKind })
        }} testId={`scene-boid-kind-${index}`} />
        <IconButton icon={TrashIcon} label={t('boidRemoveRule')} disabled={disabled} onClick={() => change({ rules: rules.filter((_, at) => at !== index) })} data-testid={`scene-boid-remove-${index}`} />
      </Row>
      {rule.kind === 'goal' || rule.kind === 'avoid' ? <Row label={t('boidTarget')}>
        <Choice label={t('boidTarget')} quiet disabled={disabled} value={rule.target == null ? '' : String(rule.target)} choices={[["", t('boidNoTarget')], ...info.objects.filter(other => other.id !== object.id).map(other => [String(other.id), other.name] as const)]} onChange={value => changeRule(index, { ...rule, target: value ? Number(value) : undefined })} testId={`scene-boid-target-${index}`} />
      </Row> : null}
    </div>)}
    <Button size="sm" disabled={disabled || rules.length >= 8} onClick={() => change({ rules: [...rules, { kind: 'goal' }] })} data-testid="scene-boid-add">{t('boidAddRule')}</Button>
  </fieldset>
}
