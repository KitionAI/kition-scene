/** A node group as the engine's `inspect()` lists it (`nodeGroups`). */
export type SocketInfo = {
  id: string
  name: string
  type: string
  default?: unknown
  implicit?: string
  multi?: boolean
  /** Follows data rather than the node's kind: a group's interface, a zone's items. */
  dynamic?: boolean
}

/** A zone node's pair (the other node of its zone) and the items the zone carries. */
export type ZoneInfo = { pair: number | null; items: { name: string; type: string }[] }

export type NodeInfo = {
  id: number
  kind: string
  title: string
  category: string
  /** Where the node's top left corner sits, in canvas units (y down). */
  location: [number, number]
  values: Record<string, unknown>
  inputs: SocketInfo[]
  outputs: SocketInfo[]
  settings: { id: string; choices: string[]; value: string }[]
  /** For a zone's input and output nodes. */
  zone?: ZoneInfo
}

export type LinkInfo = { from: number; fromSocket: string; to: number; toSocket: string }

export type NodeGroupInfo = {
  id: number
  name: string
  /** What the group works on: geometry (Geometry Nodes modifiers) or a render (compositing). */
  tree: 'geometry' | 'compositor' | 'shader'
  inputs: { name: string; type: string; default: unknown }[]
  /** The outputs beside the geometry a Group node using the group gives. */
  outputs: { name: string; type: string }[]
  nodes: NodeInfo[]
  links: LinkInfo[]
}

/** A node kind as `nodes.describe` lists it. */
export type NodeKindInfo = { id: string; title: string; category: string }
