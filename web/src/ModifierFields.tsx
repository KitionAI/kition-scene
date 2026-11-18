import type { ReactNode } from 'react'
import { ImagePlusIcon, TrashIcon } from '@kition/tessera-icons/react'
import { Checkbox, Choice, IconButton, useKitEditor, useReadOnly } from '@kition/kit-editor'
import { NumberInput, Row, useRun } from './SidePanel'
import { encodeBase64, useImageUrl } from './TextureSlot'
import './ModifierFields.css'
import type { Modifier, SceneInfo, SceneObject } from './scene'

const IMAGE_FORMATS = ['png', 'jpg', 'jpeg']

type Props = { object: SceneObject; info: SceneInfo; modifier: Modifier; index: number }

/** Picks another object for a modifier (`operand`), from those `fits` allows. */
function OperandRow({ object, info, modifier, index, label, fits }: Props & { label: string; fits: (other: SceneObject) => boolean }) {
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const others = info.objects.filter((other) => other.id !== object.id && fits(other))
  return (
    <Row label={label}>
      <Choice
        label={label}
        quiet
        value={modifier.operand != null ? String(modifier.operand) : 'none'}
        choices={[['none', t('noTarget')], ...others.map((other) => [String(other.id), other.name] as const)]}
        disabled={readOnly}
        testId={`scene-modifier-${index}-operand`}
        onChange={(value) => value !== 'none' && run('modifier.set', { object: object.id, index, operand: Number(value) })}
      />
    </Row>
  )
}

function ShrinkwrapFields(props: Props) {
  const { object, modifier, index } = props
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const set = (values: Record<string, unknown>) => run('modifier.set', { object: object.id, index, ...values })
  const method = String(modifier.wrapMethod ?? 'nearestSurface')
  return (
    <>
      <OperandRow {...props} label={t('wrapTarget')} fits={(other) => other.type === 'mesh' || other.type === 'curve' || other.type === 'text'} />
      <Row label={t('wrapMethod')}>
        <Choice
          label={t('wrapMethod')}
          quiet
          value={method}
          choices={(['nearestSurface', 'project', 'nearestVertex'] as const).map((value) => [value, t(`wrap_${value}`)] as const)}
          disabled={readOnly}
          testId={`scene-modifier-${index}-wrapMethod`}
          onChange={(wrapMethod) => set({ wrapMethod })}
        />
      </Row>
      <Row label={t('offset')}>
        <NumberInput label={t('offset')} value={Number(modifier.offset ?? 0)} min={-100} max={100} step={0.01} unit="m" onCommit={(offset) => set({ offset })} testId={`scene-modifier-${index}-offset`} />
      </Row>
      {method === 'project' ? (
        <Row label={t('projectDirections')}>
          <div className="scene-checks-column">
            <Checkbox label={t('projectPositive')} checked={modifier.positive !== false} disabled={readOnly} onChange={(positive) => set({ positive })} />
            <Checkbox label={t('projectNegative')} checked={modifier.negative === true} disabled={readOnly} onChange={(negative) => set({ negative })} />
          </div>
        </Row>
      ) : null}
    </>
  )
}

function LatticeFields(props: Props) {
  const { object, modifier, index } = props
  const { t } = useKitEditor()
  const run = useRun()
  return (
    <>
      <OperandRow {...props} label={t('latticeObject')} fits={(other) => other.type === 'lattice'} />
      <Row label={t('strength')}>
        <NumberInput label={t('strength')} value={Number(modifier.strength ?? 1)} min={-10} max={10} step={0.05} onCommit={(strength) => run('modifier.set', { object: object.id, index, strength })} testId={`scene-modifier-${index}-strength`} />
      </Row>
    </>
  )
}

function DecimateFields({ object, modifier, index }: Props) {
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const set = (values: Record<string, unknown>) => run('modifier.set', { object: object.id, index, ...values })
  const mode = String(modifier.mode ?? 'collapse')
  return (
    <>
      <Row label={t('decimateMode')}>
        <Choice
          label={t('decimateMode')}
          quiet
          value={mode}
          choices={(['collapse', 'unsubdivide', 'planar'] as const).map((value) => [value, t(`decimate_${value}`)] as const)}
          disabled={readOnly}
          testId={`scene-modifier-${index}-mode`}
          onChange={(value) => set({ mode: value })}
        />
      </Row>
      {mode === 'collapse' ? (
        <Row label={t('ratio')}>
          <NumberInput label={t('ratio')} value={Number(modifier.ratio ?? 1)} min={0} max={1} step={0.05} onCommit={(ratio) => set({ ratio })} testId={`scene-modifier-${index}-ratio`} />
        </Row>
      ) : mode === 'unsubdivide' ? (
        <Row label={t('iterations')}>
          <NumberInput label={t('iterations')} value={Number(modifier.iterations ?? 0)} min={0} max={16} step={1} onCommit={(iterations) => set({ iterations })} testId={`scene-modifier-${index}-iterations`} />
        </Row>
      ) : (
        <Row label={t('angleLimit')}>
          <NumberInput label={t('angleLimit')} value={Number(modifier.angleLimit ?? 5)} min={0} max={180} step={1} unit="°" onCommit={(angleLimit) => set({ angleLimit })} testId={`scene-modifier-${index}-angleLimit`} />
        </Row>
      )}
    </>
  )
}

/** A Displace modifier's image: a thumbnail, a picker, and where the image is looked up. */
export function DisplaceImageFields({ object, info, modifier, index }: Props) {
  const { host, report, t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const hash = typeof modifier.image === 'string' ? modifier.image : null
  const image = hash ? info.images.find((item) => item.hash === hash) : undefined
  const url = useImageUrl(hash)
  const choose = async () => {
    try {
      const path = await host.pick('import', IMAGE_FORMATS)
      if (!path) return
      const { bytes } = await host.files.read(path)
      const imageName = path.split('/').pop() ?? path
      await run('modifier.setImage', { object: object.id, index, base64: encodeBase64(new Uint8Array(bytes)), imageName })
    } catch (error) {
      report(t('imageFailed'), error)
    }
  }
  return (
    <>
      <Row label={t('displaceImage')}>
        <div className="scene-texture" data-testid={`scene-modifier-${index}-image`}>
          <span className="scene-texture-thumb" aria-hidden={!url}>
            {url ? <img src={url} alt="" draggable={false} /> : null}
          </span>
          <span className="scene-texture-name" title={image?.name ?? ''}>
            {image ? image.name : t('displaceNoImage')}
          </span>
          <IconButton icon={ImagePlusIcon} label={t('displaceChooseImage')} disabled={readOnly} onClick={() => void choose()} data-testid={`scene-modifier-${index}-image-choose`} />
          {hash ? <IconButton icon={TrashIcon} label={t('displaceClearImage')} disabled={readOnly} onClick={() => void run('modifier.setImage', { object: object.id, index, clear: true })} data-testid={`scene-modifier-${index}-image-clear`} /> : null}
        </div>
      </Row>
      {hash ? (
        <Row label={t('textureCoordinates')}>
          <Choice
            label={t('textureCoordinates')}
            quiet
            value={String(modifier.coordinates ?? 'local')}
            choices={(['local', 'global', 'uv'] as const).map((value) => [value, t(`coordinates_${value}`)] as const)}
            disabled={readOnly}
            testId={`scene-modifier-${index}-coordinates`}
            onChange={(coordinates) => run('modifier.set', { object: object.id, index, coordinates })}
          />
        </Row>
      ) : null}
    </>
  )
}

/** An Armature modifier: dual quaternions, what binds vertices to bones (their groups, the bones'
 * envelopes), its own vertex group, and Multi Modifier. */
function ArmatureFields({ object, modifier, index }: Props) {
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const set = (values: Record<string, unknown>) => run('modifier.set', { object: object.id, index, ...values })
  const group = typeof modifier.vertexGroup === 'string' ? modifier.vertexGroup : ''
  return (
    <>
      <Row label={t('armatureBind')}>
        <div className="scene-checks-column">
          <Checkbox label={t('preserveVolume')} checked={modifier.preserveVolume === true} disabled={readOnly} onChange={(preserveVolume) => set({ preserveVolume })} />
          <Checkbox label={t('bindVertexGroups')} checked={modifier.vertexGroups !== false} disabled={readOnly} onChange={(vertexGroups) => set({ vertexGroups })} />
          <Checkbox label={t('bindEnvelopes')} checked={modifier.envelopes === true} disabled={readOnly} onChange={(envelopes) => set({ envelopes })} testId={`scene-modifier-${index}-envelopes`} />
          <Checkbox label={t('multiModifier')} checked={modifier.multiModifier === true} disabled={readOnly} onChange={(multiModifier) => set({ multiModifier })} />
        </div>
      </Row>
      <Row label={t('paintGroup')}>
        <Choice
          label={t('paintGroup')}
          quiet
          value={group}
          disabled={readOnly}
          choices={[['', t('noTarget')], ...object.vertexGroups.map((item) => [item.name, item.name] as const)]}
          onChange={(vertexGroup) => set({ vertexGroup: vertexGroup || null })}
          testId={`scene-modifier-${index}-vertexGroup`}
        />
      </Row>
      {group ? (
        <Row label={t('invert')}>
          <Checkbox label={t('invert')} checked={modifier.invertVertexGroup === true} disabled={readOnly} onChange={(invertVertexGroup) => set({ invertVertexGroup })} />
        </Row>
      ) : null}
    </>
  )
}

/** The fields of the Armature, Shrinkwrap, Lattice, and Decimate modifiers (`null` for other
 * kinds). */
export function moreModifierFields(props: Props): ReactNode {
  switch (props.modifier.type) {
    case 'armature':
      return <ArmatureFields {...props} />
    case 'shrinkwrap':
      return <ShrinkwrapFields {...props} />
    case 'lattice':
      return <LatticeFields {...props} />
    case 'decimate':
      return <DecimateFields {...props} />
    default:
      return null
  }
}
