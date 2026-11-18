/** A driver variable as inspect shows it. */
export type DriverVariable = {
  name: string
  type: 'singleProperty' | 'transform' | 'distance' | 'rotationDifference'
  /** singleProperty: the object's (or material's) id. */
  object?: number | null
  property?: string
  index?: number
  path?: string
  /** transform: the object (and bone) read. */
  target?: { object: number; name: string; bone: string | null } | null
  channel?: string
  space?: 'world' | 'transform' | 'local'
  a?: { object: number; name: string; bone: string | null } | null
  b?: { object: number; name: string; bone: string | null } | null
}

/** A driver on a property of an object (or its material), as inspect shows it. */
export type DriverInfo = {
  object?: number
  material?: number
  property: string
  index: number
  path?: string
  kind: 'average' | 'sum' | 'min' | 'max' | 'scripted'
  expression: string
  variables: DriverVariable[]
  value: number | null
  error: string | null
  mapped: boolean
}

export type ActionInfo = { name: string; channels: number; properties: string[]; range: [number, number] | null }

export type KeyingSetInfo = { name: string; paths: { target: number; name: string; property: string; path: string }[] }

/** Drivers, actions, and keying sets from the engine's `inspect()`. */
export type AnimationData = {
  drivers: DriverInfo[]
  actions: ActionInfo[]
  keyingSets: { sets: KeyingSetInfo[]; active: string | null; builtins: string[] }
}

type Raw = Record<string, any>

export function animationDataOf(raw: Raw | undefined): AnimationData {
  const sets = (raw?.keyingSets ?? {}) as Raw
  return {
    drivers: Array.isArray(raw?.drivers) ? (raw.drivers as DriverInfo[]) : [],
    actions: Array.isArray(raw?.actions) ? (raw.actions as ActionInfo[]) : [],
    keyingSets: {
      sets: Array.isArray(sets.sets) ? (sets.sets as KeyingSetInfo[]) : [],
      active: typeof sets.active === 'string' ? sets.active : null,
      builtins: Array.isArray(sets.builtins) ? sets.builtins.map(String) : [],
    },
  }
}

/** An action strip of an object's NLA, as inspect shows it. */
export type NlaStripInfo = {
  kind: 'action' | 'transition' | 'meta'
  children: NlaStripInfo[]
  name: string
  action: string
  start: number
  end: number
  actionStart: number
  actionEnd: number
  scale: number
  repeat: number
  reversed: boolean
  blendIn: number
  blendOut: number
  autoBlend: boolean
  extrapolation: 'hold' | 'holdForward' | 'nothing'
  blend: 'replace' | 'combine' | 'add' | 'subtract' | 'multiply'
  influence: number | null
  muted: boolean
}

export type NlaTrackInfo = { name: string; muted: boolean; solo: boolean; strips: NlaStripInfo[] }

/** The strip being tweaked and how its action's frames map to the scene's. */
export type NlaTweakInfo = { track: number; strip: number; upperStack: boolean; start: number; actionStart: number; actionEnd: number; scale: number; reversed: boolean }

/** An object's NLA: tracks bottom first, how its own keys go on top, and tweak mode. */
export type ObjectNla = {
  tracks: NlaTrackInfo[]
  action: { blend: NlaStripInfo['blend']; influence: number; extrapolation: NlaStripInfo['extrapolation'] }
  tweak: NlaTweakInfo | null
}

export function nlaOf(raw: Raw | undefined): ObjectNla | null {
  if (!raw || typeof raw !== 'object' || !Array.isArray(raw.tracks)) return null
  return {
    tracks: raw.tracks as NlaTrackInfo[],
    action: { blend: raw.action?.blend ?? 'replace', influence: Number(raw.action?.influence ?? 1), extrapolation: raw.action?.extrapolation ?? 'hold' },
    tweak: raw.tweak && typeof raw.tweak === 'object' ? (raw.tweak as NlaTweakInfo) : null,
  }
}

/** The scene frame of a tweaked action's frame, and back. */
export function tweakToScene(tweak: NlaTweakInfo | null, frame: number): number {
  if (!tweak) return frame
  const local = tweak.reversed ? tweak.actionEnd - frame : frame - tweak.actionStart
  return tweak.start + local * (tweak.scale || 1)
}

export function sceneToTweak(tweak: NlaTweakInfo | null, frame: number): number {
  if (!tweak) return frame
  const local = (frame - tweak.start) / (tweak.scale || 1)
  return tweak.reversed ? tweak.actionEnd - local : tweak.actionStart + local
}

/** An F-curve modifier, as inspect shows it (and the modifier commands take it). */
export type FModifierInfo = {
  type: 'generator' | 'function' | 'envelope' | 'cycles' | 'noise' | 'stepped' | 'limits'
  muted?: boolean
  influence?: number | null
  range?: { start: number; end: number; blendIn: number; blendOut: number } | null
  [field: string]: unknown
}
