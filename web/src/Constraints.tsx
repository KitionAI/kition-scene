import type { ReactNode } from 'react'
import { ArrowDownIcon, ArrowUpIcon, CheckIcon, PlusIcon, TrashIcon } from '@kition/tessera/icons/react'
import { Button, Checkbox, Choice, IconButton, MenuButton, MenuItem, PanelSection, useKitEditor, useReadOnly } from '@kition/sdk/editor'
import { moreConstraintFields, SpaceRows } from './ConstraintFields'
import { NumberInput, Row, VectorRow, useRun } from './SidePanel'
import type { Constraint, SceneInfo, SceneObject, Vec3 } from './scene'

/** The constraints an object or a bone can have, as the engine names them. */
const OBJECT_TYPES = [
  ['trackTo', 'Track To'],
  ['dampedTrack', 'Damped Track'],
  ['copyLocation', 'Copy Location'],
  ['copyRotation', 'Copy Rotation'],
  ['limitRotation', 'Limit Rotation'],
  ['childOf', 'Child Of'],
  ['stretchTo', 'Stretch To'],
  ['floor', 'Floor'],
  ['transform', 'Transformation'],
  ['action', 'Action'],
  ['armature', 'Armature'],
] as const
const BONE_TYPES = [['ik', 'Inverse Kinematics'], ['splineIk', 'Spline IK'], ...OBJECT_TYPES.filter(([type]) => type !== 'childOf')] as const

const TRACK_AXES = ['x', 'y', 'z', '-x', '-y', '-z'] as const
const axisLabel = (axis: string) => axis.toUpperCase().replace('-', '−')
const flags = (value: unknown, fallback: boolean[]) => (Array.isArray(value) && value.length === 3 ? value.map(Boolean) : fallback)

/** Three axis checkboxes (X, Y, Z) that set one list. */
function AxesRow({ label, value, onChange }: { label: string; value: boolean[]; onChange: (value: boolean[]) => void }) {
  const readOnly = useReadOnly()
  return (
    <Row label={label}>
      <div className="scene-checks">
        {['X', 'Y', 'Z'].map((axis, index) => (
          <Checkbox key={axis} label={axis} checked={value[index] === true} disabled={readOnly} onChange={(on) => onChange(value.map((old, at) => (at === index ? on : old)))} />
        ))}
      </div>
    </Row>
  )
}

/** One constraint of a stack: on/off, name, order, apply, remove, influence, target, and its own settings. */
function ConstraintCard({ info, owner, constraint, index, count }: { info: SceneInfo; owner: { object: number; bone?: string }; constraint: Constraint; index: number; count: number }) {
  const { phrase: p, t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const target = { ...owner, index }
  const set = (values: Record<string, unknown>) => run('constraint.set', { ...target, ...values })
  const testId = `scene-constraint-${index}`
  const targets = info.objects.filter((item) => (item.id !== owner.object || owner.bone) && (constraint.type !== 'splineIk' || item.type === 'curve'))
  const pick = (slot: 'target' | 'pole', label: string) => {
    const chosen = constraint[slot]
    const armature = info.objects.find((item) => item.id === chosen?.object && item.type === 'armature')
    return (
      <>
        <Row label={label}>
          <Choice
            label={label}
            quiet
            value={chosen ? String(chosen.object) : ''}
            disabled={readOnly}
            choices={[['', t('noTarget')], ...targets.map((item) => [String(item.id), item.name] as const)]}
            onChange={(value) => set({ [slot]: value ? Number(value) : null })}
            testId={`${testId}-${slot}`}
          />
        </Row>
        {armature && chosen ? (
          <Row label={t('targetBone')}>
            <Choice
              label={t('targetBone')}
              quiet
              value={chosen.bone ?? ''}
              disabled={readOnly}
              choices={[['', t('noTarget')], ...armature.bones.map((bone) => [bone.name, bone.name] as const)]}
              onChange={(bone) => set(bone ? { [slot]: chosen.object, [slot === 'target' ? 'targetBone' : 'poleBone']: bone } : { [slot]: chosen.object })}
            />
          </Row>
        ) : null}
      </>
    )
  }
  let fields: ReactNode = null
  switch (constraint.type) {
    case 'trackTo':
    case 'dampedTrack':
      fields = (
        <>
          <Row label={t('trackAxis')}>
            <Choice label={t('trackAxis')} quiet value={String(constraint.trackAxis ?? (constraint.type === 'trackTo' ? '-z' : 'y'))} disabled={readOnly} choices={TRACK_AXES.map((axis) => [axis, axisLabel(axis)] as const)} onChange={(trackAxis) => set({ trackAxis })} />
          </Row>
          {constraint.type === 'trackTo' ? (
            <Row label={t('upAxis')}>
              <Choice label={t('upAxis')} quiet value={String(constraint.upAxis ?? 'y')} disabled={readOnly} choices={['x', 'y', 'z'].map((axis) => [axis, axisLabel(axis)] as const)} onChange={(upAxis) => set({ upAxis })} />
            </Row>
          ) : null}
        </>
      )
      break
    case 'copyLocation':
    case 'copyRotation':
      fields = (
        <>
          <AxesRow label={t('axes')} value={flags(constraint.axes, [true, true, true])} onChange={(axes) => set({ axes })} />
          <AxesRow label={t('invert')} value={flags(constraint.invert, [false, false, false])} onChange={(invert) => set({ invert })} />
          {constraint.type === 'copyLocation' ? (
            <Row label={t('offset')}>
              <Checkbox label={t('offset')} checked={constraint.offset === true} disabled={readOnly} onChange={(offset) => set({ offset })} />
            </Row>
          ) : null}
          <SpaceRows info={info} owner={owner} constraint={constraint} testId={testId} set={set} />
        </>
      )
      break
    case 'limitRotation':
      fields = (
        <>
          <AxesRow label={t('limit')} value={flags(constraint.limit, [false, false, false])} onChange={(limit) => set({ limit })} />
          <VectorRow label={t('minimum')} value={(constraint.min as Vec3 | undefined) ?? [0, 0, 0]} unit="°" step={1} onCommit={(min) => set({ min })} testId={`${testId}-min`} />
          <VectorRow label={t('maximum')} value={(constraint.max as Vec3 | undefined) ?? [0, 0, 0]} unit="°" step={1} onCommit={(max) => set({ max })} testId={`${testId}-max`} />
          <Row label={t('space')}>
            <Choice
              label={t('space')}
              quiet
              value={String(constraint.space ?? 'world')}
              disabled={readOnly}
              choices={(owner.bone ? ['world', 'local', 'pose', 'localWithParent'] : ['world', 'local']).map((space) => [space, t(`space_${space}`)] as const)}
              onChange={(space) => set({ space })}
            />
          </Row>
        </>
      )
      break
    case 'childOf':
      fields = (
        <div className="scene-bone-actions">
          <Button size="sm" disabled={readOnly} onClick={() => run('constraint.setInverse', target)} data-testid={`${testId}-set-inverse`}>
            {p('Set Inverse')}
          </Button>
          <Button size="sm" disabled={readOnly} onClick={() => run('constraint.clearInverse', target)}>
            {p('Clear Inverse')}
          </Button>
        </div>
      )
      break
    case 'ik':
      fields = (
        <>
          {pick('pole', t('poleTarget'))}
          {constraint.pole ? (
            <Row label={t('poleAngle')}>
              <NumberInput label={t('poleAngle')} value={Number(constraint.poleAngle ?? 0)} unit="°" step={1} onCommit={(poleAngle) => set({ poleAngle })} />
            </Row>
          ) : null}
          <Row label={t('chainLength')}>
            <NumberInput label={t('chainLength')} value={Number(constraint.chain ?? 0)} min={0} max={255} step={1} onCommit={(chain) => set({ chain: Math.round(chain) })} testId={`${testId}-chain`} />
          </Row>
          <Row label={t('ikIterations')}>
            <NumberInput label={t('ikIterations')} value={Number(constraint.iterations ?? 500)} min={0} max={10000} step={10} onCommit={(iterations) => set({ iterations: Math.round(iterations) })} />
          </Row>
        </>
      )
      break
    default:
      fields = moreConstraintFields({ info, owner, constraint, testId, set })
      break
  }
  const targeted = !['limitRotation', 'armature'].includes(constraint.type)
  return (
    <div className="scene-modifier" data-enabled={constraint.enabled} data-testid={testId}>
      <div className="scene-modifier-header">
        <Checkbox label={<span className="scene-modifier-name">{constraint.name}</span>} checked={constraint.enabled} disabled={readOnly} onChange={(enabled) => set({ enabled })} />
        <span className="scene-modifier-actions">
          <IconButton icon={ArrowUpIcon} label={p('Move Up')} disabled={readOnly || index === 0} onClick={() => run('constraint.move', { ...target, to: index - 1 })} />
          <IconButton icon={ArrowDownIcon} label={p('Move Down')} disabled={readOnly || index === count - 1} onClick={() => run('constraint.move', { ...target, to: index + 1 })} />
          <IconButton icon={CheckIcon} label={p('Apply')} disabled={readOnly} onClick={() => run('constraint.apply', target)} data-testid={`${testId}-apply`} />
          <IconButton icon={TrashIcon} label={p('Remove')} disabled={readOnly} onClick={() => run('constraint.remove', target)} data-testid={`${testId}-remove`} />
        </span>
      </div>
      <div className="scene-modifier-body">
        {targeted ? pick('target', t('target')) : null}
        {fields}
        <Row label={t('influence')}>
          <NumberInput label={t('influence')} value={constraint.influence} min={0} max={1} step={0.05} track onCommit={(influence) => set({ influence })} testId={`${testId}-influence`} />
        </Row>
      </div>
    </div>
  )
}

/** An object's constraint stack, or (with `bone`) a pose bone's. */
export function ConstraintsSection({ object, info, bone }: { object: SceneObject; info: SceneInfo; bone?: string }) {
  const { phrase: p, t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const owner = bone ? { object: object.id, bone } : { object: object.id }
  const stack = bone ? (object.boneConstraints[bone] ?? []) : object.constraints
  const types = bone ? BONE_TYPES : OBJECT_TYPES
  return (
    <PanelSection
      title={p(bone ? 'Bone Constraints' : 'Constraints')}
      testId={bone ? 'scene-bone-constraints' : 'scene-constraints'}
      actions={
        <MenuButton
          align="end"
          trigger={({ open, toggle }) => <IconButton icon={PlusIcon} label={p('Add Constraint')} aria-expanded={open} disabled={readOnly || info.mode === 'edit'} onClick={toggle} data-testid="scene-add-constraint" />}
        >
          {types.map(([type, label]) => (
            <MenuItem key={type} label={p(label)} onSelect={() => run('constraint.add', { ...owner, type })} testId={`scene-add-constraint-${type}`} />
          ))}
        </MenuButton>
      }
    >
      {stack.length ? (
        <div className="scene-modifiers">
          {stack.map((constraint, index) => (
            <ConstraintCard key={`${index}-${constraint.name}`} info={info} owner={owner} constraint={constraint} index={index} count={stack.length} />
          ))}
        </div>
      ) : (
        <p className="kui-muted scene-empty">{t('noConstraints')}</p>
      )}
    </PanelSection>
  )
}
