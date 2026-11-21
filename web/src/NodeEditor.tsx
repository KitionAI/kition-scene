import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent as ReactPointerEvent } from 'react'
import { Checkbox, Choice, useKitEditor, useReadOnly } from '@kition/sdk/editor'
import type { Modifier, SceneInfo, SceneObject, Vec3 } from './scene'
import type { LinkInfo, NodeGroupInfo, NodeInfo, NodeKindInfo, SocketInfo } from './NodeEditorTypes'
import { NumberInput, Row, VectorRow } from './SidePanel'
import { CompositorBackdrop, NodeColor, NodeCurves, NodeImage, NodeMatte } from './Compositor'
import './NodeEditor.css'

const WIDTH = 200
const HEADER = 25
const ROW = 24

type Point = { x: number; y: number }
type View = { x: number; y: number; zoom: number }
type Drag =
  | { kind: 'pan'; start: Point; view: View; moved: boolean }
  | { kind: 'nodes'; start: Point; ids: number[]; offset: Point }
  | { kind: 'link'; from: number; fromSocket: string; type: string; at: Point; detach: LinkInfo | null }
type Menu = { at: Point; query: string; active: number }

/** Whether a node is a zone's output node. */
function isZoneOutput(node: NodeInfo) {
  return node.kind === 'repeatOutput' || node.kind === 'simulationOutput'
}

/** Rows a node draws between its settings and its inputs: a Group node's group, the row adding
 *  a zone's items or a group's inputs and outputs. */
function extraRows(node: NodeInfo) {
  return node.kind === 'group' || isZoneOutput(node) || node.kind === 'groupInput' || node.kind === 'groupOutput' ? 1 : 0
}

/** The rows a node draws, top to bottom: outputs, settings, its extra row, then inputs. */
function rowsOf(node: NodeInfo) {
  return node.outputs.length + node.settings.length + extraRows(node) + node.inputs.length
}

/** The types a zone's items and a group's outputs take. */
const ITEM_TYPES = ['geometry', 'float', 'int', 'bool', 'vector', 'rotation'] as const

/** A name `base` numbered past the ones taken. */
function unusedName(base: string, taken: string[]) {
  if (!taken.includes(base)) return base
  let count = 2
  while (taken.includes(`${base} ${count}`)) count += 1
  return `${base} ${count}`
}

/** Each zone's nodes, from its input: the input, the output, and the nodes between them. */
function zonesOf(group: NodeGroupInfo): { input: number; output: number; kind: string; nodes: number[] }[] {
  const out: { input: number; output: number; kind: string; nodes: number[] }[] = []
  for (const node of group.nodes) {
    if ((node.kind !== 'repeatInput' && node.kind !== 'simulationInput') || node.zone?.pair == null) continue
    const output = node.zone.pair
    const reach = (start: number, forward: boolean, stop: number) => {
      const seen = new Set<number>()
      const pending = [start]
      while (pending.length) {
        const at = pending.pop()!
        for (const link of group.links) {
          const [from, to] = forward ? [link.from, link.to] : [link.to, link.from]
          if (from === at && to !== stop && !seen.has(to)) {
            seen.add(to)
            pending.push(to)
          }
        }
      }
      return seen
    }
    const after = reach(node.id, true, output)
    const before = reach(output, false, node.id)
    out.push({ input: node.id, output, kind: node.kind === 'repeatInput' ? 'repeat' : 'simulation', nodes: [node.id, output, ...[...after].filter((id) => before.has(id))] })
  }
  return out
}

/** How much taller than a row a node's curves are. */
const CURVES_EXTRA = 140

function heightOf(node: NodeInfo) {
  return HEADER + rowsOf(node) * ROW + 4 + (node.inputs.some((socket) => socket.type === 'curves') ? CURVES_EXTRA : 0)
}

/** Inputs set on the node, never linked: RGB Curves' curves, an Image node's picture, a
 *  Cryptomatte node's matte. */
function dataOnly(node: NodeInfo, socket: SocketInfo) {
  return socket.type === 'curves' || ((node.kind === 'image' || node.kind === 'cryptomatte') && socket.type === 'string')
}

/** A setting's choice as a label: `fogGlow` and `multiply_add` read as words. */
function choiceLabel(choice: string) {
  if (choice === 'nurbs') return 'NURBS'
  if (choice === 'catmullRom') return 'Catmull Rom'
  if (choice === 'bezier') return 'Bézier'
  if (choice === 'poly') return 'Poly'
  return choice.replace(/_/g, ' ').replace(/([a-z])([A-Z])/g, '$1 $2')
}

/** Where a socket's dot sits in canvas units. */
function socketAt(node: NodeInfo, side: 'input' | 'output', socket: string): Point | null {
  const index = side === 'output' ? node.outputs.findIndex((item) => item.id === socket) : node.inputs.findIndex((item) => item.id === socket)
  if (index < 0) return null
  const row = side === 'output' ? index : node.outputs.length + node.settings.length + extraRows(node) + index
  return { x: node.location[0] + (side === 'output' ? WIDTH : 0), y: node.location[1] + HEADER + row * ROW + ROW / 2 }
}

function curve(from: Point, to: Point) {
  const bend = Math.max(40, Math.abs(to.x - from.x) / 2)
  return `M${from.x},${from.y} C${from.x + bend},${from.y} ${to.x - bend},${to.y} ${to.x},${to.y}`
}

/** The group the active object's first Geometry Nodes modifier uses, else the first group. */
function defaultGroup(info: SceneInfo): number | null {
  const active = info.objects.find((object) => object.id === info.active)
  const used = active?.modifiers.find((modifier) => modifier.type === 'geometryNodes' && typeof modifier.group === 'number')
  if (used && info.nodeGroups.some((group) => group.id === used.group)) return Number(used.group)
  return info.nodeGroups[0]?.id ?? null
}

function parsed(result: unknown): any {
  if (typeof result === 'string') {
    try {
      return JSON.parse(result)
    } catch {
      return null
    }
  }
  return result
}

/** One unlinked input's value: a number, a vector, a checkbox, a colour, curves, an object, a
 *  picture, or a text. */
function SocketValue({ node, group, socket, info, onSet, disabled }: { node: NodeInfo; group: NodeGroupInfo; socket: SocketInfo; info: SceneInfo; onSet: (value: unknown) => void; disabled: boolean }) {
  const { t } = useKitEditor()
  const value = node.values[socket.id] ?? socket.default
  const stop = (event: KeyboardEvent) => event.stopPropagation()
  const number = (current: unknown, commit: (next: number) => void, key: string) => (
    <input
      key={key}
      className="scene-node-input"
      type="number"
      step={socket.type === 'int' ? 1 : 0.1}
      defaultValue={Number(current ?? 0)}
      disabled={disabled}
      onKeyDown={(event) => {
        stop(event)
        if (event.key === 'Enter') (event.target as HTMLInputElement).blur()
      }}
      onBlur={(event) => {
        const next = Number(event.target.value)
        if (Number.isFinite(next) && next !== Number(current ?? 0)) commit(socket.type === 'int' ? Math.round(next) : next)
      }}
      data-testid={`scene-node-${node.id}-${socket.id}-${key}`}
    />
  )
  if (socket.implicit && node.values[socket.id] == null) return null
  switch (socket.type) {
    case 'float':
    case 'int':
      return number(value, onSet, `${value}`)
    case 'vector':
    case 'rotation': {
      const vector = Array.isArray(value) ? (value as number[]) : [0, 0, 0]
      return (
        <>
          {[0, 1, 2].map((axis) =>
            number(
              vector[axis],
              (next) => {
                const out = [...vector]
                out[axis] = next
                onSet(out)
              },
              `${axis}-${vector[axis]}`,
            ),
          )}
        </>
      )
    }
    case 'color':
      return <NodeColor node={node} id={socket.id} value={value} disabled={disabled} onSet={onSet} />
    case 'curves':
      return <NodeCurves node={node} disabled={disabled} onSet={onSet} />
    case 'bool':
      return <input type="checkbox" checked={value === true} disabled={disabled} onChange={(event) => onSet(event.target.checked)} data-testid={`scene-node-${node.id}-${socket.id}`} />
    case 'object':
    case 'material': {
      const items = socket.type === 'object' ? info.objects.map((object) => [object.id, object.name] as const) : info.materials.map((material) => [material.id, material.name] as const)
      return (
        <select className="scene-node-input" value={typeof value === 'number' ? String(value) : ''} disabled={disabled} onChange={(event) => onSet(event.target.value === '' ? null : Number(event.target.value))} onKeyDown={stop}>
          <option value="">{t('noneChoice')}</option>
          {items.map(([id, name]) => (
            <option key={id} value={id}>
              {name}
            </option>
          ))}
        </select>
      )
    }
    case 'string':
      if (node.kind === 'image') return <NodeImage node={node} group={group} info={info} disabled={disabled} />
      if (node.kind === 'cryptomatte') return <NodeMatte node={node} group={group} value={String(value ?? '')} disabled={disabled} onSet={onSet} />
      return (
        <input
          key={String(value ?? '')}
          className="scene-node-input"
          defaultValue={String(value ?? '')}
          disabled={disabled}
          onKeyDown={(event) => {
            stop(event)
            if (event.key === 'Enter') (event.target as HTMLInputElement).blur()
          }}
          onBlur={(event) => event.target.value !== String(value ?? '') && onSet(event.target.value)}
        />
      )
    default:
      return null
  }
}

/** One node box. */
function NodeBox({
  node,
  group,
  info,
  selected,
  offset,
  onHeader,
  onSocket,
  onEnter,
}: {
  node: NodeInfo
  group: NodeGroupInfo
  info: SceneInfo
  selected: boolean
  offset: Point | null
  onHeader: (event: ReactPointerEvent, node: NodeInfo) => void
  onSocket: (event: ReactPointerEvent, node: NodeInfo, side: 'input' | 'output', socket: SocketInfo) => void
  onEnter: (group: number) => void
}) {
  const { run, t, phrase: p } = useKitEditor()
  const readOnly = useReadOnly()
  const set = (id: string, value: unknown) => void run('nodes.setValue', { group: group.id, node: node.id, id, value })
  const linked = (socket: string) => group.links.some((link) => link.to === node.id && link.toSocket === socket)
  const [itemType, setItemType] = useState<string>('float')
  const used = typeof node.values.group === 'number' ? node.values.group : null
  const stopKeys = (event: KeyboardEvent) => event.stopPropagation()
  // Removing a socket that follows data: a zone's item, a group's input or output.
  const removable = (socket: SocketInfo, side: 'input' | 'output') =>
    !readOnly &&
    socket.dynamic === true &&
    socket.id !== 'Geometry' &&
    ((isZoneOutput(node) && side === 'input') || (node.kind === 'groupInput' && side === 'output') || (node.kind === 'groupOutput' && side === 'input'))
  const remove = (socket: SocketInfo) => {
    if (isZoneOutput(node)) void run('nodes.removeZoneItem', { group: group.id, node: node.id, name: socket.id })
    else void run(node.kind === 'groupInput' ? 'nodes.removeInput' : 'nodes.removeOutput', { group: group.id, name: socket.id })
  }
  const addSocket = () => {
    if (isZoneOutput(node)) {
      const taken = node.zone?.items.map((item) => item.name) ?? []
      void run('nodes.addZoneItem', { group: group.id, node: node.id, name: unusedName(choiceLabel(itemType).replace(/^./, (c) => c.toUpperCase()), taken), type: itemType })
    } else if (node.kind === 'groupInput') {
      const taken = group.inputs.map((input) => input.name)
      void run('nodes.addInput', { group: group.id, name: unusedName(t('groupInputName', { count: taken.length + 1 }), taken), type: itemType })
    } else {
      const taken = group.outputs.map((output) => output.name)
      void run('nodes.addOutput', { group: group.id, name: unusedName(t('groupOutputName', { count: taken.length + 1 }), taken), type: itemType })
    }
  }
  const left = node.location[0] + (offset?.x ?? 0)
  const top = node.location[1] + (offset?.y ?? 0)
  return (
    <div className="scene-node" style={{ left, top }} data-selected={selected || undefined} data-tall={node.inputs.some((socket) => socket.type === 'curves') || undefined} data-testid={`scene-node-${node.id}`} data-kind={node.kind} onPointerDown={(event) => event.stopPropagation()}>
      <div
        className="scene-node-header"
        data-category={node.category}
        onPointerDown={(event) => onHeader(event, node)}
        onDoubleClick={() => used != null && onEnter(used)}
        title={node.kind === 'group' ? node.title : p(node.title)}
      >
        {node.kind === 'group' ? node.title : p(node.title)}
        {node.kind === 'group' && used != null ? (
          <button type="button" className="scene-node-enter" onPointerDown={(event) => event.stopPropagation()} onClick={() => onEnter(used)} title={t('enterGroup')} data-testid={`scene-node-${node.id}-enter`}>
            {t('enterGroup')}
          </button>
        ) : null}
      </div>
      {node.outputs.map((socket) => (
        <div key={`o-${socket.id}`} className="scene-node-row" data-side="output">
          {removable(socket, 'output') ? (
            <button type="button" className="scene-node-remove" onClick={() => remove(socket)} title={t('removeSocket')} data-testid={`scene-node-${node.id}-remove-${socket.id}`}>
              ×
            </button>
          ) : null}
          <span className="scene-node-row-label">{socket.dynamic ? socket.name : p(socket.name)}</span>
          <span className="scene-node-socket" data-side="output" data-type={socket.type} data-node={node.id} data-socket={socket.id} onPointerDown={(event) => onSocket(event, node, 'output', socket)} data-testid={`scene-socket-${node.id}-out-${socket.id}`} />
        </div>
      ))}
      {node.settings.map((setting) => (
        <div key={`s-${setting.id}`} className="scene-node-row">
          <span className="scene-node-row-value">
            <select className="scene-node-input" value={setting.value} disabled={readOnly} onChange={(event) => set(setting.id, setting.choices.length === 2 && setting.choices[0] === 'false' ? event.target.value === 'true' : event.target.value)} onKeyDown={(event) => event.stopPropagation()} data-testid={`scene-node-${node.id}-${setting.id}`}>
              {setting.choices.map((choice) => (
                <option key={choice} value={choice}>
                  {setting.choices.length === 2 && setting.choices[0] === 'false' ? `${setting.id === 'useClamp' || setting.id === 'clamp' ? 'Clamp' : choiceLabel(setting.id)}: ${choice}` : p(choiceLabel(choice))}
                </option>
              ))}
            </select>
          </span>
        </div>
      ))}
      {node.kind === 'group' ? (
        <div className="scene-node-row">
          <span className="scene-node-row-value">
            <select
              className="scene-node-input"
              value={used == null ? '' : String(used)}
              disabled={readOnly}
              onChange={(event) => event.target.value !== '' && set('group', Number(event.target.value))}
              onKeyDown={stopKeys}
              data-testid={`scene-node-${node.id}-group`}
            >
              <option value="">{t('noneChoice')}</option>
              {info.nodeGroups
                .filter((other) => other.tree === 'geometry' && other.id !== group.id)
                .map((other) => (
                  <option key={other.id} value={other.id}>
                    {other.name}
                  </option>
                ))}
            </select>
          </span>
        </div>
      ) : null}
      {isZoneOutput(node) || node.kind === 'groupInput' || node.kind === 'groupOutput' ? (
        <div className="scene-node-row">
          <span className="scene-node-row-value">
            <select className="scene-node-input" value={itemType} disabled={readOnly} onChange={(event) => setItemType(event.target.value)} onKeyDown={stopKeys} data-testid={`scene-node-${node.id}-item-type`}>
              {ITEM_TYPES.map((type) => (
                <option key={type} value={type}>
                  {p(choiceLabel(type))}
                </option>
              ))}
            </select>
            <button type="button" className="kui-button" data-size="sm" disabled={readOnly} onClick={addSocket} data-testid={`scene-node-${node.id}-add-item`}>
              {t(isZoneOutput(node) ? 'addZoneItem' : node.kind === 'groupInput' ? 'addGroupInput' : 'addGroupOutput')}
            </button>
          </span>
        </div>
      ) : null}
      {node.inputs.map((socket) => (
        <div key={`i-${socket.id}`} className="scene-node-row" data-side="input" data-data={dataOnly(node, socket) || undefined}>
          {dataOnly(node, socket) ? null : (
            <span className="scene-node-socket" data-side="input" data-type={socket.type} data-node={node.id} data-socket={socket.id} onPointerDown={(event) => onSocket(event, node, 'input', socket)} data-testid={`scene-socket-${node.id}-in-${socket.id}`} />
          )}
          {socket.type === 'curves' ? null : <span className="scene-node-row-label">{socket.dynamic ? socket.name : p(socket.name)}</span>}
          {removable(socket, 'input') ? (
            <button type="button" className="scene-node-remove" onClick={() => remove(socket)} title={t('removeSocket')} data-testid={`scene-node-${node.id}-remove-${socket.id}`}>
              ×
            </button>
          ) : null}
          {linked(socket.id) || socket.type === 'geometry' ? null : (
            <span className="scene-node-row-value">
              <SocketValue node={node} group={group} socket={socket} info={info} disabled={readOnly} onSet={(value) => set(socket.id, value)} />
            </span>
          )}
        </div>
      ))}
    </div>
  )
}

/**
 * The geometry node editor: the group the active object's Geometry Nodes modifier uses (or any
 * other) as boxes on a canvas. Drag the background to pan, the wheel zooms, drag a node's title to
 * move it, drag from an output dot to an input dot to link (from a linked input to take its link
 * elsewhere), Shift+A or the button adds a node, Delete removes the selected ones, F frames all.
 */
export function NodeEditor({ info }: { info: SceneInfo }) {
  const { run, t, phrase: p } = useKitEditor()
  const readOnly = useReadOnly()
  const canvas = useRef<HTMLDivElement>(null)
  const [chosen, setChosen] = useState<number | null>(null)
  // The groups entered through Group nodes, from the chosen group down.
  const [path, setPath] = useState<number[]>([])
  const [view, setView] = useState<View>({ x: 0, y: 0, zoom: 1 })
  const [selected, setSelected] = useState<number[]>([])
  const [drag, setDrag] = useState<Drag | null>(null)
  const [menu, setMenu] = useState<Menu | null>(null)
  const [kinds, setKinds] = useState<{ tree: string; kinds: NodeKindInfo[] } | null>(null)
  const [backdrop, setBackdrop] = useState(true)
  const pointer = useRef<Point>({ x: 120, y: 80 })
  const framed = useRef<number | null>(null)
  const topId = chosen != null && info.nodeGroups.some((group) => group.id === chosen) ? chosen : defaultGroup(info)
  const entered = path.filter((id) => info.nodeGroups.some((group) => group.id === id))
  const groupId = entered.length ? entered[entered.length - 1] : topId
  const group = info.nodeGroups.find((item) => item.id === groupId) ?? null
  const enter = (id: number) => {
    setPath([...entered, id])
    setSelected([])
  }
  const leave = (depth: number) => {
    setPath(entered.slice(0, depth))
    setSelected([])
  }
  const active = info.objects.find((object) => object.id === info.active && object.type === 'mesh') ?? null

  const toCanvas = (point: Point): Point => ({ x: (point.x - view.x) / view.zoom, y: (point.y - view.y) / view.zoom })
  const local = (event: { clientX: number; clientY: number }): Point => {
    const box = canvas.current?.getBoundingClientRect()
    return box ? { x: event.clientX - box.left, y: event.clientY - box.top } : { x: 0, y: 0 }
  }

  const frameAll = () => {
    const box = canvas.current?.getBoundingClientRect()
    if (!group || !box || !group.nodes.length) return
    const left = Math.min(...group.nodes.map((node) => node.location[0]))
    const top = Math.min(...group.nodes.map((node) => node.location[1]))
    const right = Math.max(...group.nodes.map((node) => node.location[0] + WIDTH))
    const bottom = Math.max(...group.nodes.map((node) => node.location[1] + heightOf(node)))
    const zoom = Math.min(1.25, Math.max(0.2, Math.min(box.width / (right - left + 80), box.height / (bottom - top + 60))))
    setView({ zoom, x: box.width / 2 - ((left + right) / 2) * zoom, y: box.height / 2 - ((top + bottom) / 2) * zoom })
  }

  // A group shown for the first time is framed.
  useEffect(() => {
    if (group && framed.current !== group.id) {
      framed.current = group.id
      frameAll()
    }
  })

  const tree = group?.tree ?? 'geometry'
  const openMenu = (at: Point) => {
    setMenu({ at, query: '', active: 0 })
    if (kinds?.tree !== tree) {
      void run('nodes.describe', { tree }).then((result) => {
        const described = parsed(result)
        // A zone is added by its input (its output comes with it).
        const hidden = ['groupInput', 'groupOutput', 'repeatOutput', 'simulationOutput']
        const zones: Record<string, string> = { repeatInput: 'Repeat Zone', simulationInput: 'Simulation Zone' }
        if (Array.isArray(described?.kinds))
          setKinds({ tree, kinds: described.kinds.filter((kind: NodeKindInfo) => !hidden.includes(kind.id)).map((kind: NodeKindInfo) => (zones[kind.id] ? { ...kind, title: zones[kind.id] } : kind)) })
      })
    }
  }

  const matches = useMemo(() => {
    const query = menu?.query.trim().toLowerCase() ?? ''
    return (kinds?.tree === tree ? kinds.kinds : []).filter((kind) => !query || kind.title.toLowerCase().includes(query) || p(kind.title).toLowerCase().includes(query) || kind.id.toLowerCase().includes(query))
  }, [kinds, tree, menu?.query, p])

  const addNode = (kind: NodeKindInfo) => {
    if (!group || !menu) return
    const at = toCanvas(menu.at)
    setMenu(null)
    void run('nodes.addNode', { group: group.id, kind: kind.id, location: [Math.round(at.x), Math.round(at.y)] })
    canvas.current?.focus({ preventScroll: true })
  }

  const removeSelected = () => {
    if (!group || !selected.length || readOnly) return
    void run('nodes.removeNodes', { group: group.id, nodes: selected })
    setSelected([])
  }

  const onHeader = (event: ReactPointerEvent, node: NodeInfo) => {
    if (event.button !== 0) return
    event.stopPropagation()
    canvas.current?.focus({ preventScroll: true })
    const ids = event.shiftKey ? (selected.includes(node.id) ? selected.filter((id) => id !== node.id) : [...selected, node.id]) : selected.includes(node.id) ? selected : [node.id]
    setSelected(ids)
    if (readOnly) return
    canvas.current?.setPointerCapture(event.pointerId)
    setDrag({ kind: 'nodes', start: local(event), ids, offset: { x: 0, y: 0 } })
  }

  const onSocket = (event: ReactPointerEvent, node: NodeInfo, side: 'input' | 'output', socket: SocketInfo) => {
    if (event.button !== 0 || readOnly || !group) return
    event.stopPropagation()
    canvas.current?.setPointerCapture(event.pointerId)
    const at = toCanvas(local(event))
    if (side === 'output') {
      setDrag({ kind: 'link', from: node.id, fromSocket: socket.id, type: socket.type, at, detach: null })
      return
    }
    // From a linked input: take its link to wherever it is dropped.
    const existing = group.links.find((link) => link.to === node.id && link.toSocket === socket.id)
    if (existing) {
      const source = group.nodes.find((item) => item.id === existing.from)
      const type = source?.outputs.find((item) => item.id === existing.fromSocket)?.type ?? socket.type
      setDrag({ kind: 'link', from: existing.from, fromSocket: existing.fromSocket, type, at, detach: existing })
    }
  }

  const onPointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    canvas.current?.focus({ preventScroll: true })
    setMenu(null)
    if (event.button !== 0 && event.button !== 1) return
    canvas.current?.setPointerCapture(event.pointerId)
    setDrag({ kind: 'pan', start: local(event), view, moved: false })
  }

  const onPointerMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    const at = local(event)
    pointer.current = at
    if (!drag) return
    if (drag.kind === 'pan') {
      const dx = at.x - drag.start.x
      const dy = at.y - drag.start.y
      setView({ ...drag.view, x: drag.view.x + dx, y: drag.view.y + dy })
      if (!drag.moved && Math.hypot(dx, dy) > 3) setDrag({ ...drag, moved: true })
    } else if (drag.kind === 'nodes') {
      setDrag({ ...drag, offset: { x: (at.x - drag.start.x) / view.zoom, y: (at.y - drag.start.y) / view.zoom } })
    } else {
      setDrag({ ...drag, at: toCanvas(at) })
    }
  }

  const onPointerUp = (event: ReactPointerEvent<HTMLDivElement>) => {
    const current = drag
    setDrag(null)
    if (!current || !group) return
    if (current.kind === 'pan') {
      if (!current.moved) setSelected([])
    } else if (current.kind === 'nodes') {
      const { x, y } = current.offset
      if (Math.hypot(x, y) > 0.5) void run('nodes.moveNodes', { group: group.id, nodes: current.ids, offset: [Math.round(x), Math.round(y)] })
    } else {
      const target = document.elementFromPoint(event.clientX, event.clientY)?.closest<HTMLElement>('.scene-node-socket[data-side="input"]')
      const to = target ? Number(target.dataset.node) : null
      const toSocket = target?.dataset.socket ?? null
      const detach = current.detach
      if (detach && (to !== detach.to || toSocket !== detach.toSocket)) {
        void run('nodes.unlink', { group: group.id, to: detach.to, toSocket: detach.toSocket, from: detach.from, fromSocket: detach.fromSocket })
      }
      if (to != null && toSocket && !(detach && to === detach.to && toSocket === detach.toSocket)) {
        void run('nodes.link', { group: group.id, from: current.from, fromSocket: current.fromSocket, to, toSocket })
      }
    }
  }

  const onWheel = (event: React.WheelEvent<HTMLDivElement>) => {
    const at = local(event)
    const zoom = Math.min(2.5, Math.max(0.15, view.zoom * Math.exp(-event.deltaY * 0.0015)))
    setView({ zoom, x: at.x - ((at.x - view.x) / view.zoom) * zoom, y: at.y - ((at.y - view.y) / view.zoom) * zoom })
  }

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.target !== canvas.current) return
    if ((event.key === 'Delete' || event.key === 'Backspace' || event.key === 'x' || event.key === 'X') && !event.metaKey && !event.ctrlKey) {
      event.preventDefault()
      event.stopPropagation()
      removeSelected()
    } else if ((event.key === 'a' || event.key === 'A') && event.shiftKey && !readOnly) {
      event.preventDefault()
      event.stopPropagation()
      openMenu(pointer.current)
    } else if (event.key === 'Tab' && !event.metaKey && !event.ctrlKey) {
      // Tab enters the selected Group node's group; Shift+Tab (or Tab with none) goes back up.
      event.preventDefault()
      event.stopPropagation()
      const node = selected.length === 1 ? group?.nodes.find((item) => item.id === selected[0]) : undefined
      const used = node?.kind === 'group' && typeof node.values.group === 'number' ? node.values.group : null
      if (used != null && !event.shiftKey) enter(used)
      else if (entered.length) leave(entered.length - 1)
    } else if ((event.key === 'f' || event.key === 'F' || event.key === 'Home') && !event.metaKey && !event.ctrlKey) {
      event.preventDefault()
      event.stopPropagation()
      frameAll()
    } else if (!event.metaKey && !event.ctrlKey) {
      // The viewport's single-key shortcuts do not reach through the node editor.
      event.stopPropagation()
    }
  }

  const nodeById = (id: number) => group?.nodes.find((node) => node.id === id)
  const moved = (node: NodeInfo): NodeInfo =>
    drag?.kind === 'nodes' && drag.ids.includes(node.id) ? { ...node, location: [node.location[0] + drag.offset.x, node.location[1] + drag.offset.y] } : node
  const categories = [...new Set(matches.map((kind) => kind.category))]
  let counter = 0

  return (
    <section className="scene-nodes" data-testid="scene-nodes" onKeyDown={(event) => (event.target as HTMLElement).tagName !== 'DIV' && event.stopPropagation()}>
      <div className="scene-nodes-header">
        <strong>{p(tree === 'compositor' ? 'Compositing' : tree === 'shader' ? 'Shader Nodes' : 'Geometry Nodes')}</strong>
        {info.nodeGroups.length ? (
          <Choice label={t('nodeGroup')} quiet value={String(topId ?? '')} choices={info.nodeGroups.map((item) => [String(item.id), item.name] as const)} onChange={(value) => {
              setChosen(Number(value))
              setPath([])
            }} testId="scene-nodes-group" />
        ) : null}
        {entered.length ? (
          <nav className="scene-nodes-path" aria-label={t('groupPath')} data-testid="scene-nodes-path">
            {[topId, ...entered].map((id, depth) => (
              <span key={`${depth}-${id}`} className="scene-nodes-crumb">
                {depth > 0 ? <span className="scene-nodes-crumb-gap">›</span> : null}
                <button type="button" className="kui-button" data-size="sm" data-current={depth === entered.length || undefined} disabled={depth === entered.length} onClick={() => leave(depth)} data-testid={`scene-nodes-crumb-${depth}`}>
                  {info.nodeGroups.find((item) => item.id === id)?.name ?? ''}
                </button>
              </span>
            ))}
          </nav>
        ) : null}
        <button type="button" className="kui-button" data-size="sm" disabled={readOnly || !active} onClick={() => void run('nodes.newGroup', active ? { object: active.id } : {}).then(() => setChosen(null))} data-testid="scene-nodes-new">
          {t('newNodeGroup')}
        </button>
        <button type="button" className="kui-button" data-size="sm" disabled={readOnly} onClick={() => void run('compositor.new', {}).then((result) => setChosen(parsed(result)?.group ?? null))} data-testid="scene-nodes-new-compositor">
          {t('newCompositor')}
        </button>
        {tree === 'compositor' ? <Checkbox label={t('backdrop')} checked={backdrop} onChange={setBackdrop} testId="scene-nodes-backdrop" /> : null}
        <span className="scene-nodes-spacer" />
        <button type="button" className="kui-button" data-size="sm" disabled={readOnly || !group} onClick={() => openMenu({ x: 24, y: 12 })} data-testid="scene-nodes-add">
          {t('addNode')}
        </button>
        <button type="button" className="kui-button" data-size="sm" disabled={readOnly || !selected.length} onClick={removeSelected} data-testid="scene-nodes-delete">
          {t('deleteNodes')}
        </button>
        <button type="button" className="kui-button" data-size="sm" disabled={!group} onClick={frameAll} data-testid="scene-nodes-frame">
          {t('frameAll')}
        </button>
      </div>
      <div
        ref={canvas}
        className="scene-nodes-canvas"
        tabIndex={0}
        data-panning={drag?.kind === 'pan' && drag.moved ? true : undefined}
        style={{ backgroundSize: `${20 * view.zoom}px ${20 * view.zoom}px`, backgroundPosition: `${view.x}px ${view.y}px` }}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={() => setDrag(null)}
        onWheel={onWheel}
        onKeyDown={onKeyDown}
        data-testid="scene-nodes-canvas"
      >
        {group ? (
          <div className="scene-nodes-world" style={{ transform: `translate(${view.x}px, ${view.y}px) scale(${view.zoom})` }}>
            <svg className="scene-nodes-links" width={1} height={1}>
              {group.links.map((link) => {
                const from = nodeById(link.from)
                const to = nodeById(link.to)
                if (!from || !to) return null
                if (drag?.kind === 'link' && drag.detach && drag.detach.to === link.to && drag.detach.toSocket === link.toSocket) return null
                const start = socketAt(moved(from), 'output', link.fromSocket)
                const end = socketAt(moved(to), 'input', link.toSocket)
                if (!start || !end) return null
                const type = from.outputs.find((item) => item.id === link.fromSocket)?.type
                return <path key={`${link.from}-${link.fromSocket}-${link.to}-${link.toSocket}`} className="scene-nodes-link" data-type={type} d={curve(start, end)} />
              })}
              {drag?.kind === 'link'
                ? (() => {
                    const from = nodeById(drag.from)
                    const start = from ? socketAt(from, 'output', drag.fromSocket) : null
                    return start ? <path className="scene-nodes-link" data-type={drag.type} data-dragging d={curve(start, drag.at)} /> : null
                  })()
                : null}
            </svg>
            {zonesOf(group).map((zone) => {
              const boxes = zone.nodes.map((id) => nodeById(id)).filter((node): node is NodeInfo => node != null).map(moved)
              if (!boxes.length) return null
              const left = Math.min(...boxes.map((node) => node.location[0])) - 16
              const top = Math.min(...boxes.map((node) => node.location[1])) - 16
              const right = Math.max(...boxes.map((node) => node.location[0] + WIDTH)) + 16
              const bottom = Math.max(...boxes.map((node) => node.location[1] + heightOf(node))) + 16
              return <div key={`zone-${zone.input}`} className="scene-nodes-zone" data-kind={zone.kind} style={{ left, top, width: right - left, height: bottom - top }} data-testid={`scene-nodes-zone-${zone.input}`} />
            })}
            {group.nodes.map((node) => (
              <NodeBox
                key={node.id}
                node={node}
                group={group}
                info={info}
                selected={selected.includes(node.id)}
                offset={drag?.kind === 'nodes' && drag.ids.includes(node.id) ? drag.offset : null}
                onHeader={onHeader}
                onSocket={onSocket}
                onEnter={enter}
              />
            ))}
          </div>
        ) : (
          <div className="scene-nodes-empty">{t('noNodeGroup')}</div>
        )}
        {menu ? (
          <div className="scene-nodes-menu" style={{ left: menu.at.x, top: menu.at.y }} onPointerDown={(event) => event.stopPropagation()} data-testid="scene-nodes-menu">
            <input
              autoFocus
              placeholder={t('searchNodes')}
              value={menu.query}
              onChange={(event) => setMenu({ ...menu, query: event.target.value, active: 0 })}
              onKeyDown={(event) => {
                event.stopPropagation()
                if (event.key === 'Escape') setMenu(null)
                else if (event.key === 'ArrowDown') setMenu({ ...menu, active: Math.min(matches.length - 1, menu.active + 1) })
                else if (event.key === 'ArrowUp') setMenu({ ...menu, active: Math.max(0, menu.active - 1) })
                else if (event.key === 'Enter' && matches.length) {
                  const ordered = categories.flatMap((category) => matches.filter((kind) => kind.category === category))
                  const kind = ordered[menu.active] ?? ordered[0]
                  if (kind) addNode(kind)
                }
              }}
              data-testid="scene-nodes-search"
            />
            {matches.length ? null : <div className="scene-nodes-menu-category">{t('noNodesFound')}</div>}
            {categories.map((category) => (
              <div key={category}>
                <div className="scene-nodes-menu-category">{p(category)}</div>
                {matches
                  .filter((kind) => kind.category === category)
                  .map((kind) => {
                    const index = counter++
                    return (
                      <button key={kind.id} type="button" className="scene-nodes-menu-item" data-active={index === menu.active || undefined} onClick={() => addNode(kind)} data-testid={`scene-nodes-add-${kind.id}`}>
                        {p(kind.title)}
                      </button>
                    )
                  })}
              </div>
            ))}
          </div>
        ) : null}
        {group && tree === 'compositor' && backdrop && info.camera != null ? (
          <CompositorBackdrop group={group} revision={`${JSON.stringify(group)}|${JSON.stringify(info.world)}|${info.render.width}x${info.render.height}|${info.objects.length}`} />
        ) : null}
        <span className="scene-nodes-hint">{t('nodesHint')}</span>
      </div>
    </section>
  )
}

/** A Geometry Nodes modifier's settings: its group, and the group's inputs as fields. */
export function GeometryNodesFields({ object, info, modifier, index }: { object: SceneObject; info: SceneInfo; modifier: Modifier; index: number }) {
  const { run, t } = useKitEditor()
  const readOnly = useReadOnly()
  const group = info.nodeGroups.find((item) => item.id === modifier.group) ?? null
  const inputs = (modifier.inputs as Record<string, unknown> | undefined) ?? {}
  const set = (values: Record<string, unknown>) => void run('modifier.set', { object: object.id, index, ...values })
  const setInput = (name: string, value: unknown) => set({ inputs: { ...inputs, [name]: value } })
  return (
    <div className="scene-gn-fields">
      <Row label={t('nodeGroup')}>
        <span className="scene-gn-group">
          <Choice
            label={t('nodeGroup')}
            quiet
            value={group ? String(group.id) : ''}
            choices={[['', t('noneChoice')] as const, ...info.nodeGroups.filter((item) => item.tree === 'geometry').map((item) => [String(item.id), item.name] as const)]}
            disabled={readOnly}
            onChange={(value) => set({ group: value === '' ? null : Number(value) })}
            testId={`scene-modifier-${index}-group`}
          />
          <button type="button" className="kui-button" data-size="sm" disabled={readOnly} onClick={() => void run('nodes.newGroup', { object: object.id, modifier: index })} data-testid={`scene-modifier-${index}-new-group`}>
            {t('newNodeGroup')}
          </button>
        </span>
      </Row>
      {group?.inputs.map((input) => {
        const value = inputs[input.name] ?? input.default
        switch (input.type) {
          case 'float':
          case 'int':
            return <NumberInput key={input.name} label={input.name} value={Number(value ?? 0)} step={input.type === 'int' ? 1 : 0.1} onCommit={(next) => setInput(input.name, input.type === 'int' ? Math.round(next) : next)} testId={`scene-modifier-${index}-input-${input.name}`} />
          case 'vector':
          case 'rotation':
            return <VectorRow key={input.name} label={input.name} value={(Array.isArray(value) ? value : [0, 0, 0]) as Vec3} step={0.1} onCommit={(next) => setInput(input.name, next)} testId={`scene-modifier-${index}-input-${input.name}`} />
          case 'bool':
            return <Checkbox key={input.name} label={input.name} checked={value === true} disabled={readOnly} onChange={(next) => setInput(input.name, next)} />
          case 'object':
            return (
              <Row key={input.name} label={input.name}>
                <Choice
                  label={input.name}
                  quiet
                  value={typeof value === 'number' ? String(value) : ''}
                  choices={[['', t('noneChoice')] as const, ...info.objects.filter((other) => other.id !== object.id).map((other) => [String(other.id), other.name] as const)]}
                  disabled={readOnly}
                  onChange={(next) => setInput(input.name, next === '' ? null : Number(next))}
                />
              </Row>
            )
          default:
            return null
        }
      })}
    </div>
  )
}
