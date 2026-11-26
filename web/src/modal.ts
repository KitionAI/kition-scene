/**
 * The page's side of the modal transform (G, R, S; Shift+D; the transform tools' drags): the
 * engine's state machine (`bindings/wasm/src/modal.rs`) does the work; the page hands it what the
 * SDK keeps from the tools (letters, digits, the right button, the wheel) and the pointer, and
 * draws what it answers (the header, the axis lines, the dashed line, the status bar's keys).
 *
 * Begin, confirm, cancel and the wheel are session commands, run in order with the tools' events.
 * The pointer and the keys go the quick way (`modalStep`): no inspect, no software picture, the GPU
 * viewport's frame in the same answer. Everything runs one after the other; pointer moves waiting
 * behind a call are taken together, the last one counting.
 */
import type { GlFrame } from './gl/protocol'

export type Vec3 = [number, number, number]

export type ModalKind = 'move' | 'rotate' | 'scale'

/** Edit mode's operations the modal runs: E, I, Ctrl+B and Ctrl+R. */
export type ModalOperation = 'extrude' | 'inset' | 'bevel' | 'loopCut'

/** What a modal under way does (a loop cut's slide after its click is `slide`). */
export type ModalAnswerKind = ModalKind | 'inset' | 'bevel' | 'loopCut' | 'slide'

/** What the engine answers about the transform under way. */
export type ModalAnswer = {
  active: boolean
  kind?: ModalAnswerKind
  started?: 'key' | 'press'
  header?: string
  changed?: boolean
  /** The value: a move's offset, a rotation's degrees, a scale's factors, or an operation's amounts by name. */
  values?: unknown
  /** The wheel is the modal's (a bevel's segments, a loop cut's cuts). */
  wheel?: boolean
  /** An extrusion's move. */
  extrude?: boolean
  /** A loop cut's loops before its click: world segments [ax, ay, az, bx, by, bz]. */
  loops?: number[][]
  /** The same loops on the engine's picture, in document pixels [ax, ay, bx, by]. */
  loopScreen?: number[][]
  constraint?: { axis: 'x' | 'y' | 'z'; plane: boolean; orientation: 'global' | 'local' | 'normal' } | null
  numeric?: { index: number; texts: string[] } | null
  precision?: boolean
  step?: boolean
  pivot?: Vec3
  pivotScreen?: [number, number]
  pointer?: [number, number]
  /** The axes it keeps to: world direction, and the line across the picture in document pixels. */
  lines?: Array<{ axis: number; direction: Vec3; screen: [number, number, number, number] | null }>
  dashed?: boolean
  /** Proportional editing's radius in world units, when vertices follow. */
  proportional?: number | null
  /** The same radius on the screen at the pivot, in document pixels, as the engine's view shows it. */
  proportionalPixels?: number | null
  duplicate?: boolean
  taken?: boolean
  frame?: GlFrame
}

type Hooks = {
  /** A session command, in order with the tools' events; answers its result (JSON). */
  run: (command: string, params: Record<string, unknown>, refresh: 'all' | 'repaint' | 'none') => Promise<string | null>
  /** The engine's own method (`modalStep`), straight to the worker. */
  invoke: (name: string, ...args: unknown[]) => Promise<unknown>
  /** The GPU viewport draws now (otherwise the software picture shows the transform). */
  gpu: () => boolean
  /** The GPU viewport takes the frame the answer brought. */
  adopt: (frame: GlFrame) => void
}

const INACTIVE: ModalAnswer = { active: false }

function parse(text: unknown): ModalAnswer {
  if (typeof text !== 'string') return INACTIVE
  try {
    const value = JSON.parse(text) as ModalAnswer
    return value && typeof value === 'object' ? value : INACTIVE
  } catch {
    return INACTIVE
  }
}

/** For tests: the last answer, and how many answers came. */
type ModalProbe = { answer: ModalAnswer; answers: number }

declare global {
  interface Window {
    sceneModal?: ModalProbe
  }
}

export class ModalTransform {
  answer: ModalAnswer = INACTIVE
  /** Where the pointer last was over the viewport, in document pixels. */
  pointer: [number, number] | null = null
  hooks: Hooks | null = null
  private starting = false
  private listeners = new Set<() => void>()
  private chain: Promise<unknown> = Promise.resolve()
  private pending: { at: [number, number]; precision: boolean; step: boolean } | null = null
  private probe: ModalProbe = { answer: INACTIVE, answers: 0 }

  constructor() {
    if (import.meta.env.DEV && typeof window !== 'undefined') window.sceneModal = this.probe
  }

  /** A transform is under way, or one is starting. */
  get active() {
    return this.answer.active || this.starting
  }

  /** One of edit mode's operations a key started (or an extrusion's move): the left button confirms it wherever it is. */
  get operates() {
    const answer = this.answer
    return answer.active && answer.started === 'key' && (answer.extrude === true || ['inset', 'bevel', 'loopCut', 'slide'].includes(answer.kind ?? ''))
  }

  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  snapshot = () => this.answer

  private emit() {
    for (const listener of [...this.listeners]) listener()
  }

  private take(answer: ModalAnswer) {
    const { frame, ...rest } = answer
    if (frame && rest.active) this.hooks?.adopt(frame)
    this.answer = rest.active ? rest : INACTIVE
    this.probe.answer = this.answer
    this.probe.answers += 1
    this.emit()
  }

  private enqueue(work: () => Promise<void>) {
    const next = this.chain.then(work, work)
    this.chain = next.catch(() => undefined)
    return next
  }

  /** A quick step: the pointer, a key, the state. */
  private async step(command: string, params: Record<string, unknown>) {
    const hooks = this.hooks
    if (!hooks) return
    if (hooks.gpu()) {
      this.take(parse(await hooks.invoke('modalStep', command, JSON.stringify(params), true)))
    } else {
      // The software picture follows: the command moves the revision, and the canvas repaints.
      this.take(parse(await hooks.run(command, params, 'repaint')))
    }
  }

  /** Starts a transform of the selection, or one of edit mode's operations, where the pointer is. */
  begin(kind: ModalKind | ModalOperation, options: { duplicate?: boolean } = {}) {
    const hooks = this.hooks
    if (!hooks || this.active) return
    this.starting = true
    this.emit()
    const at = this.pointer
    void this.enqueue(async () => {
      try {
        const params: Record<string, unknown> = { kind, ...(at ? { at } : {}), ...(options.duplicate ? { duplicate: true } : {}) }
        // Copies and an operation's geometry show at once: the editor reads the document again.
        const refresh = options.duplicate || !['move', 'rotate', 'scale'].includes(kind) ? 'all' : 'none'
        this.take(parse(await hooks.run('modal.begin', params, refresh)))
      } catch {
        // Nothing to transform: the key did nothing.
        this.take(INACTIVE)
      } finally {
        this.starting = false
        this.emit()
      }
    })
  }

  /** The pointer moved (document pixels; Shift slows, Ctrl or Cmd steps). */
  move(at: [number, number], precision: boolean, step: boolean) {
    this.pointer = at
    const queued = this.pending != null
    this.pending = { at, precision, step }
    if (queued) return
    void this.enqueue(async () => {
      const latest = this.pending
      this.pending = null
      if (latest) await this.step('modal.pointer', latest).catch(() => undefined)
    })
  }

  /** The modifiers changed with the pointer still: the value is read again with them. */
  modifiers(precision: boolean, step: boolean) {
    if (this.pointer && this.active) this.move(this.pointer, precision, step)
  }

  key(key: string, shift: boolean, ctrl: boolean) {
    void this.enqueue(() => this.step('modal.key', { key, shift, ctrl }).catch(() => undefined))
  }

  /** Confirms the modal; a loop cut's first confirmation goes on to its slide, still under way. */
  confirm() {
    const hooks = this.hooks
    if (!hooks) return
    void this.enqueue(async () => {
      const answer = await hooks.run('modal.confirm', {}, 'all').catch(() => null)
      this.take(answer ? parse(answer) : INACTIVE)
    })
  }

  cancel() {
    const hooks = this.hooks
    if (!hooks) return
    void this.enqueue(async () => {
      await hooks.run('modal.cancel', {}, 'all').catch(() => undefined)
      this.take(INACTIVE)
    })
  }

  /** The wheel, when the modal takes it (a bevel's segments, a loop cut's cuts, proportional editing's radius); answers whether. */
  wheel(delta: number) {
    const hooks = this.hooks
    if (!hooks || (this.answer.proportional == null && !this.answer.wheel)) return false
    void this.enqueue(async () => {
      this.take(parse(await hooks.run('modal.wheel', { delta }, 'all').catch(() => null)))
    })
    return true
  }

  /** The document changed (the tools may have ended the transform): ask how it stands. */
  sync() {
    if (!this.answer.active) return
    void this.enqueue(() => this.step('modal.state', {}).catch(() => undefined))
  }
}

/** The page's one transform controller. */
export const modalTransform = new ModalTransform()
