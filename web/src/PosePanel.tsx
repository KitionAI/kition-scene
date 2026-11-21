import { useState } from 'react'
import { DiamondIcon, PlusIcon, TrashIcon } from '@kition/tessera/icons/react'
import { Button, Checkbox, Choice, IconButton, PanelSection, Segmented, useKitEditor, useReadOnly } from '@kition/sdk/editor'
import { NumberInput, Row, VectorRow, useRun } from './SidePanel'
import { ConstraintsSection } from './Constraints'
import type { SceneBone, SceneInfo, SceneObject } from './scene'

/** The armature's mode switch, its bones, and the active bone's rest shape or pose. */
function BonesSection({ object, info }: { object: SceneObject; info: SceneInfo }) {
  const { phrase: p, t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const editing = info.mode === 'armatureEdit' || info.mode === 'pose'
  const mode = info.mode === 'armatureEdit' ? 'edit' : info.mode === 'pose' ? 'pose' : 'object'
  const bone: SceneBone | undefined = object.bones.find((item) => item.name === info.activeBone)
  return (
    <PanelSection title={p('Bones')} testId="scene-bones">
      <Row label={t('armatureMode')}>
        <Segmented
          label={t('armatureMode')}
          value={mode}
          disabled={readOnly}
          segments={[
            { value: 'object', label: t('modeObject') },
            { value: 'edit', label: t('modeEdit') },
            { value: 'pose', label: t('modePose') },
          ]}
          onChange={(value) => {
            if (value === 'pose') run('mode.pose', { object: object.id, on: true })
            else if (value === 'object') run(info.mode === 'pose' ? 'mode.pose' : 'mode.armature', info.mode === 'pose' ? { on: false } : {})
            else if (info.mode !== 'armatureEdit') run('mode.armature', { object: object.id })
          }}
        />
      </Row>
      <div className="scene-bone-list" role="listbox" aria-label={p('Bones')} data-testid="scene-bone-list">
        {object.bones.map((item) => {
          const depth = depthOf(object.bones, item)
          const selected = info.bones.includes(item.name)
          return (
            <button
              key={item.name}
              type="button"
              role="option"
              aria-selected={selected}
              className="scene-bone-row"
              data-active={item.name === info.activeBone || undefined}
              style={{ paddingLeft: 8 + depth * 12 }}
              disabled={!editing}
              onClick={(event) => run('bone.select', { bones: [item.name], extend: event.shiftKey || event.metaKey || event.ctrlKey })}
              data-testid="scene-bone-item"
              data-name={item.name}
            >
              {item.name}
            </button>
          )
        })}
      </div>
      {info.mode === 'armatureEdit' ? (
        <div className="scene-bone-actions">
          <Button size="sm" disabled={readOnly} onClick={() => run('bone.add', {})} data-testid="scene-bone-add">
            {p('Add Bone')}
          </Button>
          <Button size="sm" disabled={readOnly || info.bones.length === 0} onClick={() => run('bone.extrude', {})} data-testid="scene-bone-extrude">
            {p('Extrude Bone')}
          </Button>
        </div>
      ) : null}
      {bone && info.mode === 'armatureEdit' ? (
        <>
          <VectorRow label={t('head')} value={bone.head} unit="m" step={0.1} onCommit={(head) => run('bone.set', { bone: bone.name, head })} testId="scene-bone-head" />
          <VectorRow label={t('tail')} value={bone.tail} unit="m" step={0.1} onCommit={(tail) => run('bone.set', { bone: bone.name, tail })} testId="scene-bone-tail" />
          <Row label={t('roll')}>
            <NumberInput label={t('roll')} value={bone.roll} unit="°" step={1} onCommit={(roll) => run('bone.set', { bone: bone.name, roll })} testId="scene-bone-roll" />
          </Row>
          <Row label={t('parentBone')}>
            <Choice
              label={t('parentBone')}
              quiet
              value={bone.parent ?? ''}
              disabled={readOnly}
              choices={[['', t('noParent')], ...object.bones.filter((other) => other.name !== bone.name).map((other) => [other.name, other.name] as const)]}
              onChange={(parent) => run('bone.set', { bone: bone.name, parent })}
            />
          </Row>
          <Row label={t('boneOptions')}>
            <div className="scene-checks">
              <Checkbox label={t('connected')} checked={bone.connected} disabled={readOnly || !bone.parent} onChange={(connected) => run('bone.set', { bone: bone.name, connected })} />
              <Checkbox label={t('deform')} checked={bone.deform} disabled={readOnly} onChange={(deform) => run('bone.set', { bone: bone.name, deform })} />
            </div>
          </Row>
          <EnvelopeRows bone={bone} />
        </>
      ) : null}
      {bone && info.mode === 'pose' ? (
        <>
          <VectorRow label={t('location')} value={bone.pose.location} unit="m" step={0.1} onCommit={(location) => run('pose.set', { bones: [bone.name], location })} testId="scene-pose-location" />
          <VectorRow label={t('rotation')} value={bone.pose.rotation} unit="°" step={1} onCommit={(rotation) => run('pose.set', { bones: [bone.name], rotation })} testId="scene-pose-rotation" />
          <VectorRow label={t('scale')} value={bone.pose.scale} step={0.1} onCommit={(scale) => run('pose.set', { bones: [bone.name], scale })} testId="scene-pose-scale" />
          <ShapeRows bone={bone} object={object} info={info} />
          <div className="scene-bone-actions">
            <Button size="sm" disabled={readOnly} onClick={() => run('pose.insertKey', {})} data-testid="scene-pose-key">
              {p('Insert Keyframe')}
            </Button>
            <Button size="sm" disabled={readOnly} onClick={() => run('pose.clear', {})}>
              {p('Clear Pose')}
            </Button>
          </div>
        </>
      ) : null}
    </PanelSection>
  )
}

/** A bone's envelope: the radii at its head and tail, how far past them it fades, its weight,
 * and whether it multiplies the bone's vertex group. */
function EnvelopeRows({ bone }: { bone: SceneBone }) {
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const set = (values: Record<string, unknown>) => run('bone.set', { bone: bone.name, ...values })
  const field = (key: 'headRadius' | 'tailRadius' | 'distance' | 'weight', name: string, unit?: string) => (
    <Row label={t(name)}>
      <NumberInput label={t(name)} value={bone.envelope[key]} min={0} max={1000} step={0.05} unit={unit} onCommit={(value) => set({ [name]: value })} testId={`scene-bone-${name}`} />
    </Row>
  )
  return (
    <>
      {field('headRadius', 'headRadius', 'm')}
      {field('tailRadius', 'tailRadius', 'm')}
      {field('distance', 'envelopeDistance', 'm')}
      {field('weight', 'envelopeWeight')}
      <Row label={t('envelope')}>
        <Checkbox label={t('envelopeMultiply')} checked={bone.envelope.multiply} disabled={readOnly} onChange={(envelopeMultiply) => set({ envelopeMultiply })} />
      </Row>
    </>
  )
}

/** A bone's custom shape: the object drawing it, and its offset, turn, and scale in the bone's
 * axes, scaled by the bone's length or not, drawn as wire or not. */
function ShapeRows({ bone, object, info }: { bone: SceneBone; object: SceneObject; info: SceneInfo }) {
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const set = (values: Record<string, unknown>) => run('bone.set', { bone: bone.name, ...values })
  const shapes = info.objects.filter((other) => other.id !== object.id && (other.type === 'mesh' || other.type === 'curve' || other.type === 'text'))
  const shape = bone.shape
  return (
    <>
      <Row label={t('customShape')}>
        <Choice
          label={t('customShape')}
          quiet
          value={shape ? String(shape.object) : 'none'}
          disabled={readOnly}
          choices={[['none', t('noShape')], ...shapes.map((other) => [String(other.id), other.name] as const)]}
          onChange={(value) => set({ shape: value === 'none' ? null : Number(value) })}
          testId="scene-bone-shape"
        />
      </Row>
      {shape ? (
        <>
          <VectorRow label={t('shapeTranslation')} value={shape.translation} unit="m" step={0.1} onCommit={(shapeTranslation) => set({ shapeTranslation })} testId="scene-bone-shape-translation" />
          <VectorRow label={t('shapeRotation')} value={shape.rotation} unit="°" step={1} onCommit={(shapeRotation) => set({ shapeRotation })} testId="scene-bone-shape-rotation" />
          <VectorRow label={t('shapeScale')} value={shape.scale} step={0.1} onCommit={(shapeScale) => set({ shapeScale })} testId="scene-bone-shape-scale" />
          <Row label={t('boneOptions')}>
            <div className="scene-checks">
              <Checkbox label={t('shapeBoneSize')} checked={shape.boneSize} disabled={readOnly} onChange={(shapeBoneSize) => set({ shapeBoneSize })} />
              <Checkbox label={t('shapeWire')} checked={shape.wire} disabled={readOnly} onChange={(shapeWire) => set({ shapeWire })} testId="scene-bone-shape-wire" />
            </div>
          </Row>
        </>
      ) : null}
    </>
  )
}

function depthOf(bones: SceneBone[], bone: SceneBone) {
  let depth = 0
  let parent = bone.parent
  while (parent && depth < bones.length) {
    depth += 1
    parent = bones.find((other) => other.name === parent)?.parent ?? null
  }
  return depth
}

/** A mesh's vertex groups: add, remove, and (in edit mode) assign the selection at a weight. */
function GroupsSection({ object, info }: { object: SceneObject; info: SceneInfo }) {
  const { phrase: p, t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const [current, setCurrent] = useState<string | null>(null)
  const [weight, setWeight] = useState(1)
  const chosen = object.vertexGroups.find((group) => group.name === current)?.name ?? object.vertexGroups[0]?.name ?? null
  const editing = info.mode === 'edit' && info.editing === object.id
  return (
    <PanelSection
      title={p('Vertex Groups')}
      testId="scene-vertex-groups"
      actions={<IconButton icon={PlusIcon} label={p('Add Vertex Group')} disabled={readOnly} onClick={() => run('vertexGroup.add', { object: object.id })} data-testid="scene-group-add" />}
    >
      {object.vertexGroups.length ? (
        <div className="scene-bone-list" role="listbox" aria-label={p('Vertex Groups')}>
          {object.vertexGroups.map((group) => (
            <div key={group.name} className="scene-key-row" data-active={group.name === chosen || undefined}>
              <button type="button" role="option" aria-selected={group.name === chosen} className="scene-bone-row" onClick={() => setCurrent(group.name)} data-testid="scene-group-item" data-name={group.name}>
                {group.name}
                <span className="kui-muted tabular-nums">{t('groupCount', { count: group.vertices })}</span>
              </button>
              <IconButton icon={TrashIcon} label={p('Remove Vertex Group')} disabled={readOnly} onClick={() => run('vertexGroup.remove', { object: object.id, name: group.name })} />
            </div>
          ))}
        </div>
      ) : (
        <p className="kui-muted scene-empty">{t('noGroups')}</p>
      )}
      {editing && chosen ? (
        <>
          <Row label={t('weight')}>
            <NumberInput label={t('weight')} value={weight} min={0} max={1} step={0.05} track onCommit={setWeight} testId="scene-group-weight" />
          </Row>
          <div className="scene-bone-actions">
            <Button size="sm" disabled={readOnly || info.vertices.length === 0} onClick={() => run('vertexGroup.assign', { name: chosen, weight })} data-testid="scene-group-assign">
              {p('Assign to Group')}
            </Button>
            <Button size="sm" disabled={readOnly || info.vertices.length === 0} onClick={() => run('vertexGroup.unassign', { name: chosen })}>
              {p('Remove from Group')}
            </Button>
            <Button size="sm" onClick={() => run('vertexGroup.select', { name: chosen })}>
              {p('Select Group')}
            </Button>
          </div>
        </>
      ) : null}
    </PanelSection>
  )
}

/** A mesh's shape keys: the basis, each key's value, and keying it on this frame. */
function ShapeKeysSection({ object, info }: { object: SceneObject; info: SceneInfo }) {
  const { phrase: p, t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const editing = info.mode === 'edit' && info.editing === object.id
  return (
    <PanelSection
      title={p('Shape Keys')}
      testId="scene-shape-keys"
      actions={<IconButton icon={PlusIcon} label={p('Add Shape Key')} disabled={readOnly} onClick={() => run('shapeKey.add', { object: object.id })} data-testid="scene-shape-add" />}
    >
      {object.shapeKeys.length ? (
        <>
          <div className="scene-key-row scene-basis">
            <span>{t('basis')}</span>
          </div>
          {object.shapeKeys.map((key) => (
            <div key={key.name} className="scene-key-row" data-testid="scene-shape-item" data-name={key.name}>
              <span className="scene-key-name">{key.name}</span>
              <NumberInput label={key.name} value={key.value} min={0} max={1} step={0.05} track onCommit={(value) => run('shapeKey.set', { object: object.id, name: key.name, value })} testId={`scene-shape-value-${key.name}`} />
              <IconButton icon={DiamondIcon} label={p('Key Shape Value')} disabled={readOnly} onClick={() => run('shapeKey.insertKey', { object: object.id, name: key.name })} />
              <IconButton icon={TrashIcon} label={p('Remove Shape Key')} disabled={readOnly} onClick={() => run('shapeKey.remove', { object: object.id, name: key.name })} />
            </div>
          ))}
          {editing ? <ShapeNudge object={object} info={info} /> : null}
        </>
      ) : (
        <p className="kui-muted scene-empty">{t('noShapeKeys')}</p>
      )}
    </PanelSection>
  )
}

/** Moves the selected vertices in one shape key (the shape is sculpted by offsets). */
function ShapeNudge({ object, info }: { object: SceneObject; info: SceneInfo }) {
  const { phrase: p, t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const [key, setKey] = useState(object.shapeKeys[0]?.name ?? '')
  const [offset, setOffset] = useState<[number, number, number]>([0, 0, 0.1])
  const name = object.shapeKeys.some((item) => item.name === key) ? key : (object.shapeKeys[0]?.name ?? '')
  return (
    <>
      <Row label={t('shapeKey')}>
        <Choice label={t('shapeKey')} quiet value={name} choices={object.shapeKeys.map((item) => [item.name, item.name] as const)} onChange={setKey} />
      </Row>
      <VectorRow label={t('offset')} value={offset} unit="m" step={0.05} onCommit={setOffset} testId="scene-shape-offset" />
      <div className="scene-bone-actions">
        <Button size="sm" disabled={readOnly || info.vertices.length === 0 || !name} onClick={() => run('shapeKey.offset', { name, offset })} data-testid="scene-shape-move">
          {p('Move in Shape Key')}
        </Button>
      </div>
    </>
  )
}

/** The rigging sections the active object has. */
export function RigSections({ object, info }: { object: SceneObject; info: SceneInfo }) {
  if (object.type === 'armature')
    return (
      <>
        <BonesSection object={object} info={info} />
        {info.mode === 'pose' && info.activeBone ? <ConstraintsSection key={`bone-${info.activeBone}`} object={object} info={info} bone={info.activeBone} /> : null}
      </>
    )
  if (object.type !== 'mesh') return null
  return (
    <>
      <GroupsSection object={object} info={info} />
      <ShapeKeysSection object={object} info={info} />
    </>
  )
}
