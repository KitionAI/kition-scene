/**
 * Playback between the timeline (which plays) and the editor's keys (plan scene-usable-by-hand,
 * phase 8): a short tap of Space plays or pauses, Escape during playback stops it and goes back to
 * the frame it began at, as in Blender.
 */
export const PLAYBACK_EVENT = 'scene-playback'
export type PlaybackRequest = 'toggle' | 'stop'

/** Whether the timeline plays now (it keeps this up to date). */
export const playback = { playing: false }

export function requestPlayback(request: PlaybackRequest) {
  window.dispatchEvent(new CustomEvent<PlaybackRequest>(PLAYBACK_EVENT, { detail: request }))
}

/** A press of Space shorter than this, with no pointer press during it, is a tap (play or pause). */
export const TAP_MS = 250

/**
 * Space as a tap: the frame keeps holding Space for its hand tool (the kit cannot turn that off),
 * so a tap is told from a hold by its length and by no pointer press while it was down.
 */
export class SpaceTap {
  private down: { at: number; pressed: boolean } | null = null

  constructor() {
    if (typeof window !== 'undefined') {
      window.addEventListener(
        'pointerdown',
        () => {
          if (this.down) this.down.pressed = true
        },
        { capture: true, passive: true },
      )
    }
  }

  /** `at` is the event's own time stamp (when the key went down, not when the page got to it). */
  keyDown(repeat: boolean, at: number) {
    if (repeat || this.down) return
    this.down = { at, pressed: false }
  }

  /** Answers whether the press was a tap. */
  keyUp(at: number) {
    const down = this.down
    this.down = null
    return !!down && !down.pressed && at - down.at < TAP_MS
  }

  /** Focus left: the press ends without being a tap. */
  reset() {
    this.down = null
  }
}

/** The editor's one Space tap. */
export const spaceTap = new SpaceTap()
