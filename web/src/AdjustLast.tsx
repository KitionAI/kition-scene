import { useEffect, useState } from 'react'
import { Checkbox, Choice, useKitEditor } from '@kition/sdk/editor'
import type { LastOperation, OperationField } from './scene'
import { NumberBox } from './NumberBox'

/** The event F9 sends: the panel opens or closes its fields. */
export const ADJUST_LAST_EVENT = 'scene-adjust-last'

/** F9: Adjust Last Operation's fields open (or close). */
export function toggleAdjustLast() {
  window.dispatchEvent(new CustomEvent(ADJUST_LAST_EVENT))
}

const AXES = ['X', 'Y', 'Z'] as const

/**
 * Adjust Last Operation (Blender's panel in the viewport's corner): the person's last operation,
 * its title a button that shows or hides its fields, the fields its command's parameters (as the
 * engine describes them). Changing a field runs the operation again with the new value in place of
 * its undo step (`operation.set`); anything else that happens ends it, and the panel goes.
 */
export function AdjustLast({ operation, hidden, t }: { operation: LastOperation | null; hidden: boolean; t: (key: string) => string }) {
  const editor = useKitEditor()
  const { phrase: p } = editor
  const [open, setOpen] = useState(false)
  useEffect(() => {
    const toggle = () => setOpen((value) => !value)
    window.addEventListener(ADJUST_LAST_EVENT, toggle)
    return () => window.removeEventListener(ADJUST_LAST_EVENT, toggle)
  }, [])
  if (!operation || hidden) return null
  const set = (key: string, value: unknown) => void editor.run('operation.set', { params: { [key]: value } })
  const field = (item: OperationField) => {
    const label = p(item.label)
    const testId = `scene-adjust-${item.key}`
    switch (item.type) {
      case 'boolean':
        return <Checkbox key={item.key} label={label} checked={item.value === true} onChange={(checked) => set(item.key, checked)} testId={testId} />
      case 'choice': {
        const choices = (item.choices ?? []).map((choice) => [choice, p(choice)] as const)
        const value = typeof item.value === 'string' ? item.value : ''
        return (
          <Choice
            key={item.key}
            label={label}
            value={value}
            choices={value === '' ? [['', '—'] as const, ...choices] : choices}
            onChange={(next) => {
              if (next !== '') set(item.key, next)
            }}
            testId={testId}
          />
        )
      }
      case 'vector': {
        const vector = Array.isArray(item.value) ? item.value.map(Number) : [0, 0, 0]
        return (
          <div key={item.key} className="scene-adjust-vector" data-testid={testId}>
            <span className="scene-adjust-vector-label">{label}</span>
            {AXES.map((axis, index) => (
              <NumberBox
                key={axis}
                label={axis}
                value={vector[index] ?? 0}
                min={-1e6}
                max={1e6}
                step={0.1}
                precision={4}
                testId={`${testId}-${axis.toLowerCase()}`}
                onChange={(value, final) => {
                  if (!final) return
                  const next = [...vector]
                  next[index] = value
                  set(item.key, next)
                }}
              />
            ))}
          </div>
        )
      }
      default: {
        const whole = item.type === 'integer'
        return (
          <NumberBox
            key={item.key}
            label={label}
            value={typeof item.value === 'number' ? item.value : 0}
            min={-1e6}
            max={1e6}
            step={whole ? 1 : 0.01}
            precision={whole ? 0 : 4}
            testId={testId}
            onChange={(value, final) => {
              if (final) set(item.key, value)
            }}
          />
        )
      }
    }
  }
  return (
    <div className="scene-adjust-last" data-testid="scene-adjust-last" data-open={open} data-command={operation.command ?? ''} role="group" aria-label={t('adjustLast')}>
      <button type="button" className="scene-adjust-last-title" aria-expanded={open} title={t('adjustLastToggle')} onClick={() => setOpen((value) => !value)} data-testid="scene-adjust-last-title">
        <span aria-hidden="true">{open ? '▾' : '▸'}</span> {p(operation.title)}
      </button>
      {open ? <div className="scene-adjust-last-fields">{operation.fields.map(field)}</div> : null}
    </div>
  )
}
