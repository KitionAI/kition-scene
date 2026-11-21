import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode, type RefObject } from 'react'
import { PRESENCE_INTERVAL_MS, peerColor, usePresence, useKitEditorState, type HostConnection, type KitPeerPresence, type KitPresence } from '@kition/sdk/editor'

type Editor = ReturnType<typeof useKitEditorState>

/** Where a peer lands in this view, as the engine placed it (viewport device pixels). */
export type PlacedPeer = {
  principal: string
  name: string
  color: string
  agent: boolean
  objects: number[]
  editing: number | null
  pointer: [number, number] | null
  label: [number, number] | null
  vertices: Array<[number, number]>
}

type Placement = { peers: PlacedPeer[]; width: number; height: number }

const parse = (answer: unknown) => (typeof answer === 'string' ? (JSON.parse(answer) as unknown) : answer)

/**
 * Co-editing presence for the viewport (Kit UI protocol 7e): shares this person's selection,
 * pointer, and edited vertices (the engine answers them as KDM ids and a scene point), hands the
 * other people to the engine (which outlines their objects in their colours), and answers where
 * their pointers, names, and vertices land in this view. With a host that serves no presence,
 * nothing is sent and nobody is there.
 */
export function useScenePresence(host: HostConnection, editor: Editor, revision: unknown) {
  const { doc } = editor.session
  const [local, setLocal] = useState<KitPresence | null>(null)
  const { available, peers } = usePresence(host, local)
  const [placement, setPlacement] = useState<Placement | null>(null)
  const latest = useRef({ editor, available })
  latest.current = { editor, available }
  const pointer = useRef<[number, number] | null>(null)

  const share = useCallback(async () => {
    const { editor: current, available: on } = latest.current
    if (!on || !current.session.doc) return
    try {
      const answer = parse(await current.session.run('presence.local', pointer.current ? { pointer: pointer.current } : {}, null, { refresh: 'none' }))
      if (answer && typeof answer === 'object') setLocal(answer as KitPresence)
    } catch {
      // The next change shares again.
    }
  }, [])

  // The pointer, at most every PRESENCE_INTERVAL_MS.
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const movePointer = useCallback(
    (at: [number, number] | null) => {
      pointer.current = at
      if (timer.current) return
      timer.current = setTimeout(() => {
        timer.current = null
        void share()
      }, PRESENCE_INTERVAL_MS)
    },
    [share],
  )
  useEffect(() => () => {
    if (timer.current) clearTimeout(timer.current)
  }, [])

  // What this person shares follows every change (selection, mode, vertices).
  useEffect(() => {
    if (available && doc) void share()
  }, [available, doc, revision, share])

  // The other people: handed to the engine as they change; placed again after every change.
  useEffect(() => {
    if (!available || !doc) return
    let current = true
    void (async () => {
      try {
        const answer = parse(await latest.current.editor.session.run('presence.peers', { peers: peers as KitPeerPresence[] }, null, { refresh: 'repaint' }))
        if (current) setPlacement(answer as Placement)
      } catch {
        // The next change places them again.
      }
    })()
    return () => {
      current = false
    }
  }, [available, doc, peers])
  useEffect(() => {
    if (!available || !doc || !peers.length) return
    let current = true
    void (async () => {
      try {
        const answer = parse(await latest.current.editor.session.run('presence.place', {}, null, { refresh: 'none' }))
        if (current) setPlacement(answer as Placement)
      } catch {
        // Placed again on the next change.
      }
    })()
    return () => {
      current = false
    }
  }, [available, doc, revision, peers.length])

  return { available, peers: available ? (placement?.peers ?? []) : [], movePointer }
}

/** Each object's peers (by key), for the outliner's badges. */
export function peersByObject(peers: readonly PlacedPeer[]) {
  const out = new Map<number, PlacedPeer[]>()
  for (const peer of peers) {
    for (const key of new Set([...peer.objects, ...(peer.editing != null ? [peer.editing] : [])])) out.set(key, [...(out.get(key) ?? []), peer])
  }
  return out
}

/**
 * The other people over the viewport: each pointer as a cursor with the person's name, a name
 * tag at the first object they selected (their outline is the engine's), and the vertices they
 * selected in edit mode. Positions come in device pixels from the canvas's top left.
 */
export function PresenceLayer({ peers, region }: { peers: readonly PlacedPeer[]; region: RefObject<HTMLDivElement | null> }) {
  const [offset, setOffset] = useState({ x: 0, y: 0 })
  useEffect(() => {
    const element = region.current
    const canvas = element?.querySelector('canvas')
    if (!element || !canvas) return
    const outer = element.getBoundingClientRect()
    const inner = canvas.getBoundingClientRect()
    setOffset({ x: inner.left - outer.left, y: inner.top - outer.top })
  }, [peers, region])
  if (!peers.length) return null
  const dpr = window.devicePixelRatio || 1
  const at = ([x, y]: [number, number]) => ({ left: offset.x + x / dpr, top: offset.y + y / dpr })
  return (
    <div className="scene-presence" aria-hidden="true" data-testid="scene-presence">
      {peers.map((peer) => {
        const color = peerColor(peer.color)
        return (
          <div key={peer.principal} data-testid="scene-presence-peer" data-name={peer.name} data-agent={peer.agent || undefined}>
            {peer.label && !peer.pointer ? (
              <span className="scene-presence-tag" style={{ ...at(peer.label), background: color, transform: 'translate(-2px, -100%)' }}>
                {peer.name}
              </span>
            ) : null}
            {peer.vertices.map((vertex, index) => (
              <span key={index} className="scene-presence-vertex" style={{ ...at(vertex), background: color }} />
            ))}
            {peer.pointer ? (
              <span className="scene-presence-cursor" style={at(peer.pointer)} data-testid="scene-presence-cursor">
                <svg width="14" height="18" viewBox="0 0 14 18">
                  <path d="M1 1L1 15L5 11L8 17L10.5 16L7.5 10L13 10Z" fill={color} stroke="#ffffff" strokeWidth="1.2" strokeLinejoin="round" />
                </svg>
                <span className="scene-presence-tag" style={{ background: color }}>
                  {peer.name}
                </span>
              </span>
            ) : null}
          </div>
        )
      })}
    </div>
  )
}

const PeersContext = createContext<Map<number, PlacedPeer[]>>(new Map())

/** Gives the panels each object's peers. */
export function PeersProvider({ peers, children }: { peers: readonly PlacedPeer[]; children: ReactNode }) {
  const byObject = useMemo(() => peersByObject(peers), [peers])
  return <PeersContext.Provider value={byObject}>{children}</PeersContext.Provider>
}

/**
 * Who else has an object: a dot in their colour with their initial, titled with their name (and
 * "editing" when they edit its mesh). The agent shows like anyone, named Agent.
 */
export function PresenceBadges({ object, label }: { object: number; label: (name: string, editing: boolean) => string }) {
  const peers = useContext(PeersContext).get(object) ?? []
  if (!peers.length) return null
  return (
    <span className="scene-presence-badges" data-testid="scene-presence-badges">
      {peers.map((peer) => (
        <span
          key={peer.principal}
          className="scene-presence-badge"
          style={{ background: peerColor(peer.color) }}
          title={label(peer.name, peer.editing === object)}
          data-name={peer.name}
          data-editing={peer.editing === object || undefined}
        >
          {peer.name.slice(0, 1).toUpperCase()}
        </span>
      ))}
    </span>
  )
}
