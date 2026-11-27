import { useEffect, useRef, useState, type KeyboardEvent, type MouseEvent, type PointerEvent } from 'react'
import { readNumber } from './numeric'

/** Digits after the point that `step` has (0.1 → 1, 1 → 0). */
function decimalsOf(step: number) {
  if (!Number.isFinite(step) || step <= 0) return 0
  const text = String(step)
  const dot = text.indexOf('.')
  return dot === -1 ? 0 : text.length - dot - 1
}

function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value))
}

/** Gives the keyboard back to the editor (the viewport's shortcuts), as Blender does after a field. */
export function focusEditor() {
  document.querySelector<HTMLElement>('.kui-app')?.focus({ preventScroll: true })
}

/** What the pointer's field offers the keys and its right-click menu (Blender's field under the pointer). */
export type FieldActions = {
  label: string
  copy: () => void
  /** Null when the field cannot be changed (read-only). */
  paste: (() => void) | null
  /** Null when the field has no default the engine describes. */
  reset: (() => void) | null
  /** Null when the property is not animatable. */
  insertKey: (() => void) | null
}

/** Each field's actions, by its element. */
const fields = new WeakMap<Element, FieldActions>()

/** Where the pointer last was on the page, in client pixels. */
const pointer = { x: 0, y: 0, seen: false }
if (typeof window !== 'undefined') {
  const seen = (event: globalThis.PointerEvent | globalThis.MouseEvent) => {
    pointer.x = event.clientX
    pointer.y = event.clientY
    pointer.seen = true
  }
  window.addEventListener('pointermove', seen, { capture: true, passive: true })
  window.addEventListener('pointerdown', seen, { capture: true, passive: true })
  window.addEventListener('mousemove', seen, { capture: true, passive: true })
}

/** The field under the pointer, if any (found where the pointer is when a key goes down). */
function hoveredField() {
  if (!pointer.seen) return null
  const element = document.elementFromPoint(pointer.x, pointer.y)?.closest('.kui-number-field')
  return element ? (fields.get(element) ?? null) : null
}

/** The value Ctrl/Cmd+C over a field took (also given to the system clipboard when it allows). */
let copied: string | null = null

/** The event a field's right button sends: the editor opens its menu at the pointer. */
export const FIELD_MENU_EVENT = 'scene-field-menu'
export type FieldMenuDetail = { at: { x: number; y: number }; actions: FieldActions }

/**
 * Over a field, Backspace resets it to its default and Ctrl/Cmd+C and Ctrl/Cmd+V copy and paste
 * its value, as in Blender; answers whether `event` was one of those.
 */
export function fieldKey(event: Pick<globalThis.KeyboardEvent, 'code' | 'key' | 'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey' | 'preventDefault' | 'stopPropagation'>) {
  if (event.altKey || event.shiftKey) return false
  const field = hoveredField()
  if (!field) return false
  const command = event.metaKey || event.ctrlKey
  let action: (() => void) | null = null
  if (!command && event.key === 'Backspace') action = field.reset ?? (() => undefined)
  else if (command && event.code === 'KeyC') action = field.copy
  else if (command && event.code === 'KeyV') action = field.paste ?? (() => undefined)
  if (!action) return false
  event.preventDefault()
  event.stopPropagation()
  action()
  return true
}

/** For tests: the value Ctrl/Cmd+C over a field took last. */
declare global {
  interface Window {
    sceneFieldCopied?: () => string | null
  }
}
if (import.meta.env.DEV && typeof window !== 'undefined') window.sceneFieldCopied = () => copied

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
  /** The unit shown after the value: `m` and `mm` take lengths, `°` angles (see `readNumber`). */
  unit?: string
  disabled?: boolean
  testId?: string
  /** The label is the row's own, shown beside the field already: a narrow grip stands in for it. */
  grip?: boolean
  /** Called while dragging with `final: false`, and once with `final: true` on release, Enter, an arrow, or blur. */
  onChange: (value: number, final: boolean) => void
  /** A drag let go of with Escape: the value goes back to where it was. */
  onCancel?: () => void
  /** Puts the field (and the vector it belongs to) back to its default; none when it has none. */
  onReset?: () => void
  /** Keys the property the field shows at the current frame; none when it is not animatable. */
  onInsertKey?: () => void
}

/**
 * A boxed number field in the frame's look (kit-editor's `kui-number-field`) whose step governs
 * only dragging and the arrow keys: a typed value keeps its digits up to `precision` (kit-editor's
 * field rounds what is typed to the step), and may carry units and arithmetic (`50cm`, `1+1`,
 * `90deg`); an entry that does not read shakes the field and puts the value back, sending
 * nothing. Dragging the label scrubs (Shift finely, Ctrl/Cmd in whole steps of ten), reporting each
 * value with `final: false`; Escape during a drag puts the value back. A click on the label, or a
 * double click, types into the field. Enter and Escape while typing give the keyboard back to the
 * editor. The wheel does nothing over it. Over the field, Backspace resets it and Ctrl/Cmd+C and V
 * copy and paste (`fieldKey`); the right button opens Copy, Paste, Reset to Default and Insert
 * Keyframe.
 */
export function NumberBox({ label, value, min, max, step, precision, unit, disabled, testId, grip, onChange, onCancel, onReset, onInsertKey }: NumberBoxProps) {
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
  const [invalid, setInvalid] = useState(0)
  const editing = useRef(false)
  const input = useRef<HTMLInputElement>(null)
  const drag = useRef<{ pointerId: number; startX: number; startValue: number; last: number; sent: boolean; element: HTMLElement; escape: (event: globalThis.KeyboardEvent) => void } | null>(null)
  const latest = useRef({ value, onChange, onCancel, onReset, onInsertKey, disabled, format, resolve })
  latest.current = { value, onChange, onCancel, onReset, onInsertKey, disabled, format, resolve }

  useEffect(() => {
    if (editing.current || drag.current) return
    setText(format(value))
    // `format` follows `digits` and `precision`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value, digits, precision])

  // The shake ends by itself.
  useEffect(() => {
    if (!invalid) return
    const timer = window.setTimeout(() => setInvalid(0), 450)
    return () => window.clearTimeout(timer)
  }, [invalid])

  /** Sets the value from text typed or pasted; answers whether it read. */
  const apply = (raw: string) => {
    const current = latest.current
    const parsed = readNumber(raw, unit)
    if (parsed === null) {
      setText(current.format(current.value))
      setInvalid(Date.now())
      return false
    }
    const next = current.resolve(parsed)
    setText(current.format(next))
    if (next !== current.value) current.onChange(next, true)
    return true
  }

  const actions = useRef<FieldActions | null>(null)
  actions.current = {
    label,
    copy: () => {
      const shown = latest.current.format(latest.current.value)
      copied = shown
      void navigator.clipboard?.writeText(shown).catch(() => undefined)
    },
    paste: disabled ? null : () => {
      if (copied != null) apply(copied)
    },
    reset: disabled || !onReset ? null : () => latest.current.onReset?.(),
    insertKey: disabled || !onInsertKey ? null : () => latest.current.onInsertKey?.(),
  }
  const self = useRef<FieldActions>({
    get label() {
      return actions.current?.label ?? ''
    },
    copy: () => actions.current?.copy(),
    get paste() {
      return actions.current?.paste ?? null
    },
    get reset() {
      return actions.current?.reset ?? null
    },
    get insertKey() {
      return actions.current?.insertKey ?? null
    },
  } as FieldActions)

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
    // A press on the label let go of without moving types into the field.
    else if (!disabled) {
      input.current?.focus()
      input.current?.select()
    }
  }

  const scrubbed = (event: PointerEvent<HTMLElement>, current: NonNullable<typeof drag.current>) => {
    const steps = (event.clientX - current.startX) / 2
    const raw = current.startValue + steps * step * (event.shiftKey ? 0.1 : 1)
    // Ctrl or Cmd: whole steps of ten times the field's step (1 m, 10°, 1.0).
    if (event.ctrlKey || event.metaKey) {
      const coarse = step * 10
      return resolve(Math.round(raw / coarse) * coarse)
    }
    return resolve(raw)
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
    // An entry left as it is shows the value rounded for display: it changes nothing.
    if (raw === format(value)) return
    apply(raw)
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

  const onContextMenu = (event: MouseEvent<HTMLElement>) => {
    event.preventDefault()
    event.stopPropagation()
    const detail: FieldMenuDetail = { at: { x: event.clientX, y: event.clientY }, actions: self.current }
    window.dispatchEvent(new CustomEvent(FIELD_MENU_EVENT, { detail }))
  }

  return (
    <div
      className="kui-number-field"
      data-testid={testId}
      data-invalid={invalid ? 'true' : undefined}
      ref={(element) => {
        if (element) fields.set(element, self.current)
      }}
      onContextMenu={onContextMenu}
    >
      <span
        className={grip ? 'kui-number-field-label scene-number-grip' : 'kui-number-field-label'}
        data-testid={testId ? `${testId}-label` : undefined}
        aria-hidden="true"
        title={grip ? label : undefined}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={() => endDrag(true)}
        onDoubleClick={() => {
          if (disabled) return
          input.current?.focus()
          input.current?.select()
        }}
      >
        {grip ? null : label}
      </span>
      <input
        ref={input}
        className="kui-field kui-number-field-input"
        type="text"
        inputMode="decimal"
        role="spinbutton"
        aria-label={label}
        aria-valuemin={min}
        aria-valuemax={max}
        aria-valuenow={value}
        aria-valuetext={unit ? `${format(value)} ${unit}` : format(value)}
        aria-invalid={invalid ? true : undefined}
        value={text}
        disabled={disabled}
        onChange={(event) => {
          if (disabled) return
          editing.current = true
          setText(event.target.value)
        }}
        onDoubleClick={(event) => event.currentTarget.select()}
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
