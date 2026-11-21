import { Button, Checkbox, Choice, useKitEditor, useReadOnly } from '@kition/sdk/editor'
import { useState } from 'react'
import { NumberInput, useRun } from './SidePanel'
import type { SceneInfo } from './scene'

const BRUSHES = ['draw', 'clayStrips', 'smooth', 'grab', 'inflate', 'crease', 'flatten', 'pinch', 'snakeHook', 'mask', 'drawFaceSets'] as const
const TOOLS = ['brush', 'boxMask', 'lassoMask', 'boxHide', 'lassoHide', 'boxFaceSet', 'lassoFaceSet'] as const
const FALLOFFS = ['smooth', 'sphere', 'root', 'linear', 'sharp', 'constant'] as const
const AXES = ['X', 'Y', 'Z'] as const

/** Sculpt mode on or off for the active mesh (object mode and sculpt mode). */
export function SculptToggle({ info }: { info: SceneInfo }) {
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const active = info.objects.find((object) => object.id === info.active)
  if (info.mode !== 'sculpt' && (info.mode !== 'object' || active?.type !== 'mesh')) return null
  return <Checkbox label={t('sculptMode')} checked={info.mode === 'sculpt'} disabled={readOnly} onChange={(on) => run('mode.sculpt', { on })} testId="scene-sculpt" />
}

/** The sculpt tool header over the viewport: the tool, the brush and its settings, the mask,
 * and remesh (dynamic topology and face sets are in the side panel). */
export function SculptControls({ info }: { info: SceneInfo }) {
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const [voxel, setVoxel] = useState(0.1)
  const sculpt = info.sculpt
  if (info.mode !== 'sculpt' || !sculpt) return null
  const brush = sculpt.brush
  const set = (values: Record<string, unknown>) => run('sculpt.brush', values)
  return (
    <div className="scene-weight-paint scene-sculpt" role="toolbar" aria-label={t('sculptMode')} data-testid="scene-sculpt-toolbar">
      <Choice label={t('sculptTool')} quiet value={sculpt.tool} disabled={readOnly} choices={TOOLS.map((tool) => [tool, t(`sculptTool_${tool}`)] as const)} onChange={(tool) => run('sculpt.tool', { tool })} testId="scene-sculpt-tool" />
      {sculpt.levels > 0 ? (
        <span className="scene-sculpt-label" data-testid="scene-sculpt-level">
          {t('sculptLevel', { level: sculpt.level, levels: sculpt.levels })}
        </span>
      ) : null}
      <Choice label={t('sculptBrush')} quiet value={brush.kind} disabled={readOnly} choices={BRUSHES.map((kind) => [kind, t(`sculpt_${kind}`)] as const)} onChange={(kind) => set({ kind })} testId="scene-sculpt-brush" />
      <span className="scene-weight-field">
        <NumberInput label={t('brushRadius')} value={brush.radius} min={1} max={2000} step={5} unit="px" onCommit={(radius) => set({ radius })} testId="scene-sculpt-radius" />
      </span>
      <span className="scene-weight-field">
        <NumberInput label={t('strength')} value={brush.strength} min={0} max={1} step={0.05} track onCommit={(strength) => set({ strength })} testId="scene-sculpt-strength" />
      </span>
      <Choice label={t('falloff')} quiet value={brush.falloff} disabled={readOnly} choices={FALLOFFS.map((falloff) => [falloff, t(`falloff_${falloff}`)] as const)} onChange={(falloff) => set({ falloff })} />
      <Checkbox label={t('sculptSubtract')} checked={brush.subtract} disabled={readOnly} onChange={(subtract) => set({ subtract })} testId="scene-sculpt-subtract" />
      <span className="scene-sculpt-axes" aria-label={t('symmetry')}>
        <span className="scene-sculpt-label">{t('symmetry')}</span>
        {AXES.map((axis, slot) => (
          <Checkbox key={axis} label={axis} checked={brush.symmetry[slot] ?? false} disabled={readOnly} onChange={(on) => set({ symmetry: brush.symmetry.map((value, other) => (other === slot ? on : value)) })} />
        ))}
      </span>
      <Checkbox label={t('frontFacesOnly')} checked={brush.frontFacesOnly} disabled={readOnly} onChange={(frontFacesOnly) => set({ frontFacesOnly })} />
      <span className="scene-weight-field">
        <NumberInput label={t('autoSmooth')} value={brush.autoSmooth} min={0} max={1} step={0.05} track onCommit={(autoSmooth) => set({ autoSmooth })} />
      </span>
      <Button size="sm" disabled={readOnly} onClick={() => run('sculpt.maskInvert', {})} data-testid="scene-sculpt-mask-invert">
        {t('maskInvert')}
      </Button>
      <Button size="sm" disabled={readOnly || sculpt.masked === 0} onClick={() => run('sculpt.maskClear', {})} data-testid="scene-sculpt-mask-clear">
        {t('maskClear')}
      </Button>
      <span className="scene-weight-field">
        <NumberInput label={t('voxelSize')} value={voxel} min={0.001} max={10} step={0.01} unit="m" onCommit={setVoxel} testId="scene-sculpt-voxel" />
      </span>
      <Button size="sm" disabled={readOnly} onClick={() => run('sculpt.remesh', { voxelSize: voxel })} data-testid="scene-sculpt-remesh">
        {t('remesh')}
      </Button>
    </div>
  )
}
