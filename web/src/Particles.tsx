import { Button, Checkbox, Choice, IconButton, MenuButton, MenuItem, PanelSection, useKitEditor, useReadOnly } from '@kition/sdk/editor'
import { PlusIcon, TrashIcon } from '@kition/tessera/icons/react'
import { useState } from 'react'
import { NumberInput, Row, VectorRow, useRun } from './SidePanel'
import type { HairDynamicsInfo, ParticleSystemInfo, SceneInfo, SceneObject } from './scene'
import { BoidSettings } from './Boids'
import './Particles.css'

const GROOM_BRUSHES = ['comb', 'cut', 'length', 'puff', 'smooth'] as const

/** The active mesh's particle systems: the list, Add and Remove, and the chosen system's settings. */
export function ParticlesSection({ object, info }: { object: SceneObject; info: SceneInfo }) {
  const { phrase: p, t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const systems = info.particles.filter((system) => system.object === object.id)
  const [chosen, setChosen] = useState(0)
  const index = Math.min(chosen, Math.max(systems.length - 1, 0))
  const system = systems[index] ?? null
  const editing = info.mode === 'particleEdit'
  const busy = readOnly || (info.mode !== 'object' && !editing)
  const add = (kind: 'emitter' | 'hair') => {
    setChosen(systems.length)
    run('particle.add', { object: object.id, kind })
  }
  return (
    <PanelSection
      title={p('Particles')}
      testId="scene-particles"
      actions={
        <MenuButton align="end" trigger={({ open, toggle }) => <IconButton icon={PlusIcon} label={p('Add Particle System')} aria-expanded={open} disabled={busy} onClick={toggle} data-testid="scene-add-particles" />}>
          <MenuItem label={t('particleEmitter')} onSelect={() => add('emitter')} testId="scene-add-particles-emitter" />
          <MenuItem label={t('particleHair')} onSelect={() => add('hair')} testId="scene-add-particles-hair" />
        </MenuButton>
      }
    >
      {system ? (
        <>
          <div className="scene-particles-list" role="listbox" aria-label={p('Particles')}>
            {systems.map((item) => (
              <button
                key={`${item.index}-${item.name}`}
                type="button"
                role="option"
                aria-selected={item.index === index}
                className="scene-particles-item"
                onClick={() => setChosen(item.index)}
                data-testid={`scene-particles-item-${item.index}`}
              >
                <span className="scene-particles-name">{item.name}</span>
                <span className="scene-particles-kind">{t(item.kind === 'hair' ? 'particleHair' : 'particleEmitter')}</span>
              </button>
            ))}
          </div>
          <SystemSettings key={`${object.id}-${index}`} object={object} info={info} system={system} disabled={busy} />
          <div className="scene-particles-actions">
            {system.kind === 'hair' ? (
              <Checkbox
                label={t('particleEditMode')}
                checked={editing}
                disabled={readOnly || (info.mode !== 'object' && !editing)}
                onChange={(on) => run('mode.particleEdit', { on, object: object.id, system: system.index })}
                testId="scene-particle-edit"
              />
            ) : null}
            <Button size="sm" disabled={busy || editing} onClick={() => run('particle.convert', { object: object.id, system: system.index })} data-testid="scene-particles-convert">
              {p('Convert')}
            </Button>
            <IconButton
              icon={TrashIcon}
              label={p('Remove Particle System')}
              disabled={busy || editing}
              onClick={() => {
                setChosen(Math.max(index - 1, 0))
                run('particle.remove', { object: object.id, system: system.index })
              }}
              data-testid="scene-particles-remove"
            />
          </div>
        </>
      ) : (
        <p className="kui-muted scene-empty">{t('noParticles')}</p>
      )}
    </PanelSection>
  )
}

/** One system's settings, as the reference's Particles properties group them. */
function SystemSettings({ object, info, system, disabled }: { object: SceneObject; info: SceneInfo; system: ParticleSystemInfo; disabled: boolean }) {
  const { t } = useKitEditor()
  const run = useRun()
  const set = (values: Record<string, unknown>) => run('particle.set', { object: object.id, system: system.index, ...values })
  const meshes = info.objects.filter((other) => other.type === 'mesh' && other.id !== object.id)
  const colliders = system.colliders ?? []
  const number = (key: keyof ParticleSystemInfo, label: string, options: { min?: number; max?: number; step?: number; unit?: string } = {}) => (
    <div className="scene-particles-field">
      <NumberInput label={label} value={Number(system[key])} min={options.min ?? 0} max={options.max} step={options.step ?? 1} unit={options.unit} onCommit={(value) => set({ [key]: value })} testId={`scene-particles-${key}`} />
    </div>
  )
  return (
    <div className="scene-particles-settings" aria-disabled={disabled}>
      {number('count', system.kind === 'hair' ? t('particleStrands') : t('particleCount'), { max: system.physics === 'boids' ? 2048 : 100000 })}
      {number('seed', t('particleSeed'))}
      <Row label={t('particleEmitFrom')}>
        <Choice label={t('particleEmitFrom')} quiet value={system.emitFrom} disabled={disabled} choices={(system.physics === 'boids' ? ['faces', 'verts'] as const : ['faces', 'verts', 'volume'] as const).map((value) => [value, t(`emitFrom_${value}`)] as const)} onChange={(emitFrom) => set({ emitFrom })} testId="scene-particles-emitFrom" />
      </Row>
      <Row label={t('particleDistribution')}>
        <Choice label={t('particleDistribution')} quiet value={system.distribution} disabled={disabled} choices={(['jittered', 'random'] as const).map((value) => [value, t(`distribution_${value}`)] as const)} onChange={(distribution) => set({ distribution })} />
      </Row>
      <Row label={t('particleDensityGroup')}>
        <Choice
          label={t('particleDensityGroup')}
          quiet
          value={system.densityGroup ?? ''}
          disabled={disabled}
          choices={[['', t('particleDensityNone')], ...object.vertexGroups.map((group) => [group.name, group.name] as const)]}
          onChange={(densityGroup) => set({ densityGroup })}
          testId="scene-particles-density"
        />
      </Row>
      {system.kind === 'emitter' ? (
        <>
          {number('frameStart', t('particleFrameStart'), { min: -100000, max: system.physics === 'boids' ? 100000 : undefined })}
          {number('frameEnd', t('particleFrameEnd'), { min: -100000, max: system.physics === 'boids' ? 100000 : undefined })}
          {number('lifetime', t('particleLifetime'), { min: 0.01, max: system.physics === 'boids' ? 1000 : undefined })}
          {number('lifetimeRandom', t('particleLifetimeRandom'), { max: 1, step: 0.05 })}
          <Row label={t('particlePhysics')}>
            <Choice label={t('particlePhysics')} quiet value={system.physics} disabled={disabled} choices={[["newtonian", t('particleNewtonian')], ["boids", t('particleBoids')]]} onChange={(physics) => set({ physics })} testId="scene-particles-physics" />
          </Row>
          {system.physics === 'boids' ? <BoidSettings object={object} info={info} system={system} disabled={disabled} set={set} /> : <>
          {number('normalVelocity', t('particleNormalVelocity'), { min: -1000, step: 0.1, unit: 'm/s' })}
          <VectorRow label={t('particleObjectVelocity')} value={system.objectVelocity} unit="m/s" step={0.1} onCommit={(objectVelocity) => set({ objectVelocity })} testId="scene-particles-objectVelocity" />
          {number('randomVelocity', t('particleRandomVelocity'), { step: 0.1, unit: 'm/s' })}
          {number('gravity', t('particleGravity'), { min: -100, step: 0.1 })}
          {number('drag', t('particleDrag'), { step: 0.05 })}
          {number('bounce', t('particleBounce'), { max: 1, step: 0.05 })}
          <Checkbox label={t('particleDieOnHit')} checked={system.dieOnCollision} disabled={disabled} onChange={(dieOnCollision) => set({ dieOnCollision })} />
          {meshes.length ? (
            <div className="scene-particles-colliders" aria-label={t('particleColliders')}>
              <span className="scene-row-label">{t('particleColliders')}</span>
              {meshes.map((other) => (
                <Checkbox
                  key={other.id}
                  label={other.name}
                  checked={colliders.includes(other.id)}
                  disabled={disabled}
                  onChange={(on) => set({ colliders: on ? [...colliders, other.id] : colliders.filter((id) => id !== other.id) })}
                  testId={`scene-particles-collider-${other.name}`}
                />
              ))}
            </div>
          ) : null}
          </>}
          <Row label={t('particleRenderAs')}>
            <Choice label={t('particleRenderAs')} quiet value={system.renderAs} disabled={disabled} choices={(['halo', 'object', 'none'] as const).map((value) => [value, t(`renderAs_${value}`)] as const)} onChange={(renderAs) => set({ renderAs })} testId="scene-particles-renderAs" />
          </Row>
          {system.renderAs === 'object' ? (
            <Row label={t('particleInstance')}>
              <Choice
                label={t('particleInstance')}
                quiet
                value={system.instanceObject != null ? String(system.instanceObject) : 'none'}
                disabled={disabled}
                choices={[['none', t('particleNoInstance')] as const, ...meshes.map((other) => [String(other.id), other.name] as const)]}
                onChange={(value) => set({ instanceObject: value === 'none' ? null : Number(value) })}
                testId="scene-particles-instance"
              />
            </Row>
          ) : null}
          {number('size', t('particleSize'), { max: system.physics === 'boids' ? 10 : undefined, step: 0.01, unit: 'm' })}
          {number('sizeRandom', t('particleSizeRandom'), { max: 1, step: 0.05 })}
          {system.renderAs === 'object' ? number('rotationRandom', t('particleRotationRandom'), { max: 1, step: 0.05 }) : null}
        </>
      ) : (
        <>
          {number('hairLength', t('particleHairLength'), { step: 0.05, unit: 'm' })}
          {number('segments', t('particleSegments'), { min: 1, max: 50 })}
          {number('strandWidth', t('particleStrandWidth'), { step: 0.001, unit: 'm' })}
          <Row label={t('particleChildren')}>
            <Choice label={t('particleChildren')} quiet value={system.children} disabled={disabled} choices={(['none', 'simple', 'interpolated'] as const).map((value) => [value, t(`children_${value}`)] as const)} onChange={(children) => set({ children })} testId="scene-particles-children" />
          </Row>
          {system.children !== 'none' ? number('childCount', t('particleChildCount'), { max: 100 }) : null}
          {system.children === 'simple' ? number('childRadius', t('particleChildRadius'), { step: 0.01, unit: 'm' }) : null}
          {system.groomed ? <p className="kui-muted scene-particles-note">{t('particleGroomed')}</p> : null}
          <HairDynamicsSettings system={system} disabled={disabled} set={set} />
        </>
      )}
    </div>
  )
}

/** Hair dynamics: on and off, and its settings (baked with the scene's physics). */
function HairDynamicsSettings({ system, disabled, set }: { system: ParticleSystemInfo; disabled: boolean; set: (values: Record<string, unknown>) => void }) {
  const { t } = useKitEditor()
  const dynamics = system.dynamics
  const number = (key: keyof HairDynamicsInfo, label: string, options: { min?: number; max?: number; step?: number; unit?: string } = {}) => (
    <div className="scene-particles-field">
      <NumberInput
        label={label}
        value={Number(dynamics?.[key] ?? 0)}
        min={options.min ?? 0}
        max={options.max}
        step={options.step ?? 0.1}
        unit={options.unit}
        onCommit={(value) => set({ dynamics: { [key]: key === 'quality' ? Math.round(value) : value } })}
        testId={`scene-hair-${key}`}
      />
    </div>
  )
  return (
    <>
      <Checkbox label={t('hairDynamics')} checked={dynamics != null} disabled={disabled} onChange={(on) => set({ dynamics: on })} testId="scene-hair-dynamics" />
      {dynamics ? (
        <>
          {number('quality', t('hairQuality'), { min: 1, max: 80, step: 1 })}
          {number('mass', t('hairMass'), { min: 0.0001, max: 1000, step: 0.05, unit: 'kg' })}
          {number('stiffness', t('hairStiffness'), { max: 10000 })}
          {number('damping', t('hairDamping'), { max: 1000 })}
          {number('airDamping', t('hairAirDamping'), { max: 10 })}
          {number('pinStiffness', t('hairPinStiffness'), { max: 10000 })}
          <Checkbox label={t('hairCollision')} checked={dynamics.collision} disabled={disabled} onChange={(collision) => set({ dynamics: { collision } })} testId="scene-hair-collision" />
          {dynamics.collision ? number('collisionDistance', t('hairCollisionDistance'), { min: 0.001, max: 1, step: 0.005, unit: 'm' }) : null}
        </>
      ) : null}
    </>
  )
}

/** Particle Edit's tool header over the viewport: the grooming brush and Delete Edit. */
export function ParticleEditControls({ info }: { info: SceneInfo }) {
  const { phrase: p, t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const edit = info.particleEdit
  if (info.mode !== 'particleEdit' || !edit) return null
  const brush = edit.brush
  const set = (values: Record<string, unknown>) => run('particleEdit.brush', values)
  return (
    <div className="scene-weight-paint scene-particle-toolbar" role="toolbar" aria-label={t('particleEditMode')} data-testid="scene-particle-toolbar">
      <Choice label={t('groomBrush')} quiet value={brush.kind} disabled={readOnly} choices={GROOM_BRUSHES.map((kind) => [kind, t(`groom_${kind}`)] as const)} onChange={(kind) => set({ kind })} testId="scene-groom-brush" />
      <span className="scene-weight-field">
        <NumberInput label={t('brushRadius')} value={brush.radius} min={1} max={2000} step={5} unit="px" onCommit={(radius) => set({ radius })} testId="scene-groom-radius" />
      </span>
      <span className="scene-weight-field">
        <NumberInput label={t('strength')} value={brush.strength} min={0} max={1} step={0.05} track onCommit={(strength) => set({ strength })} testId="scene-groom-strength" />
      </span>
      {brush.kind === 'length' || brush.kind === 'puff' ? <Checkbox label={t('groomSubtract')} checked={brush.subtract} disabled={readOnly} onChange={(subtract) => set({ subtract })} testId="scene-groom-subtract" /> : null}
      <Button size="sm" disabled={readOnly} onClick={() => run('particleEdit.reset', {})} data-testid="scene-groom-reset">
        {p('Delete Edit')}
      </Button>
      <Button size="sm" onClick={() => run('mode.particleEdit', { on: false })} data-testid="scene-groom-exit">
        {t('particleEditExit')}
      </Button>
    </div>
  )
}
