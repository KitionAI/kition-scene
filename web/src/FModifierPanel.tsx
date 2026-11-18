import { PlusIcon, TrashIcon } from '@kition/tessera-icons/react'
import { Button, Checkbox, Choice, IconButton, MenuButton, MenuItem, useKitEditor, useReadOnly } from '@kition/kit-editor'
import type { FModifierInfo } from './AnimationData'
import { NumberInput, Row } from './SidePanel'

const KINDS = ['generator', 'function', 'envelope', 'cycles', 'noise', 'stepped', 'limits'] as const
const CYCLES = ['none', 'repeat', 'repeatOffset', 'mirror'] as const

/** The channel a modifier command names. */
export type ChannelRef = { target: number; property: string; index: number; path: string }

const num = (value: unknown, fallback = 0) => (typeof value === 'number' && Number.isFinite(value) ? value : fallback)

function Fields({ modifier, set, frame }: { modifier: FModifierInfo; set: (values: Record<string, unknown>) => void; frame: number }) {
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  const number = (field: string, label: string, options: { min?: number; max?: number; step?: number } = {}) => (
    <Row label={label} key={field}>
      <NumberInput label={label} value={num(modifier[field])} min={options.min} max={options.max} step={options.step ?? 0.1} onCommit={(value) => set({ [field]: value })} testId={`scene-fmod-${field}`} />
    </Row>
  )
  const toggle = (field: string, label: string) => (
    <Checkbox key={field} label={label} checked={modifier[field] === true} disabled={readOnly} onChange={(on) => set({ [field]: on })} testId={`scene-fmod-${field}`} />
  )
  switch (modifier.type) {
    case 'generator': {
      const coefficients = Array.isArray(modifier.coefficients) ? (modifier.coefficients as number[]) : []
      const factorized = modifier.mode === 'factorized'
      return (
        <>
          <Row label={t('fmodMode')}>
            <Choice
              label={t('fmodMode')}
              quiet
              value={factorized ? 'factorized' : 'expanded'}
              disabled={readOnly}
              choices={(['expanded', 'factorized'] as const).map((mode) => [mode, t(`fmode_${mode}`)] as const)}
              onChange={(mode) => set({ mode, coefficients: mode === 'factorized' && coefficients.length % 2 === 1 ? [...coefficients, 0] : coefficients })}
            />
            {toggle('additive', t('fmodAdditive'))}
          </Row>
          <Row label={t('fmodCoefficients')}>
            {coefficients.map((value, at) => (
              <NumberInput key={at} label={`c${at}`} value={value} step={0.1} onCommit={(next) => set({ coefficients: coefficients.map((old, slot) => (slot === at ? next : old)) })} testId={`scene-fmod-coefficient-${at}`} />
            ))}
            <IconButton icon={PlusIcon} label={t('fmodAddCoefficient')} disabled={readOnly || coefficients.length >= 22} onClick={() => set({ coefficients: [...coefficients, ...(factorized ? [1, 0] : [0])] })} />
          </Row>
        </>
      )
    }
    case 'function':
      return (
        <>
          <Row label={t('fmodFunction')}>
            <Choice
              label={t('fmodFunction')}
              quiet
              value={String(modifier.function ?? 'sin')}
              disabled={readOnly}
              choices={(['sin', 'cos', 'tan', 'sqrt', 'ln', 'sinc'] as const).map((fn) => [fn, t(`ffn_${fn}`)] as const)}
              onChange={(fn) => set({ function: fn })}
              testId="scene-fmod-function"
            />
            {toggle('additive', t('fmodAdditive'))}
          </Row>
          {number('amplitude', t('fmodAmplitude'))}
          {number('phaseMultiplier', t('fmodPhaseMultiplier'))}
          {number('phaseOffset', t('fmodPhaseOffset'))}
          {number('valueOffset', t('fmodValueOffset'))}
        </>
      )
    case 'envelope': {
      const points = Array.isArray(modifier.points) ? (modifier.points as { frame: number; min: number; max: number }[]) : []
      const setPoints = (next: { frame: number; min: number; max: number }[]) => set({ points: [...next].sort((a, b) => a.frame - b.frame) })
      return (
        <>
          {number('reference', t('fmodReference'))}
          {number('min', t('fmodMin'))}
          {number('max', t('fmodMax'))}
          <Row label={t('fmodPoints')}>
            <Button size="sm" disabled={readOnly || points.some((point) => point.frame === frame)} onClick={() => setPoints([...points, { frame, min: num(modifier.min, -1), max: num(modifier.max, 1) }])} data-testid="scene-fmod-add-point">
              {t('fmodAddPoint')}
            </Button>
          </Row>
          {points.map((point, at) => (
            <Row label={`${t('fmodPointFrame')} ${point.frame}`} key={`${point.frame}-${at}`}>
              <NumberInput label={t('fmodMin')} value={point.min} step={0.1} onCommit={(min) => setPoints(points.map((old, slot) => (slot === at ? { ...old, min } : old)))} />
              <NumberInput label={t('fmodMax')} value={point.max} step={0.1} onCommit={(max) => setPoints(points.map((old, slot) => (slot === at ? { ...old, max } : old)))} />
              <IconButton icon={TrashIcon} label={t('fmodRemovePoint')} disabled={readOnly} onClick={() => setPoints(points.filter((_, slot) => slot !== at))} />
            </Row>
          ))}
        </>
      )
    }
    case 'cycles':
      return (
        <>
          <Row label={t('fmodBefore')}>
            <Choice label={t('fmodBefore')} quiet value={String(modifier.before ?? 'repeat')} disabled={readOnly} choices={CYCLES.map((mode) => [mode, t(`fcycle_${mode}`)] as const)} onChange={(before) => set({ before })} testId="scene-fmod-before" />
            <NumberInput label={t('fmodBeforeCount')} value={num(modifier.beforeCount)} min={0} step={1} onCommit={(count) => set({ beforeCount: Math.round(count) })} />
          </Row>
          <Row label={t('fmodAfter')}>
            <Choice label={t('fmodAfter')} quiet value={String(modifier.after ?? 'repeat')} disabled={readOnly} choices={CYCLES.map((mode) => [mode, t(`fcycle_${mode}`)] as const)} onChange={(after) => set({ after })} testId="scene-fmod-after" />
            <NumberInput label={t('fmodAfterCount')} value={num(modifier.afterCount)} min={0} step={1} onCommit={(count) => set({ afterCount: Math.round(count) })} />
          </Row>
        </>
      )
    case 'noise':
      return (
        <>
          <Row label={t('fmodNoiseBlend')}>
            <Choice
              label={t('fmodNoiseBlend')}
              quiet
              value={String(modifier.blend ?? 'replace')}
              disabled={readOnly}
              choices={(['replace', 'add', 'subtract', 'multiply'] as const).map((blend) => [blend, t(`fnoise_${blend}`)] as const)}
              onChange={(blend) => set({ blend })}
            />
          </Row>
          {number('scale', t('fmodScale'))}
          {number('strength', t('fmodStrength'))}
          {number('phase', t('fmodPhase'))}
          {number('offset', t('fmodOffset'))}
          <Row label={t('fmodDepth')}>
            <NumberInput label={t('fmodDepth')} value={num(modifier.depth)} min={0} max={32} step={1} onCommit={(depth) => set({ depth: Math.round(depth) })} />
          </Row>
          {number('lacunarity', t('fmodLacunarity'), { min: 0 })}
          {number('roughness', t('fmodRoughness'), { min: 0 })}
        </>
      )
    case 'limits':
      return <>{(['minX', 'maxX', 'minY', 'maxY'] as const).map((field) => (
        <Row key={field} label={t(`fmod_${field}`)}>
          <Checkbox label={t(`fmod_${field}`)} checked={typeof modifier[field] === 'number'} disabled={readOnly} onChange={(on) => set({ [field]: on ? (field.endsWith('X') ? frame : 0) : null })} testId={`scene-fmod-use-${field}`} />
          {typeof modifier[field] === 'number' ? <NumberInput label={t(`fmod_${field}`)} value={num(modifier[field])} step={field.endsWith('X') ? 1 : 0.1} onCommit={(value) => set({ [field]: value })} testId={`scene-fmod-${field}`} /> : null}
        </Row>
      ))}</>
    case 'stepped':
      return (
        <>
          {number('step', t('fmodStep'), { min: 0.01, step: 1 })}
          {number('offset', t('fmodOffset'), { step: 1 })}
          <Row label={t('fmodUseStart')}>
            <Checkbox label={t('fmodUseStart')} checked={typeof modifier.start === 'number'} disabled={readOnly} onChange={(on) => set({ start: on ? frame : null })} />
            {typeof modifier.start === 'number' ? <NumberInput label={t('fmodUseStart')} value={modifier.start} step={1} onCommit={(start) => set({ start })} /> : null}
          </Row>
          <Row label={t('fmodUseEnd')}>
            <Checkbox label={t('fmodUseEnd')} checked={typeof modifier.end === 'number'} disabled={readOnly} onChange={(on) => set({ end: on ? frame : null })} />
            {typeof modifier.end === 'number' ? <NumberInput label={t('fmodUseEnd')} value={modifier.end} step={1} onCommit={(end) => set({ end })} /> : null}
          </Row>
        </>
      )
    default:
      return null
  }
}

/**
 * The picked curve's F-curve modifier stack: add one from the menu, change its settings, its
 * frame range (with blends in and out) and influence, mute or remove it. Each change is one
 * command and one undo step.
 */
export function FModifierPanel({ channel, modifiers, frame, label }: { channel: ChannelRef; modifiers: FModifierInfo[]; frame: number; label: string }) {
  const { run, t } = useKitEditor()
  const readOnly = useReadOnly()
  return (
    <div className="scene-fmods" data-testid="scene-fmods">
      <div className="scene-fmods-head">
        <strong>{t('fmodModifiers')}</strong>
        <span className="kui-muted">{label}</span>
        <span style={{ flex: 1 }} />
        <MenuButton
          align="end"
          trigger={({ open, toggle }) => (
            <Button size="sm" aria-expanded={open} disabled={readOnly} onClick={toggle} data-testid="scene-fmod-add">
              {t('fmodAdd')}
            </Button>
          )}
        >
          {KINDS.map((kind) => (
            <MenuItem key={kind} label={t(`fmod_${kind}`)} onSelect={() => run('graph.addModifier', { ...channel, type: kind })} testId={`scene-fmod-add-${kind}`} />
          ))}
        </MenuButton>
      </div>
      {modifiers.map((modifier, at) => {
        const set = (values: Record<string, unknown>) => run('graph.setModifier', { ...channel, modifier: at, ...values })
        const range = modifier.range ?? null
        return (
          <div className="scene-fmod" key={`${at}-${modifier.type}`} data-muted={modifier.muted || undefined} data-testid={`scene-fmod-${at}`}>
            <div className="scene-fmod-head">
              <strong>{t(`fmod_${modifier.type}`)}</strong>
              <span style={{ flex: 1 }} />
              <Checkbox label={t('fmodMute')} checked={modifier.muted === true} disabled={readOnly} onChange={(muted) => set({ muted })} />
              <IconButton icon={TrashIcon} label={t('fmodRemove')} disabled={readOnly} onClick={() => run('graph.removeModifier', { ...channel, modifier: at })} data-testid={`scene-fmod-${at}-remove`} />
            </div>
            <div className="scene-fmod-fields">
              <Fields modifier={modifier} set={set} frame={frame} />
              <Row label={t('fmodUseInfluence')}>
                <Checkbox label={t('fmodUseInfluence')} checked={typeof modifier.influence === 'number'} disabled={readOnly} onChange={(on) => set({ influence: on ? 1 : null })} />
                {typeof modifier.influence === 'number' ? <NumberInput label={t('fmodUseInfluence')} value={modifier.influence} min={0} max={1} step={0.05} onCommit={(influence) => set({ influence })} /> : null}
              </Row>
              <Row label={t('fmodUseRange')}>
                <Checkbox label={t('fmodUseRange')} checked={range != null} disabled={readOnly} onChange={(on) => set({ range: on ? { start: frame, end: frame + 10, blendIn: 0, blendOut: 0 } : null })} testId={`scene-fmod-${at}-range`} />
              </Row>
              {range ? (
                <>
                  <Row label={t('fmodRangeStart')}>
                    <NumberInput label={t('fmodRangeStart')} value={range.start} step={1} onCommit={(start) => set({ range: { ...range, start } })} />
                    <NumberInput label={t('fmodRangeEnd')} value={range.end} step={1} onCommit={(end) => set({ range: { ...range, end } })} />
                  </Row>
                  <Row label={t('fmodBlendIn')}>
                    <NumberInput label={t('fmodBlendIn')} value={range.blendIn} min={0} step={1} onCommit={(blendIn) => set({ range: { ...range, blendIn } })} />
                    <NumberInput label={t('fmodBlendOut')} value={range.blendOut} min={0} step={1} onCommit={(blendOut) => set({ range: { ...range, blendOut } })} />
                  </Row>
                </>
              ) : null}
            </div>
          </div>
        )
      })}
    </div>
  )
}
