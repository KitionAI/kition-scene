import { Button, Checkbox, Choice, Segmented, useKitEditor, useReadOnly } from '@kition/kit-editor'
import { NumberInput, useRun } from './SidePanel'
import type { SceneInfo } from './scene'

const FALLOFFS = ['smooth', 'sphere', 'root', 'linear', 'sharp', 'constant'] as const
const TOOLS = ['brush', 'linearGradient', 'radialGradient', 'sampleWeight'] as const

/** Weight paint on or off for the active mesh (object mode and weight paint mode). */
export function WeightPaintToggle({ info }: { info: SceneInfo }) {
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const active = info.objects.find((object) => object.id === info.active)
  if (info.mode !== 'weightPaint' && (info.mode !== 'object' || active?.type !== 'mesh')) return null
  return <Checkbox label={t('weightPaint')} checked={info.mode === 'weightPaint'} disabled={readOnly} onChange={(on) => run('mode.weightPaint', { on })} testId="scene-weight-paint" />
}

/** The weight paint tool header over the viewport: the tool (brush, gradients, Sample Weight),
 * the group painted, the brush, and X mirror. */
export function WeightPaintControls({ info }: { info: SceneInfo }) {
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const paint = info.paint
  if (info.mode !== 'weightPaint' || !paint) return null
  const object = info.objects.find((item) => item.id === info.editing)
  const groups = object?.vertexGroups.map((group) => group.name) ?? []
  const group = paint.group ?? groups[0] ?? ''
  const brush = (values: Record<string, unknown>) => run('paint.brush', values)
  return (
    <div className="scene-weight-paint" role="toolbar" aria-label={t('weightPaint')} data-testid="scene-weight-toolbar">
      {groups.length ? (
        <Choice label={t('paintGroup')} quiet value={group} disabled={readOnly} choices={groups.map((name) => [name, name] as const)} onChange={(name) => run('paint.group', { name })} testId="scene-paint-group" />
      ) : null}
      <Segmented
        label={t('weightTool')}
        value={paint.tool}
        disabled={readOnly}
        segments={TOOLS.map((tool) => ({ value: tool, label: t(`tool_${tool}`) }))}
        onChange={(tool) => run('paint.tool', { tool })}
        testId="scene-weight-tool"
      />
      <Segmented
        label={t('brushMode')}
        value={paint.brush.mode}
        disabled={readOnly}
        segments={(['draw', 'add', 'subtract', 'blur'] as const).map((mode) => ({ value: mode, label: t(`brush_${mode}`) }))}
        onChange={(mode) => brush({ mode })}
        testId="scene-brush-mode"
      />
      <span className="scene-weight-field">
        <NumberInput label={t('weight')} value={paint.brush.weight} min={0} max={1} step={0.05} track onCommit={(weight) => brush({ weight })} testId="scene-brush-weight" />
      </span>
      <span className="scene-weight-field">
        <NumberInput label={t('brushRadius')} value={paint.brush.radius} min={1} max={2000} step={5} unit="px" onCommit={(radius) => brush({ radius })} testId="scene-brush-radius" />
      </span>
      <span className="scene-weight-field">
        <NumberInput label={t('strength')} value={paint.brush.strength} min={0} max={1} step={0.05} track onCommit={(strength) => brush({ strength })} testId="scene-brush-strength" />
      </span>
      <Choice label={t('falloff')} quiet value={paint.brush.falloff} disabled={readOnly} choices={FALLOFFS.map((falloff) => [falloff, t(`falloff_${falloff}`)] as const)} onChange={(falloff) => brush({ falloff })} />
      <Checkbox label={t('frontFacesOnly')} checked={paint.brush.frontFacesOnly} disabled={readOnly} onChange={(frontFacesOnly) => brush({ frontFacesOnly })} />
      <Checkbox label={t('xMirror')} checked={paint.mirror.x} disabled={readOnly} onChange={(xMirror) => brush({ xMirror })} testId="scene-weight-x-mirror" />
      {paint.mirror.x ? (
        <>
          <Checkbox label={t('topologyMirror')} checked={paint.mirror.topology} disabled={readOnly} onChange={(topologyMirror) => brush({ topologyMirror })} testId="scene-weight-topology-mirror" />
          <Checkbox label={t('mirrorGroups')} checked={paint.mirror.groups} disabled={readOnly} onChange={(mirrorGroups) => brush({ mirrorGroups })} />
        </>
      ) : null}
      <Button size="sm" disabled={readOnly || groups.length === 0} onClick={() => run('paint.normalizeAll', {})} data-testid="scene-normalize-all">
        {t('normalizeAll')}
      </Button>
    </div>
  )
}
