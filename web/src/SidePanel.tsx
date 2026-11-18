import { PresenceBadges } from './Presence'
import { useEffect, useState, type ReactNode } from 'react'
import { ArrowDownIcon, ArrowUpIcon, BoxIcon, CameraIcon, WaypointsIcon, CheckIcon, EyeIcon, EyeOffIcon, Grid2x2Icon, LightbulbIcon, Move3dIcon, PlusIcon, SplineIcon, TrashIcon, TypeIcon, type IconComponent } from '@kition/tessera-icons/react'
import {
  Checkbox,
  Choice,
  ColorPicker,
  IconButton,
  MenuButton,
  MenuItem,
  NumberField,
  PanelSection,
  Popover,
  PopoverContent,
  PopoverTrigger,
  Segmented,
  useKitEditor,
  useReadOnly,
} from '@kition/kit-editor'
import { TextureSlot } from './TextureSlot'
import { WorldImageSlot } from './WorldImage'
import { RigSections } from './PosePanel'
import { ShapeSections } from './CurvePanel'
import { LatticeSection } from './LatticePanel'
import { DisplaceImageFields, moreModifierFields } from './ModifierFields'
import { ConstraintsSection } from './Constraints'
import { ActionsSection, DriversSection, KeyingSetsSection } from './AnimationPanel'
import { PhysicsSection, PhysicsWorldSection } from './PhysicsPanel'
import { ParticlesSection } from './Particles'
import { MultiresFields, SculptSections } from './SculptPanel'
import { GeometryNodesFields } from './NodeEditor'
import { CompositingRows } from './Compositor'
import { childrenOf, linearHex, type Modifier, type SceneInfo, type SceneObject, type Vec3 } from './scene'

const TYPE_ICONS: Record<SceneObject['type'], IconComponent> = { mesh: BoxIcon, light: LightbulbIcon, camera: CameraIcon, empty: Move3dIcon, armature: WaypointsIcon, curve: SplineIcon, text: TypeIcon, lattice: Grid2x2Icon }
const round = (value: number, digits = 4) => Math.round(value * 10 ** digits) / 10 ** digits
/** Whether a property is keyed on this frame, animated, or neither (the field's tint). */
const keyState = (object: SceneObject, property: string) => (object.keyedNow.includes(property) ? 'keyed' : object.animated.includes(property) ? 'animated' : undefined)

/** A run that refreshes the editor; failures toast. */
export function useRun() {
  const { run } = useKitEditor()
  return (command: string, params: Record<string, unknown>) => void run(command, params)
}

// ---------------------------------------------------------------- fields

/** A labelled row of the properties grid. */
export function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="scene-row">
      <span className="scene-row-label">{label}</span>
      <div className="scene-row-value">{children}</div>
    </div>
  )
}

/** One number that commits once (on release, Enter, or blur), showing its draft meanwhile. */
export function NumberInput({ label, value, onCommit, min = -1e6, max = 1e6, step = 0.01, unit, testId, track }: { label: string; value: number; onCommit: (value: number) => void; min?: number; max?: number; step?: number; unit?: string; testId?: string; track?: boolean }) {
  const readOnly = useReadOnly()
  const [draft, setDraft] = useState<number | null>(null)
  useEffect(() => setDraft(null), [value])
  return (
    <NumberField
      label={label}
      value={draft ?? round(value)}
      min={min}
      max={max}
      step={step}
      unit={unit}
      track={track}
      disabled={readOnly}
      testId={testId}
      onChange={(next, final) => {
        if (!final) return setDraft(next)
        setDraft(null)
        if (Math.abs(next - value) > 1e-9) onCommit(next)
      }}
    />
  )
}

/** X, Y, and Z of one vector; a change sends the whole vector. */
export function VectorRow({ label, value, unit, step, onCommit, testId, animated }: { label: string; value: Vec3; unit?: string; step?: number; onCommit: (value: Vec3) => void; testId: string; animated?: 'keyed' | 'animated' }) {
  return (
    <Row label={label}>
      <div className="scene-vector" data-animated={animated} data-testid={animated ? `${testId}-${animated}` : undefined}>
        {(['X', 'Y', 'Z'] as const).map((axis, index) => (
          <NumberInput
            key={axis}
            label={axis}
            value={value[index]}
            unit={unit}
            step={step}
            testId={`${testId}-${axis.toLowerCase()}`}
            onCommit={(next) => {
              const vector: Vec3 = [...value]
              vector[index] = next
              onCommit(vector)
            }}
          />
        ))}
      </div>
    </Row>
  )
}

/** A colour swatch that opens a picker; the colour commits when a drag ends. */
function ColorRow({ label, value, onCommit, testId }: { label: string; value: readonly number[]; onCommit: (hex: string) => void; testId: string }) {
  const readOnly = useReadOnly()
  const hex = linearHex(value)
  const rgb = [Number.parseInt(hex.slice(1, 3), 16), Number.parseInt(hex.slice(3, 5), 16), Number.parseInt(hex.slice(5, 7), 16)] as const
  const [draft, setDraft] = useState<readonly [number, number, number] | null>(null)
  const shown = draft ?? rgb
  const toHex = (color: readonly number[]) => `#${color.map((channel) => Math.round(channel).toString(16).padStart(2, '0')).join('')}`
  return (
    <Row label={label}>
      <Popover>
        <PopoverTrigger asChild>
          <button type="button" className="scene-swatch" disabled={readOnly} aria-label={label} data-testid={testId}>
            <span style={{ background: toHex(shown) }} />
            <code>{toHex(shown)}</code>
          </button>
        </PopoverTrigger>
        <PopoverContent align="end" className="scene-color-popover">
          <ColorPicker
            value={shown}
            testId={`${testId}-picker`}
            onChange={(next, final) => {
              if (!final) return setDraft(next)
              setDraft(null)
              onCommit(toHex(next))
            }}
          />
        </PopoverContent>
      </Popover>
    </Row>
  )
}

// ---------------------------------------------------------------- outliner

function OutlinerRow({ object, info, depth }: { object: SceneObject; info: SceneInfo; depth: number }) {
  const { phrase: p, t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const [renaming, setRenaming] = useState(false)
  const Icon = TYPE_ICONS[object.type] ?? Move3dIcon
  const selected = info.selected.includes(object.id)
  const active = info.active === object.id
  const children = childrenOf(info.objects, object.id)
  return (
    <>
      <div className="scene-outliner-row" data-selected={selected || undefined} data-active={active || undefined} data-hidden={object.hidden || undefined} style={{ paddingLeft: 8 + depth * 14 }} data-testid="scene-outliner-item" data-name={object.name}>
        <button
          type="button"
          className="scene-outliner-name"
          onClick={(event) => run('select.objects', { objects: [object.id], extend: event.shiftKey || event.metaKey || event.ctrlKey })}
          onDoubleClick={() => !readOnly && setRenaming(true)}
          aria-pressed={selected}
        >
          <Icon size={14} strokeWidth={1.75} aria-hidden="true" className="scene-outliner-icon" data-type={object.type} />
          {renaming ? (
            <input
              className="kui-field scene-rename"
              defaultValue={object.name}
              autoFocus
              onClick={(event) => event.stopPropagation()}
              onKeyDown={(event) => {
                event.stopPropagation()
                if (event.key === 'Enter') event.currentTarget.blur()
                if (event.key === 'Escape') setRenaming(false)
              }}
              onBlur={(event) => {
                const name = event.currentTarget.value.trim()
                setRenaming(false)
                if (name && name !== object.name) run('object.rename', { object: object.id, name })
              }}
              data-testid="scene-rename"
            />
          ) : (
            <span className="scene-outliner-label">{object.name}</span>
          )}
          {object.id === info.camera ? <span className="scene-badge">{t('activeCamera')}</span> : null}
          <PresenceBadges object={object.id} label={(name, editing) => t(editing ? 'peerEditing' : 'peerSelected', { name })} />
        </button>
        <IconButton
          icon={object.hidden ? EyeOffIcon : EyeIcon}
          label={p(object.hidden ? 'Show Hidden' : 'Hide')}
          disabled={readOnly || info.mode === 'edit'}
          className="scene-outliner-eye"
          onClick={() => run(object.hidden ? 'object.reveal' : 'object.hide', { objects: [object.id] })}
          data-testid={`scene-outliner-eye-${object.name}`}
        />
      </div>
      {children.map((child) => (
        <OutlinerRow key={child.id} object={child} info={info} depth={depth + 1} />
      ))}
    </>
  )
}

function Outliner({ info }: { info: SceneInfo }) {
  const { phrase: p, t } = useKitEditor()
  const top = childrenOf(info.objects, null)
  return (
    <PanelSection title={p('Outliner')} testId="scene-outliner-section">
      <div className="scene-outliner" role="tree" aria-label={p('Outliner')} data-testid="scene-outliner">
        {top.length ? top.map((object) => <OutlinerRow key={object.id} object={object} info={info} depth={0} />) : <p className="kui-muted scene-empty">{t('emptyScene')}</p>}
      </div>
    </PanelSection>
  )
}

// ---------------------------------------------------------------- properties

function ObjectSection({ object, info }: { object: SceneObject; info: SceneInfo }) {
  const { phrase: p, t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const objects = [object.id]
  return (
    <PanelSection title={p('Object')} testId="scene-object">
      <Row label={t('name')}>
        <input
          key={object.name}
          className="kui-field scene-name"
          defaultValue={object.name}
          disabled={readOnly}
          onKeyDown={(event) => {
            if (event.key === 'Enter') event.currentTarget.blur()
          }}
          onBlur={(event) => {
            const name = event.currentTarget.value.trim()
            if (name && name !== object.name) run('object.rename', { object: object.id, name })
          }}
          data-testid="scene-name"
        />
      </Row>
      <VectorRow label={t('location')} value={object.location} unit="m" step={0.1} onCommit={(location) => run('object.transform', { objects, location })} testId="scene-location" animated={keyState(object, 'location')} />
      <VectorRow label={t('rotation')} value={object.rotation} unit="°" step={1} onCommit={(rotation) => run('object.transform', { objects, rotation })} testId="scene-rotation" animated={keyState(object, 'rotation')} />
      <VectorRow label={t('scale')} value={object.scale} step={0.1} onCommit={(scale) => run('object.transform', { objects, scale })} testId="scene-scale" animated={keyState(object, 'scale')} />
      {object.dimensions ? (
        <Row label={t('dimensions')}>
          <span className="scene-readout tabular-nums">{object.dimensions.map((value) => `${round(value, 3)} m`).join(' × ')}</span>
        </Row>
      ) : null}
      {object.mesh ? (
        <>
          <Row label={t('geometry')}>
            <span className="scene-readout tabular-nums" data-testid="scene-mesh-stats">
              {t('meshStats', { vertices: object.mesh.vertices, edges: object.mesh.edges, faces: object.mesh.faces })}
            </span>
          </Row>
          <Row label={t('shading')}>
            <Segmented
              label={t('shading')}
              value={object.mesh.smoothAngle != null ? 'auto' : object.mesh.smooth ? 'smooth' : 'flat'}
              disabled={readOnly || info.mode === 'edit'}
              segments={[
                { value: 'flat', label: p('Flat') },
                { value: 'smooth', label: p('Smooth') },
                { value: 'auto', label: p('Auto') },
              ]}
              onChange={(value) =>
                run(value === 'auto' ? 'object.shadeSmoothByAngle' : value === 'smooth' ? 'object.shadeSmooth' : 'object.shadeFlat', { objects })
              }
            />
          </Row>
        </>
      ) : null}
    </PanelSection>
  )
}

const MODIFIER_TYPES = [
  ['mirror', 'Mirror'],
  ['array', 'Array'],
  ['subdivision', 'Subdivision'],
  ['solidify', 'Solidify'],
  ['triangulate', 'Triangulate'],
  ['weld', 'Weld'],
  ['smooth', 'Smooth'],
  ['bevel', 'Bevel'],
  ['boolean', 'Boolean'],
  ['screw', 'Screw'],
  ['displace', 'Displace'],
  ['decimate', 'Decimate'],
  ['wireframe', 'Wireframe'],
  ['simpleDeform', 'SimpleDeform'],
  ['cast', 'Cast'],
  ['multires', 'Multires'],
  ['shrinkwrap', 'Shrinkwrap'],
  ['lattice', 'Lattice'],
  ['geometryNodes', 'Geometry Nodes'],
] as const

function ModifierCard({ object, info, modifier, index, count }: { object: SceneObject; info: SceneInfo; modifier: Modifier; index: number; count: number }) {
  const { phrase: p, t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const target = { object: object.id, index }
  const set = (values: Record<string, unknown>) => run('modifier.set', { ...target, ...values })
  const num = (key: string, label: string, options: { min?: number; max?: number; step?: number; unit?: string } = {}) => (
    <Row label={label}>
      <NumberInput label={label} value={Number(modifier[key] ?? 0)} min={options.min} max={options.max} step={options.step} unit={options.unit} onCommit={(value) => set({ [key]: value })} testId={`scene-modifier-${index}-${key}`} />
    </Row>
  )
  let fields: ReactNode = null
  switch (modifier.type) {
    case 'mirror': {
      const axes = (modifier.axes as boolean[] | undefined) ?? [true, false, false]
      fields = (
        <>
          <Row label={t('axes')}>
            <div className="scene-checks">
              {(['X', 'Y', 'Z'] as const).map((axis, slot) => (
                <Checkbox key={axis} label={axis} checked={axes[slot]} disabled={readOnly} onChange={(on) => set({ axes: axes.map((value, other) => (other === slot ? on : value)) })} />
              ))}
            </div>
          </Row>
          <Row label={t('merge')}>
            <Checkbox label={t('mergeSeam')} checked={modifier.merge !== false} disabled={readOnly} onChange={(merge) => set({ merge })} />
          </Row>
        </>
      )
      break
    }
    case 'array': {
      const offset = (modifier.relativeOffset as Vec3 | undefined) ?? [1, 0, 0]
      fields = (
        <>
          {num('count', t('count'), { min: 1, max: 1000, step: 1 })}
          <VectorRow label={t('relativeOffset')} value={offset} step={0.1} onCommit={(relativeOffset) => set({ relativeOffset })} testId={`scene-modifier-${index}-offset`} />
        </>
      )
      break
    }
    case 'subdivision':
      fields = num('levels', t('levels'), { min: 0, max: 6, step: 1 })
      break
    case 'solidify':
      fields = (
        <>
          {num('thickness', t('thickness'), { min: -100, max: 100, step: 0.01, unit: 'm' })}
          {num('offset', t('offset'), { min: -1, max: 1, step: 0.1 })}
        </>
      )
      break
    case 'weld':
      fields = num('distance', t('distance'), { min: 0, max: 10, step: 0.001, unit: 'm' })
      break
    case 'smooth':
      fields = (
        <>
          {num('factor', t('factor'), { min: -10, max: 10, step: 0.1 })}
          {num('iterations', t('iterations'), { min: 0, max: 100, step: 1 })}
        </>
      )
      break
    case 'bevel':
      fields = (
        <>
          {num('width', t('width'), { min: 0, max: 100, step: 0.01, unit: 'm' })}
          {num('segments', t('segments'), { min: 1, max: 64, step: 1 })}
          {num('angleLimit', t('angleLimit'), { min: 0, max: 180, step: 1, unit: '°' })}
        </>
      )
      break
    case 'boolean': {
      const others = info.objects.filter((other) => other.type === 'mesh' && other.id !== object.id)
      fields = (
        <>
          <Row label={t('operation')}>
            <Choice
              label={t('operation')}
              quiet
              value={String(modifier.operation ?? 'difference')}
              choices={[['difference', t('difference')], ['union', t('union')], ['intersect', t('intersect')]]}
              disabled={readOnly}
              testId={`scene-modifier-${index}-operation`}
              onChange={(operation) => set({ operation })}
            />
          </Row>
          <Row label={t('operand')}>
            <Choice
              label={t('operand')}
              quiet
              value={modifier.operand != null ? String(modifier.operand) : 'none'}
              choices={[['none', t('noOperand')], ...others.map((other) => [String(other.id), other.name] as const)]}
              disabled={readOnly}
              testId={`scene-modifier-${index}-operand`}
              onChange={(value) => value !== 'none' && set({ operand: Number(value) })}
            />
          </Row>
        </>
      )
      break
    }
    case 'screw':
      fields = (
        <>
          {num('steps', t('steps'), { min: 1, max: 1000, step: 1 })}
          {num('angle', t('angle'), { min: -3600, max: 3600, step: 1, unit: '°' })}
          {num('screwOffset', t('screwOffset'), { min: -100, max: 100, step: 0.1, unit: 'm' })}
          <Row label={t('axis')}>
            <Choice
              label={t('axis')}
              quiet
              value={String(modifier.axis ?? 'z')}
              choices={[['x', 'X'], ['y', 'Y'], ['z', 'Z']]}
              disabled={readOnly}
              onChange={(axis) => set({ axis })}
            />
          </Row>
        </>
      )
      break
    case 'displace':
      fields = (
        <>
          {num('strength', t('strength'), { min: -100, max: 100, step: 0.1, unit: 'm' })}
          {num('midLevel', t('midLevel'), { min: -10, max: 10, step: 0.05 })}
          {num('noiseScale', t('noiseScale'), { min: 0, max: 100, step: 0.1, unit: 'm' })}
          <Row label={t('direction')}>
            <Choice
              label={t('direction')}
              quiet
              value={String(modifier.direction ?? 'normal')}
              choices={[['normal', t('normalDirection')], ['x', 'X'], ['y', 'Y'], ['z', 'Z']]}
              disabled={readOnly}
              onChange={(direction) => set({ direction })}
            />
          </Row>
          <DisplaceImageFields object={object} info={info} modifier={modifier} index={index} />
        </>
      )
      break
    case 'wireframe':
      fields = (
        <>
          {num('thickness', t('thickness'), { min: 0, max: 100, step: 0.01, unit: 'm' })}
          {num('offset', t('offset'), { min: -1, max: 1, step: 0.1 })}
        </>
      )
      break
    case 'simpleDeform': {
      const twisting = modifier.mode === 'twist' || modifier.mode === 'bend' || modifier.mode == null
      fields = (
        <>
          <Row label={t('deformMode')}>
            <Choice
              label={t('deformMode')}
              quiet
              value={String(modifier.mode ?? 'twist')}
              choices={[['twist', t('twist')], ['bend', t('bend')], ['taper', t('taper')], ['stretch', t('stretch')]]}
              disabled={readOnly}
              testId={`scene-modifier-${index}-mode`}
              onChange={(mode) => set({ mode })}
            />
          </Row>
          {twisting ? num('angle', t('angle'), { min: -3600, max: 3600, step: 1, unit: '°' }) : num('factor', t('factor'), { min: -100, max: 100, step: 0.05 })}
          <Row label={t('axis')}>
            <Choice label={t('axis')} quiet value={String(modifier.axis ?? 'x')} choices={[['x', 'X'], ['y', 'Y'], ['z', 'Z']]} disabled={readOnly} onChange={(axis) => set({ axis })} />
          </Row>
        </>
      )
      break
    }
    case 'cast':
      fields = num('factor', t('factor'), { min: -10, max: 10, step: 0.1 })
      break
    case 'multires':
      fields = <MultiresFields object={object} info={info} modifier={modifier} index={index} />
      break
    case 'geometryNodes':
      fields = <GeometryNodesFields object={object} info={info} modifier={modifier} index={index} />
      break
    default:
      fields = moreModifierFields({ object, info, modifier, index })
      break
  }
  return (
    <div className="scene-modifier" data-enabled={modifier.enabled} data-testid={`scene-modifier-${index}`}>
      <div className="scene-modifier-header">
        <Checkbox label={<span className="scene-modifier-name">{modifier.name}</span>} checked={modifier.enabled} disabled={readOnly} onChange={(enabled) => set({ enabled })} />
        <span className="scene-modifier-actions">
          <IconButton icon={ArrowUpIcon} label={p('Move Up')} disabled={readOnly || index === 0} onClick={() => run('modifier.move', { ...target, to: index - 1 })} />
          <IconButton icon={ArrowDownIcon} label={p('Move Down')} disabled={readOnly || index === count - 1} onClick={() => run('modifier.move', { ...target, to: index + 1 })} />
          <IconButton icon={CheckIcon} label={p('Apply')} disabled={readOnly} onClick={() => run('modifier.apply', target)} data-testid={`scene-modifier-${index}-apply`} />
          <IconButton icon={TrashIcon} label={p('Remove')} disabled={readOnly} onClick={() => run('modifier.remove', target)} data-testid={`scene-modifier-${index}-remove`} />
        </span>
      </div>
      {fields ? <div className="scene-modifier-body">{fields}</div> : null}
    </div>
  )
}

function ModifiersSection({ object, info }: { object: SceneObject; info: SceneInfo }) {
  const { phrase: p, t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  return (
    <PanelSection
      title={p('Modifiers')}
      testId="scene-modifiers"
      actions={
        <MenuButton
          align="end"
          trigger={({ open, toggle }) => <IconButton icon={PlusIcon} label={p('Add Modifier')} aria-expanded={open} disabled={readOnly || info.mode === 'edit'} onClick={toggle} data-testid="scene-add-modifier" />}
        >
          {MODIFIER_TYPES.map(([type, label]) => (
            <MenuItem key={type} label={p(label)} onSelect={() => run('modifier.add', { object: object.id, type })} testId={`scene-add-modifier-${type}`} />
          ))}
        </MenuButton>
      }
    >
      {object.modifiers.length ? (
        <div className="scene-modifiers">
          {object.modifiers.map((modifier, index) => (
            <ModifierCard key={`${index}-${modifier.name}`} object={object} info={info} modifier={modifier} index={index} count={object.modifiers.length} />
          ))}
        </div>
      ) : (
        <p className="kui-muted scene-empty">{t('noModifiers')}</p>
      )}
    </PanelSection>
  )
}

function MaterialSection({ object, info }: { object: SceneObject; info: SceneInfo }) {
  const { phrase: p, t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const material = info.materials.find((item) => item.id === object.material) ?? null
  const choices: Array<readonly [string, string]> = [['none', t('noMaterial')], ...info.materials.map((item) => [String(item.id), item.name] as const), ['new', t('newMaterial')]]
  return (
    <PanelSection title={p('Material')} testId="scene-material">
      <Row label={t('material')}>
        <Choice
          label={t('material')}
          quiet
          value={material ? String(material.id) : 'none'}
          choices={choices}
          disabled={readOnly}
          testId="scene-material-choice"
          onChange={(value) => {
            // A new material goes to the selection, which holds the active object.
            if (value === 'new') run('material.new', {})
            else if (value === 'none') run('material.clear', { objects: [object.id] })
            else run('material.assign', { material: Number(value), objects: [object.id] })
          }}
        />
      </Row>
      {material ? (
        <>
          <ColorRow label={t('baseColor')} value={material.baseColor} onCommit={(baseColor) => run('material.set', { material: material.id, baseColor })} testId="scene-base-color" />
          <div className="scene-track">
            <NumberInput label={t('metallic')} value={material.metallic} min={0} max={1} step={0.01} track onCommit={(metallic) => run('material.set', { material: material.id, metallic })} testId="scene-metallic" />
            <NumberInput label={t('roughness')} value={material.roughness} min={0} max={1} step={0.01} track onCommit={(roughness) => run('material.set', { material: material.id, roughness })} testId="scene-roughness" />
          </div>
          <Row label={t('baseColorImage')}>
            <TextureSlot material={material} info={info} slot="baseColor" label={t('baseColorImage')} />
          </Row>
          <Row label={t('metallicRoughnessImage')}>
            <TextureSlot material={material} info={info} slot="metallicRoughness" label={t('metallicRoughnessImage')} />
          </Row>
          <Row label={t('normalImage')}>
            <TextureSlot material={material} info={info} slot="normal" label={t('normalImage')} />
          </Row>
          <Row label={t('shaderNodes')}>
            <Choice
              label={t('shaderNodes')}
              quiet
              value={material.shader != null ? String(material.shader) : 'none'}
              disabled={readOnly}
              choices={[['none', t('noneChoice')], ...info.nodeGroups.filter((group) => group.tree === 'shader').map((group) => [String(group.id), group.name] as const), ['new', t('newShader')]]}
              testId="scene-material-shader"
              onChange={(value) =>
                value === 'new' ? run('nodes.newGroup', { tree: 'shader', material: material.id }) : run('material.set', { material: material.id, shader: value === 'none' ? null : Number(value) })
              }
            />
          </Row>
          <div className="scene-track">
            <NumberInput label={t('transmission')} value={material.transmission ?? 0} min={0} max={1} step={0.01} track onCommit={(transmission) => run('material.set', { material: material.id, transmission })} testId="scene-transmission" />
            <NumberInput label={t('ior')} value={material.ior ?? 1.5} min={1} max={10} step={0.01} onCommit={(ior) => run('material.set', { material: material.id, ior })} testId="scene-ior" />
          </div>
          <Row label={t('alpha')}>
            <Choice
              label={t('alphaMode')}
              quiet
              value={material.alphaMode ?? 'opaque'}
              disabled={readOnly}
              choices={[
                ['opaque', t('alphaOpaque')],
                ['clip', t('alphaClip')],
                ['blend', t('alphaBlend')],
              ]}
              testId="scene-alpha-mode"
              onChange={(alphaMode) => run('material.set', { material: material.id, alphaMode })}
            />
          </Row>
          {(material.alphaMode ?? 'opaque') !== 'opaque' ? (
            <div className="scene-track">
              <NumberInput
                label={t('alpha')}
                value={material.baseColor[3]}
                min={0}
                max={1}
                step={0.01}
                track
                onCommit={(alpha) => run('material.set', { material: material.id, baseColor: [material.baseColor[0], material.baseColor[1], material.baseColor[2], alpha] })}
                testId="scene-alpha"
              />
              {material.alphaMode === 'clip' ? (
                <NumberInput label={t('alphaCutoff')} value={material.alphaCutoff ?? 0.5} min={0} max={1} step={0.01} onCommit={(alphaCutoff) => run('material.set', { material: material.id, alphaCutoff })} testId="scene-alpha-cutoff" />
              ) : null}
            </div>
          ) : null}
        </>
      ) : null}
    </PanelSection>
  )
}

function LightSection({ object }: { object: SceneObject }) {
  const { phrase: p, t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const light = object.light
  if (!light) return null
  const target = { object: object.id }
  return (
    <PanelSection title={p('Light')} testId="scene-light">
      <Row label={t('kind')}>
        <Choice
          label={t('kind')}
          quiet
          value={light.kind}
          disabled={readOnly}
          choices={[
            ['point', p('Point')],
            ['sun', p('Sun')],
            ['spot', p('Spot')],
            ['area', p('Area')],
          ]}
          onChange={(kind) => run('light.set', { ...target, kind })}
        />
      </Row>
      <ColorRow label={t('color')} value={light.color} onCommit={(color) => run('light.set', { ...target, color })} testId="scene-light-color" />
      <Row label={t(light.kind === 'sun' ? 'strength' : 'power')}>
        <NumberInput label={t('power')} value={light.power} min={0} max={1e6} step={light.kind === 'sun' ? 0.1 : 10} unit={light.kind === 'sun' ? 'W/m²' : 'W'} onCommit={(power) => run('light.set', { ...target, power })} testId="scene-light-power" />
      </Row>
      {light.kind === 'spot' ? (
        <Row label={t('spotSize')}>
          <NumberInput label={t('spotSize')} value={light.spotSize} min={1} max={180} step={1} unit="°" onCommit={(spotSize) => run('light.set', { ...target, spotSize })} />
        </Row>
      ) : null}
    </PanelSection>
  )
}

function CameraSection({ object, info }: { object: SceneObject; info: SceneInfo }) {
  const { phrase: p, t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const camera = object.camera
  if (!camera) return null
  const target = { object: object.id }
  return (
    <PanelSection title={p('Camera')} testId="scene-camera">
      <Row label={t('projection')}>
        <Segmented
          label={t('projection')}
          value={camera.orthographic ? 'ortho' : 'persp'}
          disabled={readOnly}
          segments={[
            { value: 'persp', label: p('Perspective') },
            { value: 'ortho', label: p('Orthographic') },
          ]}
          onChange={(value) => run('camera.set', { ...target, orthographic: value === 'ortho' })}
        />
      </Row>
      {camera.orthographic ? (
        <Row label={t('orthoScale')}>
          <NumberInput label={t('orthoScale')} value={camera.orthoScale} testId="scene-camera-ortho-scale" min={0.001} max={1e5} step={0.1} unit="m" onCommit={(orthoScale) => run('camera.set', { ...target, orthoScale })} />
        </Row>
      ) : (
        <Row label={t('lens')}>
          <NumberInput label={t('lens')} value={camera.lens} min={1} max={5000} step={1} unit="mm" onCommit={(lens) => run('camera.set', { ...target, lens })} testId="scene-camera-lens" />
        </Row>
      )}
      <Row label={t('sensorFit')}>
        <Choice label={t('sensorFit')} quiet value={camera.sensorFit} disabled={readOnly}
          choices={[["auto", t('sensorFitAuto')], ["horizontal", t('sensorFitHorizontal')], ["vertical", t('sensorFitVertical')]]}
          testId="scene-camera-sensor-fit" onChange={(sensorFit) => run('camera.set', { ...target, sensorFit })} />
      </Row>
      {!camera.orthographic ? (
        <Row label={camera.sensorFit === 'vertical' ? t('sensorHeight') : t('sensorWidth')}>
          {camera.sensorFit === 'vertical' ? (
            <NumberInput label={t('sensorHeight')} value={camera.sensorHeight} min={1} max={200} step={1} unit="mm" testId="scene-camera-sensor-height"
              onCommit={(sensorHeight) => run('camera.set', { ...target, sensorHeight })} />
          ) : (
            <NumberInput label={t('sensorWidth')} value={camera.sensor} min={1} max={200} step={1} unit="mm" testId="scene-camera-sensor-width"
              onCommit={(sensor) => run('camera.set', { ...target, sensor })} />
          )}
        </Row>
      ) : null}
      <p className="kui-muted">{t('sensorFitHint')}</p>
      <Row label={t('clip')}>
        <div className="scene-vector scene-vector-2">
          <NumberInput label={t('clipStart')} value={camera.clipStart} min={0.0001} max={1e5} step={0.01} unit="m" onCommit={(clipStart) => run('camera.set', { ...target, clipStart })} />
          <NumberInput label={t('clipEnd')} value={camera.clipEnd} min={0.001} max={1e6} step={1} unit="m" onCommit={(clipEnd) => run('camera.set', { ...target, clipEnd })} />
        </div>
      </Row>
      {!camera.orthographic ? (
        <>
          <Row label={t('depthOfField')}>
            <Checkbox label={t('depthOfFieldOn')} checked={camera.depthOfField === true} disabled={readOnly} onChange={(depthOfField) => run('camera.set', { ...target, depthOfField })} testId="scene-dof" />
          </Row>
          {camera.depthOfField ? (
            <>
              <Row label={t('focusObject')}>
                <Choice
                  label={t('focusObject')}
                  quiet
                  value={camera.focusObject != null ? String(camera.focusObject) : 'none'}
                  disabled={readOnly}
                  choices={[['none', t('focusDistanceOnly')], ...info.objects.filter((other) => other.id !== object.id).map((other) => [String(other.id), other.name] as const)]}
                  testId="scene-focus-object"
                  onChange={(value) => run('camera.set', { ...target, focusObject: value === 'none' ? null : Number(value) })}
                />
              </Row>
              <div className="scene-track">
                <NumberInput label={t('fStop')} value={camera.fStop ?? 2.8} min={0.01} max={1000} step={0.1} onCommit={(fStop) => run('camera.set', { ...target, fStop })} testId="scene-fstop" />
                {camera.focusObject == null ? (
                  <NumberInput label={t('focusDistance')} value={camera.focusDistance ?? 10} min={0.001} max={1e6} step={0.1} unit="m" onCommit={(focusDistance) => run('camera.set', { ...target, focusDistance })} testId="scene-focus-distance" />
                ) : null}
              </div>
              <div className="scene-track">
                <NumberInput
                  label={t('bokehBlades')}
                  value={camera.blades ?? 0}
                  min={0}
                  max={64}
                  step={1}
                  onCommit={(value) => {
                    // 0 is a round opening; a polygon has at least three blades.
                    const blades = Math.round(value)
                    run('camera.set', { ...target, blades: blades < 3 ? (blades > (camera.blades ?? 0) ? 3 : 0) : blades })
                  }}
                  testId="scene-bokeh-blades"
                />
                <NumberInput label={t('bladeRotation')} value={camera.bladeRotation ?? 0} min={-360} max={360} step={1} unit="°" onCommit={(bladeRotation) => run('camera.set', { ...target, bladeRotation })} testId="scene-bokeh-rotation" />
                <NumberInput label={t('bokehRatio')} value={camera.ratio ?? 1} min={0.01} max={100} step={0.05} onCommit={(ratio) => run('camera.set', { ...target, ratio })} testId="scene-bokeh-ratio" />
              </div>
            </>
          ) : null}
        </>
      ) : null}
      {info.camera !== object.id ? (
        <Row label="">
          <button type="button" className="kui-button" disabled={readOnly} onClick={() => run('object.setCamera', target)} data-testid="scene-set-camera">
            {p('Set Active Camera')}
          </button>
        </Row>
      ) : null}
    </PanelSection>
  )
}

function SceneSection({ info }: { info: SceneInfo }) {
  const { phrase: p, t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const cameras = info.objects.filter((object) => object.type === 'camera')
  const shaderGroups = info.nodeGroups.filter((group) => group.tree === 'shader')
  return (
    <PanelSection title={p('Scene')} defaultOpen={false} testId="scene-settings">
      <Row label={t('renderSize')}>
        <div className="scene-vector scene-vector-2">
          <NumberInput label="W" value={info.render.width} min={1} max={8192} step={1} unit="px" onCommit={(width) => run('render.set', { width: Math.round(width) })} testId="scene-render-width" />
          <NumberInput label="H" value={info.render.height} min={1} max={8192} step={1} unit="px" onCommit={(height) => run('render.set', { height: Math.round(height) })} testId="scene-render-height" />
        </div>
      </Row>
      <CompositingRows info={info} />
      <Row label={t('ambientOcclusion')}>
        <Checkbox label={t('ambientOcclusionOn')} checked={info.render.ao} disabled={readOnly} onChange={(ao) => run('render.set', { ao })} testId="scene-ao" />
      </Row>
      {info.render.ao ? (
        <div className="scene-track">
          <Checkbox
            label={t('aoAuto')}
            checked={info.render.aoDistance == null}
            disabled={readOnly}
            onChange={(auto) => run('render.set', { aoDistance: auto ? null : 0.2 })}
            testId="scene-ao-auto"
          />
          {info.render.aoDistance != null ? (
            <NumberInput label={t('aoDistance')} value={info.render.aoDistance} min={0.001} max={1000} step={0.05} unit="m" onCommit={(aoDistance) => run('render.set', { aoDistance })} testId="scene-ao-distance" />
          ) : null}
          <NumberInput label={t('aoStrength')} value={info.render.aoStrength} min={0} max={4} step={0.05} onCommit={(aoStrength) => run('render.set', { aoStrength })} testId="scene-ao-strength" />
        </div>
      ) : null}
      <Row label={t('viewTransform')}>
        <Choice
          label={t('viewTransform')}
          quiet
          value={info.render.view}
          disabled={readOnly}
          choices={[
            ['standard', t('viewStandard')],
            ['agx', t('viewAgx')],
            ['khronos-pbr-neutral', t('viewKhronos')],
          ]}
          testId="scene-view-transform"
          onChange={(view) => run('render.set', { view })}
        />
      </Row>
      <Row label={t('look')}>
        <Choice
          label={t('look')}
          quiet
          value={info.render.look}
          disabled={readOnly}
          choices={[
            ['none', t('lookNone')],
            ['punchy', t('lookPunchy')],
            ['greyscale', t('lookGreyscale')],
            ['high-contrast', t('lookHighContrast')],
            ['low-contrast', t('lookLowContrast')],
          ]}
          testId="scene-look"
          onChange={(look) => run('render.set', { look })}
        />
      </Row>
      <Row label={t('exposure')}>
        <NumberInput label={t('exposure')} value={info.render.exposure} min={-32} max={32} step={0.1} onCommit={(exposure) => run('render.set', { exposure })} testId="scene-exposure" />
      </Row>
      <Row label={t('motionBlur')}>
        <Checkbox label={t('motionBlurOn')} checked={info.render.motionBlur} disabled={readOnly} onChange={(motionBlur) => run('render.set', { motionBlur })} testId="scene-motion-blur" />
      </Row>
      {info.render.motionBlur ? (
        <div className="scene-track">
          <NumberInput label={t('shutter')} value={info.render.shutter} min={0.01} max={100} step={0.05} onCommit={(shutter) => run('render.set', { shutter })} testId="scene-shutter" />
          <Choice
            label={t('shutterPosition')}
            quiet
            value={info.render.shutterPosition}
            disabled={readOnly}
            choices={[
              ['center', t('shutterCenter')],
              ['start', t('shutterStart')],
              ['end', t('shutterEnd')],
            ]}
            testId="scene-shutter-position"
            onChange={(shutterPosition) => run('render.set', { shutterPosition })}
          />
        </div>
      ) : null}
      <Row label={t('camera')}>
        <Choice
          label={t('camera')}
          quiet
          value={info.camera != null ? String(info.camera) : 'none'}
          disabled={readOnly || cameras.length === 0}
          choices={info.camera == null ? [['none', t('noCamera')], ...cameras.map((camera) => [String(camera.id), camera.name] as const)] : cameras.map((camera) => [String(camera.id), camera.name] as const)}
          onChange={(value) => value !== 'none' && run('object.setCamera', { object: Number(value) })}
        />
      </Row>
      <Row label={t('worldImage')}>
        <WorldImageSlot info={info} />
      </Row>
      {info.world.image ? (
        <Row label={t('worldRotation')}>
          <NumberInput label={t('worldRotation')} value={info.world.rotation} min={-360} max={360} step={1} unit="°" onCommit={(rotation) => run('world.set', { rotation })} testId="scene-world-rotation" />
        </Row>
      ) : (
        <ColorRow label={t('world')} value={info.world.color} onCommit={(color) => run('world.set', { color })} testId="scene-world-color" />
      )}
      <Row label={t('strength')}>
        <NumberInput label={t('strength')} value={info.world.strength} min={0} max={100} step={0.1} onCommit={(strength) => run('world.set', { strength })} />
      </Row>
      <Row label={t('worldVolume')}>
        <Choice
          label={t('worldVolume')}
          quiet
          value={info.world.volume != null ? String(info.world.volume) : 'none'}
          disabled={readOnly}
          choices={[['none', t('noneChoice')], ...shaderGroups.map((group) => [String(group.id), group.name] as const), ['new', t('newVolume')]]}
          testId="scene-world-volume"
          onChange={(value) => (value === 'new' ? run('nodes.newGroup', { tree: 'shader', world: true }) : run('world.set', { volume: value === 'none' ? null : Number(value) }))}
        />
      </Row>
    </PanelSection>
  )
}

/** The Scene kit's panels beside the viewport: Outliner, the active object's properties, Scene. */
export function SidePanel({ info }: { info: SceneInfo }) {
  const { t } = useKitEditor()
  const active = info.objects.find((object) => object.id === info.active) ?? null
  return (
    <aside className="kui-panel scene-side" data-side="right" data-testid="scene-side">
      <div className="kui-panel-body">
        <Outliner info={info} />
        {active ? (
          <>
            <ObjectSection key={`object-${active.id}`} object={active} info={info} />
            <SculptSections info={info} />
            {active.type === 'mesh' ? <ModifiersSection object={active} info={info} /> : null}
            {active.type === 'mesh' ? <ParticlesSection object={active} info={info} /> : null}
            <ShapeSections object={active} info={info} />
            {active.type === 'lattice' ? <LatticeSection key={`lattice-${active.id}`} object={active} info={info} /> : null}
            {active.type === 'mesh' || active.type === 'curve' || active.type === 'text' ? <MaterialSection object={active} info={info} /> : null}
            {active.type === 'light' ? <LightSection object={active} /> : null}
            {active.type === 'camera' ? <CameraSection object={active} info={info} /> : null}
            <RigSections object={active} info={info} />
            <ConstraintsSection key={`constraints-${active.id}`} object={active} info={info} />
            <DriversSection key={`drivers-${active.id}`} object={active} info={info} />
            <PhysicsSection key={`physics-${active.id}`} object={active} info={info} />
          </>
        ) : (
          <p className="kui-muted scene-empty scene-no-active">{t('nothingSelected')}</p>
        )}
        <SceneSection info={info} />
        <ActionsSection info={info} />
        <KeyingSetsSection info={info} />
        <PhysicsWorldSection info={info} />
      </div>
    </aside>
  )
}
