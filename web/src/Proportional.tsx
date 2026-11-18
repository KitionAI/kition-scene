import { Checkbox, Choice, useKitEditor } from '@kition/kit-editor'
import type { SceneInfo } from './scene'

const FALLOFFS = ['smooth', 'sphere', 'root', 'linear', 'sharp', 'constant'] as const

/** Proportional editing in the status bar: on/off (O) and the falloff curve. */
export function ProportionalControl({ info }: { info: SceneInfo }) {
  const editor = useKitEditor()
  const { t } = editor
  const on = info.proportional != null
  return (
    <span className="scene-proportional">
      <Checkbox label={t('proportional')} checked={on} onChange={(value) => void editor.run('mesh.proportional', { on: value })} testId="scene-proportional" />
      {on ? (
        <Choice
          label={t('falloff')}
          quiet
          value={info.proportional?.falloff ?? 'smooth'}
          choices={FALLOFFS.map((falloff) => [falloff, t(`falloff_${falloff}`)] as const)}
          onChange={(falloff) => void editor.run('mesh.proportional', { on: true, falloff })}
          testId="scene-proportional-falloff"
        />
      ) : null}
      {on ? (
        <Checkbox
          label={t('connectedOnly')}
          checked={info.proportional?.connected === true}
          onChange={(connected) => void editor.run('mesh.proportional', { on: true, connected })}
          testId="scene-proportional-connected"
        />
      ) : null}
    </span>
  )
}
