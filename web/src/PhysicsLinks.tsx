import type { ReactNode } from 'react'
import { TrashIcon } from '@kition/tessera-icons/react'
import { Checkbox, Choice, IconButton, useKitEditor, useReadOnly } from '@kition/kit-editor'
import { NumberInput, Row, useRun } from './SidePanel'
import { FIELD_KINDS, JOINT_KINDS, type ConstraintInfo, type FieldInfo } from './PhysicsInfo'
import type { SceneInfo, SceneObject } from './scene'
import './PhysicsLinks.css'

const DEGREES = 180 / Math.PI
const AXES = ['x', 'y', 'z'] as const

/** A titled card with its remove button (the Physics panel's cards look alike). */
function Card({ title, testId, onRemove, children }: { title: string; testId: string; onRemove: () => void; children: ReactNode }) {
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  return (
    <div className="scene-physics-block" data-testid={testId}>
      <div className="scene-physics-block-header">
        <span className="scene-physics-block-title">{title}</span>
        <IconButton icon={TrashIcon} label={`${title}: ${t('physicsRemove')}`} disabled={readOnly} onClick={onRemove} data-testid={`${testId}-remove`} />
      </div>
      {children}
    </div>
  )
}

/** An object's force field: its kind, strength, falloff, and the kind's own settings. */
export function FieldBlock({ object, field }: { object: SceneObject; field: FieldInfo }) {
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const set = (values: Record<string, unknown>) => run('physics.field.set', { objects: [object.id], ...values })
  const number = (label: string, key: string, value: number, min: number, max: number, step: number, unit?: string) => (
    <Row label={label}>
      <NumberInput label={label} value={value} min={min} max={max} step={step} unit={unit} onCommit={(next) => set({ [key]: key === 'seed' ? Math.round(next) : next })} testId={`scene-field-${key}`} />
    </Row>
  )
  return (
    <Card title={t('physicsField')} testId="scene-field" onRemove={() => run('physics.field.remove', { objects: [object.id] })}>
      <Row label={t('physicsType')}>
        <Choice label={t('physicsType')} quiet value={field.kind} disabled={readOnly} choices={FIELD_KINDS.map((kind) => [kind, t(`field_${kind}`)] as const)} onChange={(type) => set({ type })} testId="scene-field-type" />
      </Row>
      {field.kind === 'drag' ? (
        <>
          {number(t('physicsLinearDrag'), 'linearDrag', field.linearDrag, 0, 1000, 0.1)}
          {number(t('physicsQuadraticDrag'), 'quadraticDrag', field.quadraticDrag, 0, 1000, 0.1)}
        </>
      ) : (
        <>
          {number(t('physicsStrength'), 'strength', field.strength, -10000, 10000, 0.1)}
          {field.kind !== 'vortex' ? number(t('physicsFlow'), 'flow', field.flow, 0, 100, 0.1) : null}
        </>
      )}
      {field.kind === 'turbulence' ? (
        <>
          {number(t('physicsNoiseSize'), 'size', field.size, 0.001, 1000, 0.1, 'm')}
          {number(t('physicsSeed'), 'seed', field.seed, 0, 1e6, 1)}
        </>
      ) : null}
      {number(t('physicsFalloffPower'), 'falloffPower', field.falloffPower, 0, 10, 0.5)}
      {number(t('physicsMaxDistance'), 'maxDistance', field.maxDistance ?? 0, 0, 100000, 0.5, 'm')}
    </Card>
  )
}

/** A range control: on or off, and its two ends (shown in `scale` units). */
function RangeRow({ label, value, scale, unit, testId, onChange }: { label: string; value: [number, number] | null | undefined; scale: number; unit: string; testId: string; onChange: (next: [number, number] | null) => void }) {
  const readOnly = useReadOnly()
  const { t } = useKitEditor()
  const ends = value ?? null
  return (
    <div className="scene-physics-range" data-testid={testId}>
      <Checkbox label={label} checked={ends != null} disabled={readOnly} onChange={(on) => onChange(on ? (unit === '°' ? [-Math.PI / 4, Math.PI / 4] : [-1, 1]) : null)} testId={`${testId}-on`} />
      {ends ? (
        <div className="scene-vector scene-vector-2">
          <NumberInput label={t('physicsLower')} value={ends[0] * scale} step={1} unit={unit} max={ends[1] * scale} onCommit={(low) => onChange([low / scale, ends[1]])} testId={`${testId}-lower`} />
          <NumberInput label={t('physicsUpper')} value={ends[1] * scale} step={1} unit={unit} min={ends[0] * scale} onCommit={(high) => onChange([ends[0], high / scale])} testId={`${testId}-upper`} />
        </div>
      ) : null}
    </div>
  )
}

/** A rigid body constraint: its kind, the two bodies it joins, breaking, and its limits. */
export function ConstraintBlock({ object, constraint, info }: { object: SceneObject; constraint: ConstraintInfo; info: SceneInfo }) {
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const set = (values: Record<string, unknown>) => run('physics.constraint.set', { objects: [object.id], ...values })
  const bodies = info.physics.objects.filter((item) => item.rigidBody && item.id !== object.id)
  const named = (id: number) => info.objects.find((other) => other.id === id)?.name ?? `#${id}`
  const pick = (key: 'object1' | 'object2', label: string) => (
    <Row label={label}>
      <Choice
        label={label}
        quiet
        value={String(constraint[key] ?? '')}
        disabled={readOnly}
        choices={[['', t('physicsNoBody')], ...bodies.map((body) => [String(body.id), named(body.id)] as const)]}
        onChange={(chosen) => set({ [key]: chosen === '' ? null : Number(chosen) })}
        testId={`scene-constraint-${key}`}
      />
    </Row>
  )
  // Degrees on screen for angles, the engine's radians in the commands' degrees.
  const toDegrees = (range: [number, number] | null | undefined) => (range ? [range[0] * DEGREES, range[1] * DEGREES] : null)
  return (
    <Card title={t('physicsConstraint')} testId="scene-constraint" onRemove={() => run('physics.constraint.remove', { objects: [object.id] })}>
      <Row label={t('physicsType')}>
        <Choice label={t('physicsType')} quiet value={constraint.kind} disabled={readOnly} choices={JOINT_KINDS.map((kind) => [kind, t(`joint_${kind}`)] as const)} onChange={(type) => set({ type })} testId="scene-constraint-type" />
      </Row>
      {pick('object1', t('physicsFirst'))}
      {pick('object2', t('physicsSecond'))}
      <Row label={t('physicsEnabled')}>
        <Checkbox label={t('physicsEnabled')} checked={constraint.enabled} disabled={readOnly} onChange={(enabled) => set({ enabled })} />
      </Row>
      <Row label={t('physicsNoCollisions')}>
        <Checkbox label={t('physicsNoCollisions')} checked={constraint.disableCollisions} disabled={readOnly} onChange={(disableCollisions) => set({ disableCollisions })} />
      </Row>
      <Row label={t('physicsBreakable')}>
        <Checkbox label={t('physicsBreakable')} checked={constraint.breakable} disabled={readOnly} onChange={(breakable) => set({ breakable })} testId="scene-constraint-breakable" />
      </Row>
      {constraint.breakable ? (
        <Row label={t('physicsThreshold')}>
          <NumberInput label={t('physicsThreshold')} value={constraint.breakingThreshold} min={0} max={1e9} step={0.5} onCommit={(breakingThreshold) => set({ breakingThreshold })} testId="scene-constraint-threshold" />
        </Row>
      ) : null}
      {constraint.kind === 'motor' ? (
        <>
          {(['linear', 'angular'] as const).map((drive) => {
            const enabled = `${drive}Motor` as const
            const velocity = `${drive}TargetVelocity` as const
            const impulse = `${drive}MaxImpulse` as const
            return (
              <div key={drive} className="scene-physics-range">
                <Checkbox label={t(drive === 'linear' ? 'physicsLinearMotor' : 'physicsAngularMotor')} checked={constraint[enabled]} disabled={readOnly} onChange={(on) => set({ [enabled]: on })} testId={`scene-constraint-${enabled}`} />
                {constraint[enabled] ? (
                  <>
                    <Row label={t('physicsTargetVelocity')}>
                      <NumberInput label={t('physicsTargetVelocity')} value={constraint[velocity]} min={-1e5} max={1e5} step={0.1} unit={drive === 'linear' ? 'm/s' : 'rad/s'} onCommit={(value) => set({ [velocity]: value })} testId={`scene-constraint-${velocity}`} />
                    </Row>
                    <Row label={t('physicsMaxImpulse')}>
                      <NumberInput label={t('physicsMaxImpulse')} value={constraint[impulse]} min={0} max={1e9} step={0.1} onCommit={(value) => set({ [impulse]: value })} testId={`scene-constraint-${impulse}`} />
                    </Row>
                  </>
                ) : null}
              </div>
            )
          })}
        </>
      ) : null}
      {constraint.kind === 'hinge' ? (
        <RangeRow label={t('physicsAngleLimit')} value={constraint.angleLimit} scale={DEGREES} unit="°" testId="scene-constraint-angle" onChange={(range) => set({ angleLimit: toDegrees(range) })} />
      ) : null}
      {constraint.kind === 'slider' ? (
        <RangeRow label={t('physicsDistanceLimit')} value={constraint.distanceLimit} scale={1} unit="m" testId="scene-constraint-distance" onChange={(distanceLimit) => set({ distanceLimit })} />
      ) : null}
      {constraint.kind === 'genericSpring' ? (
        <div className="scene-physics-axes">
          {AXES.map((axis, slot) => (
            <div key={axis} className="scene-physics-axis">
              <RangeRow
                label={`${t('physicsMove')} ${axis.toUpperCase()}`}
                value={constraint.linearLimits[slot]}
                scale={1}
                unit="m"
                testId={`scene-constraint-linear-${axis}`}
                onChange={(range) => set({ linearLimits: constraint.linearLimits.map((old, index) => (index === slot ? range : old)) })}
              />
              <RangeRow
                label={`${t('physicsTurn')} ${axis.toUpperCase()}`}
                value={constraint.angularLimits[slot]}
                scale={DEGREES}
                unit="°"
                testId={`scene-constraint-angular-${axis}`}
                onChange={(range) => set({ angularLimits: constraint.angularLimits.map((old, index) => toDegrees(index === slot ? range : old)) })}
              />
              <div className="scene-physics-range">
                <Checkbox
                  label={`${t('physicsSpring')} ${axis.toUpperCase()}`}
                  checked={constraint.linearSprings[slot] != null}
                  disabled={readOnly}
                  onChange={(on) => set({ linearSprings: constraint.linearSprings.map((old, index) => (index === slot ? (on ? 10 : null) : old)) })}
                  testId={`scene-constraint-spring-${axis}`}
                />
                {constraint.linearSprings[slot] != null ? (
                  <NumberInput
                    label={t('physicsStiffness')}
                    value={constraint.linearSprings[slot] ?? 0}
                    min={0}
                    max={1e7}
                    step={1}
                    unit="N/m"
                    onCommit={(stiffness) => set({ linearSprings: constraint.linearSprings.map((old, index) => (index === slot ? stiffness : old)) })}
                    testId={`scene-constraint-stiffness-${axis}`}
                  />
                ) : null}
              </div>
            </div>
          ))}
          <Row label={t('physicsDamping')}>
            <NumberInput label={t('physicsDamping')} value={constraint.springDamping} min={0} max={1e6} step={0.1} onCommit={(springDamping) => set({ springDamping })} testId="scene-constraint-damping" />
          </Row>
        </div>
      ) : null}
    </Card>
  )
}
