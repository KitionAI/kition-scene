import type { ReactNode } from 'react'
import { PlusIcon, TrashIcon } from '@kition/tessera-icons/react'
import { Checkbox, Choice, IconButton, useKitEditor, useReadOnly } from '@kition/kit-editor'
import { NumberInput, Row, VectorRow } from './SidePanel'
import type { Constraint, SceneInfo, Vec3 } from './scene'
import './ConstraintFields.css'

type Set = (values: Record<string, unknown>) => void

type Props = { info: SceneInfo; owner: { object: number; bone?: string }; constraint: Constraint; testId: string; set: Set }

const SPACES = ['world', 'local', 'pose', 'localWithParent'] as const
const AXES = ['x', 'y', 'z', '-x', '-y', '-z'] as const
const axisLabel = (axis: string) => axis.toUpperCase().replace('-', '−')
const vec = (value: unknown, fallback: Vec3): Vec3 => (Array.isArray(value) && value.length === 3 ? (value.map(Number) as Vec3) : fallback)

/** A choice among values named `${prefix}${value}` in the catalog. */
function Pick({ label, value, values, prefix, set, field, testId }: { label: string; value: string; values: readonly string[]; prefix: string; set: Set; field: string; testId?: string }) {
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  return (
    <Row label={label}>
      <Choice label={label} quiet value={value} disabled={readOnly} choices={values.map((item) => [item, t(`${prefix}${item}`)] as const)} onChange={(next) => set({ [field]: next })} testId={testId} />
    </Row>
  )
}

/** The owner's and target's spaces (bones' own spaces only on bones). */
export function SpaceRows({ owner, constraint, set, ownerSide = true, targetSide = true }: Props & { ownerSide?: boolean; targetSide?: boolean }) {
  const { t } = useKitEditor()
  const ownerSpaces = owner.bone ? SPACES : SPACES.filter((space) => space === 'world' || space === 'local')
  const targetSpaces = constraint.target?.bone ? SPACES : SPACES.filter((space) => space === 'world' || space === 'local')
  return (
    <>
      {ownerSide ? <Pick label={t('ownerSpace')} value={String(constraint.ownerSpace ?? 'world')} values={ownerSpaces} prefix="space_" set={set} field="ownerSpace" /> : null}
      {targetSide ? <Pick label={t('targetSpace')} value={String(constraint.targetSpace ?? 'world')} values={targetSpaces} prefix="space_" set={set} field="targetSpace" /> : null}
    </>
  )
}

function StretchFields({ constraint, set, testId }: Props) {
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  return (
    <>
      <Row label={t('restLength')}>
        <span className="scene-constraint-inline">
          <NumberInput label={t('restLength')} value={Number(constraint.restLength ?? 0)} min={0} max={1e6} step={0.1} unit="m" onCommit={(restLength) => set({ restLength })} testId={`${testId}-rest`} />
          <IconButton icon={TrashIcon} label={t('resetRest')} disabled={readOnly} onClick={() => set({ restLength: 0 })} />
        </span>
      </Row>
      <Row label={t('bulge')}>
        <NumberInput label={t('bulge')} value={Number(constraint.bulge ?? 1)} min={0} max={100} step={0.1} onCommit={(bulge) => set({ bulge })} />
      </Row>
      <Pick label={t('stretchVolume')} value={String(constraint.volume ?? 'xzx')} values={['xzx', 'x', 'z', 'none']} prefix="volume_" set={set} field="volume" />
      <Pick label={t('keepAxis')} value={String(constraint.keepAxis ?? 'swingY')} values={['swingY', 'planeX', 'planeZ']} prefix="keep_" set={set} field="keepAxis" />
    </>
  )
}

function FloorFields({ constraint, set, testId }: Props) {
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  return (
    <>
      <Row label={t('floorAxis')}>
        <Choice label={t('floorAxis')} quiet value={String(constraint.axis ?? 'z')} disabled={readOnly} choices={AXES.map((axis) => [axis, axisLabel(axis)] as const)} onChange={(axis) => set({ axis })} testId={`${testId}-axis`} />
      </Row>
      <Row label={t('floorOffset')}>
        <NumberInput label={t('floorOffset')} value={Number(constraint.offset ?? 0)} step={0.05} unit="m" onCommit={(offset) => set({ offset })} />
      </Row>
      <Row label={t('useRotation')}>
        <Checkbox label={t('useRotation')} checked={constraint.useRotation === true} disabled={readOnly} onChange={(useRotation) => set({ useRotation })} />
      </Row>
    </>
  )
}

function TransformFields(props: Props) {
  const { constraint, set, testId } = props
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  const from = String(constraint.mapFrom ?? 'location')
  const to = String(constraint.mapTo ?? 'location')
  const unit = (channel: string) => (channel === 'rotation' ? '°' : channel === 'location' ? 'm' : undefined)
  const neutral: Vec3 = [0, 0, 0]
  const mixes = to === 'location' ? ['add', 'replace'] : to === 'rotation' ? ['add', 'replace', 'before', 'after'] : ['replace', 'multiply']
  const mapping = Array.isArray(constraint.mapping) ? constraint.mapping.map(String) : ['x', 'y', 'z']
  return (
    <>
      <Pick label={t('mapFrom')} value={from} values={['location', 'rotation', 'scale']} prefix="channel_" set={set} field="mapFrom" testId={`${testId}-from`} />
      <VectorRow label={t('fromMin')} value={vec(constraint.fromMin, neutral)} unit={unit(from)} step={from === 'rotation' ? 1 : 0.1} onCommit={(fromMin) => set({ fromMin })} testId={`${testId}-from-min`} />
      <VectorRow label={t('fromMax')} value={vec(constraint.fromMax, neutral)} unit={unit(from)} step={from === 'rotation' ? 1 : 0.1} onCommit={(fromMax) => set({ fromMax })} testId={`${testId}-from-max`} />
      <Pick label={t('mapTo')} value={to} values={['location', 'rotation', 'scale']} prefix="channel_" set={set} field="mapTo" testId={`${testId}-to`} />
      <Row label={t('mapping')}>
        <div className="scene-constraint-mapping">
          {[0, 1, 2].map((axis) => (
            <Choice
              key={axis}
              label={`${t('mapping')} ${'XYZ'[axis]}`}
              quiet
              value={mapping[axis] ?? 'xyz'[axis]}
              disabled={readOnly}
              choices={['x', 'y', 'z'].map((item) => [item, `${'XYZ'[axis]} ← ${item.toUpperCase()}`] as const)}
              onChange={(value) => set({ mapping: mapping.map((old, at) => (at === axis ? value : old)) })}
            />
          ))}
        </div>
      </Row>
      <VectorRow label={t('toMin')} value={vec(constraint.toMin, neutral)} unit={unit(to)} step={to === 'rotation' ? 1 : 0.1} onCommit={(toMin) => set({ toMin })} testId={`${testId}-to-min`} />
      <VectorRow label={t('toMax')} value={vec(constraint.toMax, neutral)} unit={unit(to)} step={to === 'rotation' ? 1 : 0.1} onCommit={(toMax) => set({ toMax })} testId={`${testId}-to-max`} />
      <Pick label={t('mix')} value={String(constraint.mix ?? (to === 'scale' ? 'replace' : 'add'))} values={mixes} prefix="mix_" set={set} field="mix" />
      <Row label={t('extrapolate')}>
        <Checkbox label={t('extrapolate')} checked={constraint.extrapolate === true} disabled={readOnly} onChange={(extrapolate) => set({ extrapolate })} />
      </Row>
      <SpaceRows {...props} />
    </>
  )
}

const CHANNELS = ['locationX', 'locationY', 'locationZ', 'rotationX', 'rotationY', 'rotationZ', 'scaleX', 'scaleY', 'scaleZ']

function ActionFields(props: Props) {
  const { info, constraint, set, testId } = props
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  const channel = String(constraint.channel ?? 'locationX')
  const unit = channel.startsWith('rotation') ? '°' : channel.startsWith('location') ? 'm' : undefined
  return (
    <>
      <Row label={t('constraintAction')}>
        <Choice
          label={t('constraintAction')}
          quiet
          value={typeof constraint.action === 'string' ? constraint.action : ''}
          disabled={readOnly}
          choices={[['', t('noAction')], ...info.animation.actions.map((action) => [action.name, action.name] as const)]}
          onChange={(action) => set({ action: action || null })}
          testId={`${testId}-action`}
        />
      </Row>
      <Row label={t('actionChannel')}>
        <Choice label={t('actionChannel')} quiet value={channel} disabled={readOnly} choices={CHANNELS.map((item) => [item, `${t(`channel_${item.replace(/[XYZ]$/, '')}`)} ${item.slice(-1)}`] as const)} onChange={(next) => set({ channel: next })} />
      </Row>
      <Row label={t('actionMin')}>
        <NumberInput label={t('actionMin')} value={Number(constraint.min ?? 0)} step={unit === '°' ? 1 : 0.1} unit={unit} onCommit={(min) => set({ min })} testId={`${testId}-min`} />
      </Row>
      <Row label={t('actionMax')}>
        <NumberInput label={t('actionMax')} value={Number(constraint.max ?? 0)} step={unit === '°' ? 1 : 0.1} unit={unit} onCommit={(max) => set({ max })} testId={`${testId}-max`} />
      </Row>
      <Row label={t('frameStart')}>
        <NumberInput label={t('frameStart')} value={Number(constraint.frameStart ?? 0)} step={1} onCommit={(frameStart) => set({ frameStart })} />
      </Row>
      <Row label={t('frameEnd')}>
        <NumberInput label={t('frameEnd')} value={Number(constraint.frameEnd ?? 0)} step={1} onCommit={(frameEnd) => set({ frameEnd })} />
      </Row>
      <Pick label={t('mix')} value={String(constraint.mix ?? 'after')} values={['replace', 'beforeFull', 'before', 'beforeSplit', 'afterFull', 'after', 'afterSplit']} prefix="mix_" set={set} field="mix" />
      <SpaceRows {...props} ownerSide={false} />
    </>
  )
}

type ArmatureEntry = { object: number; bone: string; weight: number }

function ArmatureFields({ info, constraint, set, testId }: Props) {
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  const entries: ArmatureEntry[] = Array.isArray(constraint.targets)
    ? constraint.targets.map((entry: Record<string, unknown>) => ({ object: Number(entry.object), bone: String(entry.bone ?? ''), weight: Number(entry.weight ?? 1) }))
    : []
  const rigs = info.objects.filter((object) => object.type === 'armature' && object.bones.length > 0)
  const write = (next: ArmatureEntry[]) => set({ targets: next })
  return (
    <>
      <div className="scene-constraint-list" data-testid={`${testId}-bones`}>
        {entries.map((entry, index) => {
          const rig = rigs.find((object) => object.id === entry.object)
          return (
            <div className="scene-constraint-entry" key={`${entry.object}-${entry.bone}-${index}`}>
              <Choice
                label={t('armatureBones')}
                quiet
                value={String(entry.object)}
                disabled={readOnly}
                choices={rigs.map((object) => [String(object.id), object.name] as const)}
                onChange={(value) => {
                  const object = rigs.find((item) => item.id === Number(value))
                  write(entries.map((old, at) => (at === index ? { ...old, object: Number(value), bone: object?.bones[0]?.name ?? '' } : old)))
                }}
              />
              <Choice
                label={t('targetBone')}
                quiet
                value={entry.bone}
                disabled={readOnly}
                choices={(rig?.bones ?? []).map((bone) => [bone.name, bone.name] as const)}
                onChange={(bone) => write(entries.map((old, at) => (at === index ? { ...old, bone } : old)))}
              />
              <NumberInput label={t('boneWeight')} value={entry.weight} min={0} max={1} step={0.05} onCommit={(weight) => write(entries.map((old, at) => (at === index ? { ...old, weight } : old)))} />
              <IconButton icon={TrashIcon} label={t('removeArmatureBone')} disabled={readOnly} onClick={() => write(entries.filter((_, at) => at !== index))} />
            </div>
          )
        })}
        <IconButton
          icon={PlusIcon}
          label={t('addArmatureBone')}
          disabled={readOnly || rigs.length === 0}
          onClick={() => {
            const rig = rigs[0]
            if (rig) write([...entries, { object: rig.id, bone: rig.bones[0]?.name ?? '', weight: 1 }])
          }}
          data-testid={`${testId}-add-bone`}
        />
      </div>
      <Row label={t('preserveVolume')}>
        <Checkbox label={t('preserveVolume')} checked={constraint.preserveVolume === true} disabled={readOnly} onChange={(preserveVolume) => set({ preserveVolume })} />
      </Row>
    </>
  )
}

function SplineIkFields({ constraint, set, testId }: Props) {
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  const check = (field: string, label: string, fallback: boolean) => (
    <Row label={label}>
      <Checkbox label={label} checked={constraint[field] === undefined ? fallback : constraint[field] === true} disabled={readOnly} onChange={(on) => set({ [field]: on })} />
    </Row>
  )
  return (
    <>
      <Row label={t('splineChain')}>
        <NumberInput label={t('splineChain')} value={Number(constraint.chain ?? 1)} min={1} max={255} step={1} onCommit={(chain) => set({ chain: Math.round(chain) })} testId={`${testId}-chain`} />
      </Row>
      {check('evenDivisions', t('evenDivisions'), false)}
      {check('chainOffset', t('chainOffset'), false)}
      {check('curveRadius', t('curveRadius'), true)}
      <Pick label={t('yScale')} value={String(constraint.yScale ?? 'fitCurve')} values={['none', 'fitCurve', 'boneOriginal']} prefix="yScale_" set={set} field="yScale" />
      <Pick label={t('xzScale')} value={String(constraint.xzScale ?? 'none')} values={['none', 'boneOriginal', 'inversePreserve', 'volumePreserve']} prefix="xzScale_" set={set} field="xzScale" />
      {String(constraint.xzScale ?? 'none') === 'volumePreserve' ? (
        <Row label={t('bulge')}>
          <NumberInput label={t('bulge')} value={Number(constraint.bulge ?? 1)} min={0} max={100} step={0.1} onCommit={(bulge) => set({ bulge })} />
        </Row>
      ) : null}
    </>
  )
}

/** The settings of Stretch To, Floor, Transformation, Action, Armature, and Spline IK (`null` for other kinds). */
export function moreConstraintFields(props: Props): ReactNode {
  switch (props.constraint.type) {
    case 'stretchTo':
      return <StretchFields {...props} />
    case 'floor':
      return <FloorFields {...props} />
    case 'transform':
      return <TransformFields {...props} />
    case 'action':
      return <ActionFields {...props} />
    case 'armature':
      return <ArmatureFields {...props} />
    case 'splineIk':
      return <SplineIkFields {...props} />
    default:
      return null
  }
}
