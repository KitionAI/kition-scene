import { useEffect, useRef, useState, type KeyboardEvent, type PointerEvent } from 'react'

/** Digits after the point that `step` has (0.1 → 1, 1 → 0). */
function decimalsOf(step: number) {
  if (!Number.isFinite(step) || step <= 0) return 0
  const text = String(step)
  const dot = text.indexOf('.')
  return dot === -1 ? 0 : text.length - dot - 1
}

function parse(text: string) {
  const normalized = text.trim().replace(',', '.')
  if (normalized === '') return null
  const value = Number(normalized)
  return Number.isFinite(value) ? value : null
}

function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value))
}

/** Gives the keyboard back to the editor (the viewport's shortcuts), as Blender does after a field. */
export function focusEditor() {
  document.querySelector<HTMLElement>('.kui-app')?.focus({ preventScroll: true })
}

export type NumberBoxProps = {
  label: string
  value: number
  min: number
  max: number
  /** What dragging the label and the arrow keys move the value by. */
  step: number
  /**
   * Digits after the point a value keeps, typed or dragged (`step`'s by default). With it given,
   * the field shows no trailing zeros (0.05, not 0.050).
   */
  precision?: number
  unit?: string
  disabled?: boolean
  testId?: string
  /** The label is the row's own, shown beside the field already: a narrow grip stands in for it. */
  grip?: boolean
  /** Called while dragging with `final: false`, and once with `final: true` on release, Enter, an arrow, or blur. */
  onChange: (value: number, final: boolean) => void
  /** A drag let go of with Escape: the value goes back to where it was. */
  onCancel?: () => void
}

/**
 * A boxed number field in the frame's look (kit-editor's `kui-number-field`) whose step governs
 * only dragging and the arrow keys: a typed value keeps its digits up to `precision` (kit-editor's
 * field rounds what is typed to the step). Dragging the label scrubs (Shift ten times, Alt a tenth),
 * reporting each value with `final: false`; Escape during a drag puts the value back. Enter and
 * Escape while typing give the keyboard back to the editor.
 */
export function NumberBox({ label, value, min, max, step, precision, unit, disabled, testId, grip, onChange, onCancel }: NumberBoxProps) {
  const digits = precision ?? decimalsOf(step)
  const format = (number: number) => {
    const fixed = number.toFixed(digits)
    return precision == null || !fixed.includes('.') ? fixed : fixed.replace(/\.?0+$/, '')
  }
  const resolve = (raw: number) => {
    const factor = 10 ** digits
    return clamp(Math.round(raw * factor) / factor, min, max)
  }
  const [text, setText] = useState(() => format(value))
  const editing = useRef(false)
  const drag = useRef<{ pointerId: number; startX: number; startValue: number; last: number; sent: boolean; element: HTMLElement; escape: (event: globalThis.KeyboardEvent) => void } | null>(null)
  const latest = useRef({ value, onChange, onCancel })
  latest.current = { value, onChange, onCancel }

  useEffect(() => {
    if (editing.current || drag.current) return
    setText(format(value))
    // `format` follows `digits` and `precision`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value, digits, precision])

  const endDrag = (cancelled: boolean) => {
    const current = drag.current
    if (!current) return
    drag.current = null
    window.removeEventListener('keydown', current.escape, true)
    if (current.element.hasPointerCapture?.(current.pointerId)) current.element.releasePointerCapture(current.pointerId)
    const { value: shown, onChange: change, onCancel: cancel } = latest.current
    if (cancelled) {
      setText(format(shown))
      if (current.sent) cancel?.()
      return
    }
    setText(format(current.last))
    if (current.sent) change(current.last, true)
  }

  const scrubbed = (event: PointerEvent<HTMLElement>, current: NonNullable<typeof drag.current>) => {
    const steps = (event.clientX - current.startX) / 2
    const multiplier = event.shiftKey ? 10 : event.altKey ? 0.1 : 1
    return resolve(current.startValue + steps * step * multiplier)
  }

  const onPointerDown = (event: PointerEvent<HTMLElement>) => {
    if (disabled || event.button !== 0) return
    event.preventDefault()
    event.currentTarget.setPointerCapture?.(event.pointerId)
    const escape = (key: globalThis.KeyboardEvent) => {
      if (key.key !== 'Escape') return
      key.preventDefault()
      key.stopPropagation()
      endDrag(true)
    }
    window.addEventListener('keydown', escape, true)
    drag.current = { pointerId: event.pointerId, startX: event.clientX, startValue: value, last: value, sent: false, element: event.currentTarget, escape }
  }
  const onPointerMove = (event: PointerEvent<HTMLElement>) => {
    const current = drag.current
    if (disabled || !current || current.pointerId !== event.pointerId) return
    const next = scrubbed(event, current)
    if (next === current.last) return
    current.last = next
    current.sent = true
    setText(format(next))
    onChange(next, false)
  }
  const onPointerUp = (event: PointerEvent<HTMLElement>) => {
    const current = drag.current
    if (!current || current.pointerId !== event.pointerId) return
    endDrag(false)
  }

  const commit = (raw: string) => {
    const parsed = parse(unit && raw.trim().endsWith(unit) ? raw.trim().slice(0, -unit.length) : raw)
    if (parsed === null) {
      setText(format(value))
      return
    }
    const next = resolve(parsed)
    setText(format(next))
    if (next !== value) onChange(next, true)
  }
  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (disabled || event.nativeEvent.isComposing) return
    if (event.key === 'Enter') {
      event.preventDefault()
      editing.current = false
      commit(text)
      focusEditor()
    } else if (event.key === 'Escape') {
      // An unchanged field lets Escape reach the frame too (it leaves Focus).
      if (text !== format(value)) event.preventDefault()
      editing.current = false
      setText(format(value))
      focusEditor()
    } else if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
      event.preventDefault()
      const next = resolve(value + (event.key === 'ArrowUp' ? 1 : -1) * step * (event.shiftKey ? 10 : 1))
      editing.current = false
      setText(format(next))
      if (next !== value) onChange(next, true)
    }
  }

  return (
    <div className="kui-number-field" data-testid={testId}>
      <span
        className={grip ? 'kui-number-field-label scene-number-grip' : 'kui-number-field-label'}
        data-testid={testId ? `${testId}-label` : undefined}
        aria-hidden="true"
        title={grip ? label : undefined}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={() => endDrag(true)}
      >
        {grip ? null : label}
      </span>
      <input
        className="kui-field kui-number-field-input"
        type="text"
        inputMode="decimal"
        role="spinbutton"
        aria-label={label}
        aria-valuemin={min}
        aria-valuemax={max}
        aria-valuenow={value}
        aria-valuetext={unit ? `${format(value)} ${unit}` : format(value)}
        value={text}
        disabled={disabled}
        onChange={(event) => {
          if (disabled) return
          editing.current = true
          setText(event.target.value)
        }}
        onKeyDown={onKeyDown}
        onBlur={() => {
          if (disabled || !editing.current) return
          editing.current = false
          commit(text)
        }}
      />
      {unit ? <span className="kui-number-field-unit">{unit}</span> : null}
    </div>
  )
}
