/**
 * What a number field reads from what is typed into it (plan scene-usable-by-hand, phase 8):
 * numbers, `+ - * /`, brackets and unit suffixes, worked out by a small parser of its own (no
 * `eval`). A length field (metres, or millimetres for a lens) takes `mm`, `cm`, `m`, `km`, `in`
 * and `ft`; an angle field (degrees) takes `deg`, `°` and `rad`. A bare number is in the field's
 * own unit, unless the entry names a unit: then the first unit named is what bare numbers are in
 * (`2 + 2mm` is 4 mm, as Blender reads it). Units written one after the other add up
 * (`1m 50cm`, and `-1m 50cm` is minus the whole). Anything else does not read: the answer is null.
 */

type Kind = 'length' | 'angle'

/** Each unit's size: lengths in metres, angles in degrees. */
const UNITS: Record<string, { kind: Kind; scale: number }> = {
  mm: { kind: 'length', scale: 0.001 },
  cm: { kind: 'length', scale: 0.01 },
  m: { kind: 'length', scale: 1 },
  km: { kind: 'length', scale: 1000 },
  in: { kind: 'length', scale: 0.0254 },
  ft: { kind: 'length', scale: 0.3048 },
  deg: { kind: 'angle', scale: 1 },
  '°': { kind: 'angle', scale: 1 },
  rad: { kind: 'angle', scale: 180 / Math.PI },
}

/** The kind and size of a field's unit as the field shows it (`m`, `mm`, `°`); none for others. */
function fieldUnit(unit: string | undefined) {
  const found = unit ? UNITS[unit] : undefined
  return found ?? null
}

type Token = { type: 'number'; value: number } | { type: 'unit'; name: string } | { type: 'op'; op: string }

function tokens(text: string): Token[] | null {
  const out: Token[] = []
  let at = 0
  while (at < text.length) {
    const rest = text.slice(at)
    const space = /^\s+/.exec(rest)
    if (space) {
      at += space[0].length
      continue
    }
    const number = /^(?:\d+(?:[.,]\d*)?|[.,]\d+)(?:[eE][+-]?\d+)?/.exec(rest)
    if (number) {
      const value = Number(number[0].replace(',', '.'))
      if (!Number.isFinite(value)) return null
      out.push({ type: 'number', value })
      at += number[0].length
      continue
    }
    const word = /^(?:[a-zA-Z]+|°)/.exec(rest)
    if (word) {
      out.push({ type: 'unit', name: word[0].toLowerCase() })
      at += word[0].length
      continue
    }
    if ('+-*/()'.includes(rest[0])) {
      out.push({ type: 'op', op: rest[0] })
      at += 1
      continue
    }
    return null
  }
  return out
}

class Parser {
  private at = 0
  constructor(
    private readonly list: Token[],
    /** What a unit's number is multiplied by: its size over the reference unit's. */
    private readonly factor: (name: string) => number | null,
  ) {}

  private peek() {
    return this.list[this.at]
  }

  private isOp(op: string) {
    const token = this.peek()
    return token?.type === 'op' && token.op === op
  }

  done() {
    return this.at === this.list.length
  }

  sum(): number | null {
    let value = this.product()
    while (value != null && (this.isOp('+') || this.isOp('-'))) {
      const minus = this.isOp('-')
      this.at += 1
      const next = this.product()
      if (next == null) return null
      value = minus ? value - next : value + next
    }
    return value
  }

  private product(): number | null {
    let value = this.signed()
    while (value != null && (this.isOp('*') || this.isOp('/'))) {
      const divide = this.isOp('/')
      this.at += 1
      const next = this.signed()
      if (next == null) return null
      if (divide && next === 0) return null
      value = divide ? value / next : value * next
    }
    return value
  }

  private signed(): number | null {
    if (this.isOp('-') || this.isOp('+')) {
      const minus = this.isOp('-')
      this.at += 1
      const value = this.signed()
      return value == null ? null : minus ? -value : value
    }
    return this.chain()
  }

  /** A quantity, and the quantities with units written right after it (`1m 50cm`), added up. */
  private chain(): number | null {
    const first = this.quantity()
    if (!first) return null
    let value = first.value
    let united = first.united
    while (united && this.peek()?.type === 'number') {
      const next = this.quantity()
      if (!next) return null
      value += next.value
      united = next.united
    }
    return value
  }

  /** A number or a bracket, with the unit after it if there is one. */
  private quantity(): { value: number; united: boolean } | null {
    const token = this.peek()
    let value: number | null
    if (token?.type === 'number') {
      this.at += 1
      value = token.value
    } else if (this.isOp('(')) {
      this.at += 1
      value = this.sum()
      if (value == null || !this.isOp(')')) return null
      this.at += 1
    } else return null
    const unit = this.peek()
    if (unit?.type !== 'unit') return { value, united: false }
    const factor = this.factor(unit.name)
    if (factor == null) return null
    this.at += 1
    return { value: value * factor, united: true }
  }
}

/**
 * The value `text` gives a field whose unit shows as `unit` (`m`, `mm`, `°`, or another suffix
 * such as `px`, which is only allowed after the number), in that unit; null when it does not read.
 */
export function readNumber(text: string, unit?: string): number | null {
  const trimmed = text.trim()
  if (!trimmed) return null
  const own = fieldUnit(unit)
  let source = trimmed
  // A field without a kind of unit takes its own suffix after the number, and no other unit.
  if (!own && unit && source.toLowerCase().endsWith(unit.toLowerCase())) source = source.slice(0, -unit.length)
  const list = tokens(source)
  if (!list || list.length === 0) return null
  const named = list.filter((token): token is { type: 'unit'; name: string } => token.type === 'unit').map((token) => UNITS[token.name])
  if (named.some((found) => !found || !own || found.kind !== own.kind)) return null
  const reference = named[0]?.scale ?? own?.scale ?? 1
  const parser = new Parser(list, (name) => {
    const found = UNITS[name]
    return found && own && found.kind === own.kind ? found.scale / reference : null
  })
  const value = parser.sum()
  if (value == null || !parser.done() || !Number.isFinite(value)) return null
  const result = (value * reference) / (own?.scale ?? 1)
  return Number.isFinite(result) ? result : null
}
