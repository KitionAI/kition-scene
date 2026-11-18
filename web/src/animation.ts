import type { KitEngineInfo } from '@kition/kit-editor'

/** The timeline as the engine's `inspect()` reports it. */
export type AnimationInfo = {
  frame: number
  start: number
  end: number
  fps: number
  /** Frames with a key on the selected objects (or their materials). */
  selectedKeyFrames: number[]
  /** Frames with any key. */
  keyFrames: number[]
  animated: boolean
}

const numbers = (value: unknown): number[] => (Array.isArray(value) ? value.map(Number).filter(Number.isFinite) : [])

export function animationOf(info: KitEngineInfo | null | undefined): AnimationInfo {
  const raw = ((info?.inspected as Record<string, any> | undefined)?.animation ?? {}) as Record<string, unknown>
  const channels = Array.isArray(raw.channels) ? raw.channels : []
  return {
    frame: Number(raw.frame ?? 1),
    start: Number(raw.start ?? 1),
    end: Number(raw.end ?? 250),
    fps: Number(raw.fps ?? 24) || 24,
    selectedKeyFrames: numbers(raw.selectedKeyFrames),
    keyFrames: numbers(raw.keyFrames),
    animated: channels.length > 0,
  }
}
