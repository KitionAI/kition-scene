import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react'
import { ImageIcon } from '@kition/tessera/icons/react'
import {
  Button,
  Checkbox,
  Choice,
  Dialog,
  EditorFrame,
  FrameChrome,
  EngineToolRail,
  IconButton,
  KitEditorProvider,
  Segmented,
  commandForShortcut,
  commandMenuTabs,
  createKitStrings,
  toast,
  useEngineTools,
  useKitEditorState,
  useKitSession,
  type FrameTab,
  type HostConnection,
  type KitCanvasHandle,
  type KitCommand,
  type Phrases,
} from '@kition/sdk/editor'
import { engineManifest } from './engine'
import { CATALOGS } from './strings'
import { DEFAULT_TAB, HOST_FILE_COMMANDS, MENU_ORDER, SCENE_PRIMARY, VIEW_MENUS, describe } from './scene'
import { PeersProvider, useScenePresence } from './Presence'
import { SceneViewport } from './Viewport'
import { SidePanel } from './SidePanel'
import { Timeline } from './Timeline'
import { GraphEditor } from './GraphEditor'
import { NlaEditor } from './NlaEditor'
import { NodeEditor } from './NodeEditor'
import { animationOf } from './animation'
import { ProportionalControl } from './Proportional'
import { WeightPaintToggle } from './WeightPaint'
import { SculptToggle } from './Sculpt'
import { RenderDialog } from './RenderDialog'
import { UvEditor } from './UvEditor'
import { useSceneViewState, type SceneViewState } from './viewState'
import { TRANSFORM_MODES, toolGesture } from './gesture'
import { modalTransform, type ModalKind, type ModalOperation } from './modal'
import { PointerMenu, pointerPoint, type PointerMenuRow, type PointerMenuState } from './PointerMenu'
import { toggleAdjustLast } from './AdjustLast'
import { ModalKeys, useModalActive } from './ModalUi'
import { RENAME_EVENT } from './SidePanel'
import { focusEditor } from './NumberBox'
import { viewMenuRows, viewNavigator } from './navigator'
import { orientationMenuRows, pivotMenuRows, snapMenuRows } from './TransformControls'

const EXPORT_FORMATS = ['glb', 'gltf', 'obj', 'stl', 'fbx', 'usda', 'usdc', 'usdz', 'png'] as const
type ExportFormat = (typeof EXPORT_FORMATS)[number]
const IMPORT_FORMATS = ['glb', 'gltf', 'obj', 'stl', 'fbx', 'usd', 'usda', 'usdc', 'usdz', 'blend']

function decodeBase64(text: string) {
  const binary = atob(text)
  const bytes = new Uint8Array(binary.length)
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index)
  return bytes
}

function encodeBase64(bytes: Uint8Array) {
  let binary = ''
  for (let index = 0; index < bytes.length; index += 0x8000) binary += String.fromCharCode(...bytes.subarray(index, index + 0x8000))
  return btoa(binary)
}

/** The keys a transform under way takes, as `modal.key` names them, by the key's code. */
const MODAL_KEYS: Record<string, string> = {
  KeyX: 'x', KeyY: 'y', KeyZ: 'z', KeyC: 'c', KeyG: 'g', KeyR: 'r', KeyS: 's',
  Period: '.', NumpadDecimal: '.', Minus: '-', NumpadSubtract: '-', Slash: '/', NumpadDivide: '/',
  Backspace: 'Backspace', Tab: 'Tab',
}

/** What G, R and S start. */
const MODAL_START: Record<string, ModalKind> = { KeyG: 'move', KeyR: 'rotate', KeyS: 'scale' }

/** Menu rows that start a modal instead of running once with fixed values (Blender's menus do the same). */
const MENU_MODALS: Record<string, ModalKind | ModalOperation> = {
  'mesh.extrude': 'extrude',
  'mesh.inset': 'inset',
  'mesh.bevel': 'bevel',
}

/**
 * Menu rows whose commands need values the frame would ask for in a form of raw parameters: the
 * editor puts its own rows at the top of their menus instead, which start the modal (or the
 * rename box), by menu.
 */
const FORM_ROWS: Record<string, Array<{ id: string; label: string; shortcut: string; modal?: ModalKind | ModalOperation }>> = {
  Object: [
    { id: 'object.translate', label: 'Move', shortcut: 'G', modal: 'move' },
    { id: 'object.rotate', label: 'Rotate', shortcut: 'R', modal: 'rotate' },
    { id: 'object.scale', label: 'Scale', shortcut: 'S', modal: 'scale' },
    { id: 'object.rename', label: 'Rename', shortcut: 'F2' },
  ],
  Mesh: [
    { id: 'mesh.loopCut', label: 'Loop Cut', shortcut: 'Ctrl+R', modal: 'loopCut' },
    { id: 'mesh.translate', label: 'Move', shortcut: 'G', modal: 'move' },
    { id: 'mesh.rotate', label: 'Rotate', shortcut: 'R', modal: 'rotate' },
    { id: 'mesh.scale', label: 'Scale', shortcut: 'S', modal: 'scale' },
  ],
  Curve: [{ id: 'curve.translate', label: 'Move', shortcut: 'G', modal: 'move' }],
}
const FORM_ROW_IDS = Object.values(FORM_ROWS).flatMap((rows) => rows.map((row) => row.id))

/** The key `modal.key` takes for `event`, or null. */
function modalKey(event: Pick<KeyboardEvent, 'code'>) {
  const digit = /^(?:Digit|Numpad)([0-9])$/.exec(event.code)
  return digit ? digit[1] : (MODAL_KEYS[event.code] ?? null)
}

/** A shortcut without a modifier (Blender's single keys: X, A, E, Tab), which the frame leaves to the kit. */
function plainShortcut(shortcut: string | null | undefined) {
  return !!shortcut && !shortcut.includes('+') && !/^(Delete|Backspace|PageUp|PageDown|F[0-9]{1,2})$/.test(shortcut)
}

/** What the view's keys read of a key event. */
type NavKey = { code: string; key: string; metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; target: EventTarget | null; preventDefault: () => void; stopPropagation: () => void }

/** A key event's keys, for handing on with a modifier left out. */
function pick(event: Pick<NavKey, 'code' | 'key' | 'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey' | 'target'>) {
  return { code: event.code, key: event.key, metaKey: event.metaKey, ctrlKey: event.ctrlKey, altKey: event.altKey, shiftKey: event.shiftKey, target: event.target }
}

/** The commands that add an object: the editor puts what they add at the 3D cursor. */
const addsObject = (id: string) => id.startsWith('object.add')

function keyMatches(shortcut: string, event: Pick<KeyboardEvent, 'code' | 'key' | 'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey'>) {
  if (event.metaKey || event.ctrlKey || event.altKey || event.shiftKey) return false
  if (/^[A-Za-z]$/.test(shortcut)) return event.code === `Key${shortcut.toUpperCase()}`
  // The number row only: the numeric keypad is Blender's views, not the edit mode's select modes.
  if (/^[0-9]$/.test(shortcut)) return event.code === `Digit${shortcut}`
  // The keypad's point frames the selection; the main keyboard's opens the pivot point menu.
  if (shortcut === '.') return event.code === 'NumpadDecimal'
  return event.key === shortcut
}

export function App({ host, phrase }: { host: HostConnection; phrase: Phrases }) {
  const t = useMemo(() => createKitStrings(CATALOGS, host.init.locale), [host])
  const manifest = useMemo(engineManifest, [])
  // The person's view state: kept with the host after each save, never in the file.
  const viewState = useRef<SceneViewState | null>(null)
  const sessionOptions = useMemo(() => ({ viewState: () => (viewState.current ? { state: viewState.current } : undefined) }), [])
  const session = useKitSession(host, manifest, describe, sessionOptions)
  useSceneViewState(host, session, viewState)
  // A command refused because there is nothing for it to act on (Show Hidden with nothing hidden,
  // Delete with nothing selected) says so in the status bar, not as an error card.
  const [note, setNote] = useState<{ text: string; serial: number } | null>(null)
  const editorOptions = useMemo(
    () => ({
      report: (title: string, error: unknown) => {
        const message = error instanceof Error ? error.message : String(error)
        const nothing = /^not-allowed: (nothing is hidden|select an object first)/.exec(message)
        if (nothing) {
          setNote({ text: t(nothing[1] === 'nothing is hidden' ? 'nothingHidden' : 'noSelection'), serial: Date.now() })
          return
        }
        toast('error', title, message)
      },
    }),
    [t],
  )
  const editor = useKitEditorState(session, host, t, 'scene', phrase, undefined, editorOptions)
  const { doc, change, state } = session
  const presence = useScenePresence(host, editor, state)
  // A key pressed while the tools were busy (a pointer event in flight) comes back here once they
  // are done: it runs the command or picks the tool it would have, so no key is lost.
  const deferredKey = useRef<(key: string, shift: boolean) => void>(() => {})
  const tools = useEngineTools(session.tool, doc != null, change, { initial: 'select', onDeferredKey: (key, shift) => deferredKey.current(key, shift) })
  const canvas = useRef<KitCanvasHandle>(null)
  // A text's edit mode takes typed text: the tools answer whether they want it once the mode
  // changes, however it changed (Tab, the panel, an agent).
  const shapeMode = state?.mode === 'textEdit' || state?.mode === 'curveEdit' ? state.mode : null
  const refreshTools = tools.refresh
  useEffect(() => {
    if (doc) refreshTools()
  }, [shapeMode, doc, refreshTools])
  const { commands } = editor
  const [rendering, setRendering] = useState(false)
  const [uvOpen, setUvOpen] = useState(false)
  const [exporting, setExporting] = useState<{ format: ExportFormat; selectedOnly: boolean } | null>(null)
  const latest = useRef({ editor, host })
  latest.current = { editor, host }
  const base = (host.init.document.path.split('/').pop() ?? 'Scene').replace(/\.kscene$/i, '')

  // The development page's flows set the person's selection, view, mode and frame through this:
  // an agent's ai/run keeps an editor state of its own (C3b), so it cannot set the person's.
  useEffect(() => {
    if (!import.meta.env.DEV) return
    const page = window as Window & { kitScene?: { run: (steps: Array<{ command: string; params?: Record<string, unknown> }>) => Promise<void>; state: () => unknown; engine: () => unknown } }
    page.kitScene = {
      run: async (steps) => {
        for (const step of steps) await latest.current.editor.run(step.command, step.params ?? {})
      },
      // The scene as the person's editor shows it now (a number field's preview included).
      state: () => latest.current.editor.session.state,
      // The engine itself, for measuring what its calls cost.
      engine: () => latest.current.editor.session.doc,
    }
    return () => {
      delete page.kitScene
    }
  }, [])

  const renderImage = useCallback(() => setRendering(true), [])

  const importFile = useCallback(async () => {
    const { editor: current, host: connection } = latest.current
    try {
      const path = await connection.pick('import', IMPORT_FORMATS)
      if (!path) return
      const { bytes } = await connection.files.read(path)
      const format = path.split('.').pop()?.toLowerCase() ?? ''
      await current.run('file.import', { base64: encodeBase64(new Uint8Array(bytes)), format })
    } catch (error) {
      current.report(t('importFailed'), error)
    }
  }, [t])

  const runExport = useCallback(async (format: ExportFormat, selectedOnly: boolean) => {
    const { editor: current, host: connection } = latest.current
    try {
      const answer = await current.session.doc?.execute('file.export', { format, selectedOnly })
      const parsed = answer ? (JSON.parse(answer) as { base64: string }) : null
      if (!parsed) throw new Error('nothing exported')
      const written = await connection.exportFile(`${base}.${format}`, decodeBase64(parsed.base64))
      toast('success', t('exported', { name: written.split('/').pop() ?? written }))
    } catch (error) {
      current.report(t('exportFailed'), error)
    }
  }, [base, t])

  /** Menu rows the editor runs itself: they answer files and pictures, not changes. */
  const onCommand = useCallback(
    (id: string) => {
      const modal = MENU_MODALS[id]
      if (modal) {
        modalTransform.begin(modal)
        return
      }
      if (id === 'render.image' || id === 'render.trace.start') void renderImage()
      else if (id === 'file.export') setExporting({ format: 'glb', selectedOnly: false })
      else if (id === 'file.import') void importFile()
      else if (addsObject(id) && latest.current.editor.session.state?.mode === 'object') {
        // A new object goes where the 3D cursor is (world aligned), as in Blender.
        void latest.current.editor.run(id, { location: latest.current.editor.session.state.transform.cursor })
      } else void latest.current.editor.run(id)
    },
    [importFile, renderImage],
  )

  // Export needs the kit's format dialog and the host's destination picker.
  const tabs = useMemo<FrameTab[]>(() => commandMenuTabs(commands, {
    order: MENU_ORDER,
    exclude: [...HOST_FILE_COMMANDS, 'file.export', ...FORM_ROW_IDS],
    before: {
      File: [{ label: t('exportTitle'), testId: 'scene-cmd-file.export', onSelect: () => onCommand('file.export') }],
      ...Object.fromEntries(
        Object.entries(FORM_ROWS).map(([menu, rows]) => [
          menu,
          rows.map((row) => ({
            label: `${phrase(row.label)} (${row.shortcut})`,
            testId: `scene-cmd-${row.id}`,
            onSelect: () => (row.modal ? modalTransform.begin(row.modal) : latestRename.current()),
          })),
        ]),
      ),
    },
    phrase,
    view: VIEW_MENUS,
    primary: SCENE_PRIMARY,
    onCommand,
  }), [commands, onCommand, phrase, t])

  // Single keys (X, A, E, Tab, 1, 3, .) the frame binds only with a modifier.
  const plain = useMemo(() => commands.filter((command: KitCommand) => plainShortcut(command.shortcut)), [commands])
  const runPlain = (event: Pick<KeyboardEvent, 'code' | 'key' | 'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey'>) => {
    // A key several commands share (I: inset in edit mode, insert keyframe in object mode) runs the one that can.
    const matching = plain.filter((item) => keyMatches(item.shortcut!, event))
    const command = matching.find((item) => editor.enabled(item.id))
    if (command) {
      void editor.run(command.id, {})
      return true
    }
    // The editor's list of what can run may lag a change made a moment ago (a mode switch, a
    // selection): ask the engine itself before letting the key go.
    const doc = editor.session.doc
    if (!matching.length || !doc) return false
    void doc.disabledCommands().then((disabled) => {
      const ready = matching.find((item) => !disabled.includes(item.id))
      if (ready) void latest.current.editor.run(ready.id, {})
    })
    return true
  }
  deferredKey.current = (key, shift) => {
    // A key held back while a pointer event was on its way, and the gesture still under way.
    if (toolGesture.held) return
    const wanted = (shift ? `Shift+${key}` : key).toLowerCase()
    const matching = commands.filter((item: KitCommand) => item.shortcut?.toLowerCase() === wanted)
    const command = matching.find((item: KitCommand) => editor.enabled(item.id))
    if (command) {
      void editor.run(command.id, {})
      return
    }
    // As for keys pressed now: the engine says which can run when the editor's list lags.
    const doc = editor.session.doc
    if (matching.length && doc) {
      void doc.disabledCommands().then((disabled) => {
        const ready = matching.find((item: KitCommand) => !disabled.includes(item.id))
        if (ready) void latest.current.editor.run(ready.id, {})
      })
      return
    }
    const tool = tools.toolForKey(key, shift)
    if (tool) tools.select(tool)
  }
  /** A key one of the editor's single-key shortcuts or Delete answers, enabled or not. */
  const editorKey = (event: Pick<KeyboardEvent, 'code' | 'key' | 'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey'>) => {
    const plainKey = !event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey
    return (plainKey && (event.key === 'Delete' || event.key === 'Backspace')) || plain.some((item) => keyMatches(item.shortcut!, event))
  }
  /**
   * A Ctrl/Cmd shortcut whose command the editor's list says cannot run: that list follows a
   * change a moment later (one more call to the engine), and the frame drops such a key, so a
   * Ctrl+Z right after the first change (a menu's row) undid nothing. The engine itself says
   * whether it can run. Answers whether the key was taken.
   */
  const laggingShortcut = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!(event.metaKey || event.ctrlKey)) return false
    const command = commandForShortcut(commands, event)
    const doc = editor.session.doc
    if (!command || !doc || editor.enabled(command.id)) return false
    void doc.disabledCommands().then((disabled) => {
      if (!disabled.includes(command.id)) void latest.current.editor.run(command.id, {})
    })
    return true
  }
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement || event.target instanceof HTMLSelectElement) return false
    if (laggingShortcut(event)) return true
    // Delete and Backspace delete the selection from a panel too (an outliner row keeps the focus
    // after a click), as in Blender; on the viewport the active tool takes them first.
    const plainKey = !event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey
    const deletes = plainKey && (event.key === 'Delete' || event.key === 'Backspace') && editor.enabled('edit.delete')
    if (deletes) void editor.run('edit.delete', {})
    else if (!runPlain(event)) return false
    return true
  }
  // A key pressed while nothing in the editor has the focus (it fell to the page): the editor's
  // single-key shortcuts still answer, and the editor takes the focus.
  const latestPlain = useRef(runPlain)
  latestPlain.current = runPlain
  useEffect(() => {
    const keyDown = (event: globalThis.KeyboardEvent) => {
      if (event.target !== document.body && event.target !== document.documentElement) return
      if (toolGesture.held || modalTransform.active) return
      if (latestNavigation.current(event)) {
        document.querySelector<HTMLElement>('.kui-app')?.focus({ preventScroll: true })
        return
      }
      if (MODAL_START[event.code] || event.code === 'KeyD') {
        latestStart.current(event)
        document.querySelector<HTMLElement>('.kui-app')?.focus({ preventScroll: true })
        return
      }
      if (!latestPlain.current(event)) return
      event.preventDefault()
      document.querySelector<HTMLElement>('.kui-app')?.focus({ preventScroll: true })
    }
    window.addEventListener('keydown', keyDown)
    return () => window.removeEventListener('keydown', keyDown)
  }, [])
  // F12 renders (the engine's command answers the picture).
  const frameKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    // A menu at the pointer takes the keys (the arrows, Enter, Escape).
    if (popup) return
    // A transform under way (or a transform tool's press) takes the keys: Escape cancels it,
    // Enter confirms it, axis letters, digits and the rest go to it, and no shortcut runs.
    if (modalTransform.active || (toolGesture.held && toolGesture.transform)) {
      modalKeyDown(event)
      return
    }
    // While the left button holds a tool's gesture on the viewport (a box), keys run no
    // shortcut: X there deleted the object being moved. Escape still cancels the gesture.
    if (toolGesture.held && !['Escape', 'Shift', 'Control', 'Alt', 'Meta'].includes(event.key)) {
      event.preventDefault()
      event.stopPropagation()
      return
    }
    const typing = event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement || event.target instanceof HTMLSelectElement
    if (!typing && navigationKey(event)) return
    if (!typing && transformKey(event)) return
    if (!typing && startKey(event)) return
    if (!typing && editKey(event)) return
    if (event.key === 'F2' && !event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey && !typing) {
      event.preventDefault()
      event.stopPropagation()
      renameActive()
      return
    }
    if (event.key === 'F12' && !event.metaKey && !event.ctrlKey) {
      event.preventDefault()
      event.stopPropagation()
      if (editor.enabled('render.image')) void renderImage()
      return
    }
    // A key for the editor pressed on a panel's button (an outliner row keeps the focus after a
    // click) gives the editor the keyboard first, whoever runs it: the engine's tools hold a key
    // while a pointer event is under way and run it afterwards. The button may go away with the
    // mode (disabled in edit mode) or with what it showed (a deleted object's row), and the focus
    // would fall to the page, where Cmd+S and Cmd+Z reach nothing.
    const target = event.target
    if (target instanceof HTMLButtonElement && event.key !== ' ' && event.key !== 'Enter' && editorKey(event)) {
      event.currentTarget.querySelector<HTMLElement>('.kui-app')?.focus({ preventScroll: true })
    }
    // A checkbox or radio keeps the focus after a click, and the frame takes any input for a
    // text field; single-key shortcuts still answer there, and the editor takes the focus back.
    if (target instanceof HTMLInputElement && (target.type === 'checkbox' || target.type === 'radio') && runPlain(event)) {
      event.preventDefault()
      event.stopPropagation()
      event.currentTarget.querySelector<HTMLElement>('.kui-app')?.focus({ preventScroll: true })
    }
  }

  /** A key during a transform. */
  const modalKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const step = event.ctrlKey || event.metaKey
    if (['Shift', 'Control', 'Meta'].includes(event.key)) {
      modalTransform.modifiers(event.shiftKey, step)
      return
    }
    // Alt lets the view's own keys and buttons through (Blender's Navigate): Alt with the keypad,
    // Home or `.` navigates.
    if (event.key === 'Alt') return
    if (event.altKey && !step && !event.shiftKey && navigationKey({ ...pick(event), altKey: /^Digit/.test(event.code), preventDefault: () => event.preventDefault(), stopPropagation: () => event.stopPropagation() })) return
    // Cmd/Ctrl shortcuts (Undo, Save) still run: Undo ends the transform first, then undoes.
    if (step && event.code !== 'Tab') return
    event.preventDefault()
    event.stopPropagation()
    if (event.key === 'Escape') modalTransform.cancel()
    else if (event.key === 'Enter') modalTransform.confirm()
    else {
      const key = event.altKey || (step && event.code !== 'Tab') ? null : modalKey(event)
      if (key) modalTransform.key(key, event.shiftKey, step)
    }
  }
  const frameKeyUp = (event: KeyboardEvent<HTMLDivElement>) => {
    if (modalTransform.active && ['Shift', 'Control', 'Meta'].includes(event.key)) modalTransform.modifiers(event.shiftKey, event.ctrlKey || event.metaKey)
  }
  /**
   * The keys that start a transform where the pointer is: G, R and S (the tool they pick is
   * still picked), Shift+D (copies, then moves them); and Alt+G, Alt+R, Alt+S clear the
   * location, rotation and scale. Answers whether the key was taken whole.
   */
  const startKey = (event: Pick<KeyboardEvent, 'code' | 'key' | 'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey' | 'preventDefault' | 'stopPropagation'>) => {
    const mode = state?.mode ?? 'object'
    if (host.readOnly || !TRANSFORM_MODES.includes(mode) || event.metaKey || event.ctrlKey) return false
    const kind = MODAL_START[event.code]
    if (kind && !event.altKey && !event.shiftKey) {
      modalTransform.begin(kind)
      return false
    }
    if (event.code === 'KeyD' && event.shiftKey && !event.altKey && (mode === 'object' || mode === 'edit')) {
      event.preventDefault()
      event.stopPropagation()
      modalTransform.begin('move', { duplicate: true })
      return true
    }
    if (kind && event.altKey && !event.shiftKey && mode === 'object' && editor.enabled('object.transform') && (state?.selected.length ?? 0) > 0) {
      event.preventDefault()
      event.stopPropagation()
      const cleared = kind === 'move' ? { location: [0, 0, 0] } : kind === 'rotate' ? { rotation: [0, 0, 0] } : { scale: [1, 1, 1] }
      void editor.run('object.transform', cleared)
      return true
    }
    return false
  }
  const latestStart = useRef(startKey)
  latestStart.current = startKey

  /**
   * The view's keys (the keypad, Alt+1 / 3 / 7, Home, `.`, Ctrl/Cmd+= and −) in every mode, and
   * `` ` ``, which opens the View menu at the pointer. Not in the node editor, which has its own.
   */
  const navigationKey = (event: NavKey) => {
    if (event.target instanceof Element && event.target.closest('.scene-nodes')) return false
    const plainKey = !event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey
    if (plainKey && event.code === 'Backquote') {
      event.preventDefault()
      event.stopPropagation()
      openMenu(t('menuView'), 'scene-view-menu', viewMenuRows(t))
      return true
    }
    if (!viewNavigator.key(event)) return false
    event.preventDefault()
    event.stopPropagation()
    return true
  }
  const latestNavigation = useRef(navigationKey)
  latestNavigation.current = navigationKey

  /**
   * The 3D cursor's and the transform settings' keys: `.` the pivot point menu, `,` the
   * orientation menu, Shift+Tab snapping on or off, Shift+S the Snap menu (the cursor to the
   * origin, the selection or the active object; the selection to the cursor), Shift+C the cursor
   * back at the origin and everything framed. Answers whether the key was taken.
   */
  const transformKey = (event: Pick<KeyboardEvent, 'code' | 'key' | 'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey' | 'preventDefault' | 'stopPropagation' | 'target'>) => {
    if (!state || event.metaKey || event.ctrlKey || event.altKey) return false
    if (event.target instanceof Element && event.target.closest('.scene-nodes')) return false
    const take = () => {
      event.preventDefault()
      event.stopPropagation()
      return true
    }
    const run = (id: string, params: Record<string, unknown>) => editor.run(id, params)
    const transforms = TRANSFORM_MODES.includes(state.mode)
    if (!event.shiftKey && event.code === 'Period' && transforms) {
      openMenu(t('pivotPoint'), 'scene-pivot-menu', pivotMenuRows(state, t, run))
      return take()
    }
    if (!event.shiftKey && event.code === 'Comma' && transforms) {
      openMenu(t('orientation'), 'scene-orientation-menu', orientationMenuRows(state, t, run))
      return take()
    }
    if (event.shiftKey && event.code === 'Tab') {
      void run('transform.set', { snap: !state.transform.snap })
      return take()
    }
    if (event.shiftKey && event.code === 'KeyS' && transforms) {
      openMenu(t('menuSnap'), 'scene-snap-menu', snapMenuRows(state, t, run))
      return take()
    }
    if (event.shiftKey && event.code === 'KeyC') {
      void run('cursor.reset', {}).then(() => viewNavigator.hooks?.command('view.frameAll', true))
      return take()
    }
    return false
  }

  // A menu at the pointer (X, Delete, M, P in a mesh's edit mode).
  const [popup, setPopup] = useState<PointerMenuState | null>(null)
  const openMenu = (title: string, testId: string, rows: PointerMenuRow[]) => {
    const viewport = document.querySelector('[data-testid="scene-viewport"]')
    setPopup({ at: pointerPoint(viewport), title, testId, rows })
  }
  // Every row is offered: the editor's list of what can run may lag a selection made a moment
  // ago, and a command that cannot run says why.
  const commandRow = (id: string, label: string): PointerMenuRow => ({ id, label, onSelect: () => void editor.run(id, {}).finally(() => focusEditor()) })
  /**
   * Edit mode's keys: E extrudes and moves, I insets, Ctrl/Cmd+B bevels, Ctrl/Cmd+R loop cuts (all
   * sized by the pointer, as modals); X and Delete open the Delete menu, M the Merge menu, P the
   * Separate menu. F9 opens Adjust Last Operation's fields in any mode. Ctrl/Cmd+R never reloads
   * the page. Answers whether the key was taken.
   */
  const editKey = (event: Pick<KeyboardEvent, 'code' | 'key' | 'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey' | 'preventDefault' | 'stopPropagation'>) => {
    const take = () => {
      event.preventDefault()
      event.stopPropagation()
      return true
    }
    const command = event.metaKey || event.ctrlKey
    if (event.key === 'F9' && !command && !event.altKey && !event.shiftKey) {
      toggleAdjustLast()
      return take()
    }
    const edit = state?.mode === 'edit' && !host.readOnly
    if (event.code === 'KeyR' && command && !event.altKey && !event.shiftKey) {
      if (edit) modalTransform.begin('loopCut')
      return take()
    }
    if (!edit || event.altKey) return false
    if (command && !event.shiftKey && event.code === 'KeyB') {
      modalTransform.begin('bevel')
      return take()
    }
    if (command || event.shiftKey) return false
    switch (event.code) {
      case 'KeyE':
        modalTransform.begin('extrude')
        return take()
      case 'KeyI':
        modalTransform.begin('inset')
        return take()
      case 'KeyX':
      case 'Delete':
        openMenu(t('menuDelete'), 'scene-delete-menu', [
          commandRow('mesh.deleteVertices', t('deleteVertices')),
          commandRow('mesh.deleteFaces', t('deleteFaces')),
          'separator',
          commandRow('mesh.dissolveVertices', t('dissolveVertices')),
          commandRow('mesh.dissolveEdges', t('dissolveEdges')),
          commandRow('mesh.dissolveFaces', t('dissolveFaces')),
        ])
        return take()
      case 'KeyM':
        openMenu(t('menuMerge'), 'scene-merge-menu', [commandRow('mesh.merge', t('mergeAtCenter'))])
        return take()
      case 'KeyP':
        openMenu(t('menuSeparate'), 'scene-separate-menu', [commandRow('mesh.separate', t('separateSelection'))])
        return take()
      default:
        return false
    }
  }
  const modalActive = useModalActive()

  // The note stays a few seconds, or until the next change.
  useEffect(() => {
    if (!note) return undefined
    const timer = window.setTimeout(() => setNote(null), 4000)
    return () => window.clearTimeout(timer)
  }, [note])
  const noteRevision = useRef(change)
  useEffect(() => {
    if (noteRevision.current === change) return
    noteRevision.current = change
    setNote((current) => (current && Date.now() - current.serial > 300 ? null : current))
  }, [change])

  // The page opens with the keyboard for the editor: its shortcuts answer before any click.
  const focusedDoc = useRef<typeof doc>(null)
  useEffect(() => {
    if (!doc || focusedDoc.current === doc) return
    focusedDoc.current = doc
    if (document.activeElement === document.body || document.activeElement == null) focusEditor()
  }, [doc])

  // F2 renames the active object: in its outliner row, or, with no row shown, in a small box.
  const [renaming, setRenaming] = useState<{ id: number; name: string } | null>(null)
  const latestRename = useRef<() => void>(() => {})
  const renameActive = () => {
    const active = state?.objects.find((object) => object.id === state.active)
    if (!active || host.readOnly) return
    const row = document.querySelector<HTMLElement>(`[data-testid="scene-outliner-item"][data-id="${active.id}"]`)
    if (row && row.offsetParent != null) {
      row.scrollIntoView({ block: 'nearest' })
      window.dispatchEvent(new CustomEvent(RENAME_EVENT, { detail: active.id }))
    } else {
      setRenaming({ id: active.id, name: active.name })
    }
  }
  latestRename.current = renameActive

  const [graph, setGraph] = useState(false)
  const [nlaOpen, setNlaOpen] = useState(false)
  const [nodes, setNodes] = useState(false)
  const mode = state?.mode ?? 'object'
  const status = state ? (
    <span className="scene-status" data-testid="scene-status">
      <span className="scene-mode" data-mode={mode} data-testid="scene-mode">
        {t(mode === 'edit' ? 'editMode' : mode === 'armatureEdit' ? 'armatureEditMode' : mode === 'pose' ? 'poseMode' : mode === 'weightPaint' ? 'weightPaintMode' : mode === 'sculpt' ? 'sculptMode' : mode === 'particleEdit' ? 'particleEditMode' : mode === 'curveEdit' ? 'curveEditMode' : mode === 'textEdit' ? 'textEditMode' : mode === 'latticeEdit' ? 'latticeEditMode' : 'objectMode')}
      </span>
      <span className="tabular-nums" data-testid="scene-counts">
        {mode === 'edit'
          ? t('editCount', { vertices: state.vertices.length, faces: state.faces.length })
          : mode === 'armatureEdit' || mode === 'pose'
            ? t('bonesSelected', { count: state.bones.length })
            : mode === 'weightPaint'
              ? t('paintingGroup', { group: state.paint?.group ?? t('newGroup') })
            : mode === 'sculpt'
              ? t('maskedCount', { count: state.sculpt?.masked ?? 0 })
            : mode === 'particleEdit'
              ? t('groomingWith', { brush: t(`groom_${state.particleEdit?.brush.kind ?? 'comb'}`) })
            : mode === 'curveEdit' || mode === 'latticeEdit'
              ? t('pointsSelected', { count: state.points.length })
            : mode === 'textEdit'
              ? t('caretAt', { caret: state.caret })
            : t('objectsSelected', { selected: state.selected.length, count: state.objects.length })}
      </span>
      {modalActive ? <ModalKeys t={t} /> : tools.status ? <span>{phrase(tools.status)}</span> : null}
      {note ? (
        <span className="scene-status-note" role="status" data-testid="scene-status-note">
          {note.text}
        </span>
      ) : null}
    </span>
  ) : null
  const statusEnd = state ? (
    <span className="scene-status-end">
      {mode === 'edit' ? (
        <Segmented
          label={t('selectMode')}
          value={state.select}
          segments={[
            { value: 'vertex', label: t('vertexMode') },
            { value: 'edge', label: t('edgeMode') },
            { value: 'face', label: t('faceMode') },
          ]}
          onChange={(value) => void editor.run('mesh.selectMode', { mode: value })}
        />
      ) : null}
      {mode === 'edit' ? <ProportionalControl info={state} /> : null}
      <WeightPaintToggle info={state} />
      <SculptToggle info={state} />
      {mode === 'edit' ? <Checkbox label={t('uvEditor')} checked={uvOpen} onChange={setUvOpen} testId="scene-uv-toggle" /> : null}
      <Segmented
        label={t('shadingMode')}
        value={state.shading}
        segments={[
          { value: 'wireframe', label: t('wireframe') },
          { value: 'solid', label: t('solid') },
          { value: 'material', label: t('materialPreview') },
          { value: 'rendered', label: t('rendered') },
        ]}
        onChange={(value) => void editor.run('view.shading', { mode: value })}
      />
      <Checkbox label={t('grid')} checked={state.grid} onChange={(on) => void editor.run('view.grid', { on })} testId="scene-grid" />
      <Checkbox label={t('graphEditor')} checked={graph} onChange={setGraph} testId="scene-graph-toggle" />
      <Checkbox label={t('nlaEditor')} checked={nlaOpen} onChange={setNlaOpen} testId="scene-nla-toggle" />
      <Checkbox label={t('nodeEditor')} checked={nodes} onChange={setNodes} testId="scene-nodes-toggle" />
    </span>
  ) : null

  return (
    <KitEditorProvider editor={editor} tools={tools}>
      <div className="scene-editor-root" onKeyDownCapture={frameKeyDown} onKeyUpCapture={frameKeyUp}>
        <EditorFrame
          tabs={tabs}
          defaultTab={DEFAULT_TAB}
          canvas={canvas}
          zoom={null}
          onKeyDown={onKeyDown}
          actions={<IconButton icon={ImageIcon} label={`${phrase('Render Image')} (F12)`} disabled={!state || state.camera == null} onClick={() => void renderImage()} data-testid="scene-render" />}
          status={status}
          statusEnd={statusEnd}
          rail={<EngineToolRail />}
          panels={state ? (
            <PeersProvider peers={presence.peers}>
              <SidePanel info={state} />
            </PeersProvider>
          ) : null}
        >
          <div className="scene-stage">
            <div className="scene-split">
              <SceneViewport canvas={canvas} info={state} onRender={() => void renderImage()} presence={presence} t={t} readOnly={host.readOnly} />
              {state && uvOpen && mode === 'edit' ? (
                <FrameChrome>
                  <UvEditor info={state} onClose={() => setUvOpen(false)} />
                </FrameChrome>
              ) : null}
            </div>
            {/* The kit's own editors and the timeline: Focus shows the viewport alone. */}
            {state ? (
              <FrameChrome>
                {graph ? <GraphEditor info={state} engine={doc?.info} /> : null}
                {nlaOpen ? <NlaEditor info={state} animation={animationOf(doc?.info)} /> : null}
                {nodes ? <NodeEditor info={state} /> : null}
                <Timeline animation={animationOf(doc?.info)} hasSelection={state.selected.length > 0} info={state} />
              </FrameChrome>
            ) : null}
          </div>
        </EditorFrame>
        <PointerMenu
          menu={popup}
          onClose={() => {
            setPopup(null)
            // After the menu has given the focus back where it was (it may be gone by then).
            window.requestAnimationFrame(() => focusEditor())
          }}
        />
        {rendering ? <RenderDialog
            host={host}
            base={base}
            aspect={state ? `${state.render.width} / ${state.render.height}` : undefined}
            compositing={state && state.render.compositor != null ? state.render.compositing : null}
            onClose={() => setRendering(false)}
          /> : null}
        {renaming ? (
          <Dialog
            title={t('renameTitle')}
            testId="scene-rename-dialog"
            onClose={() => {
              setRenaming(null)
              focusEditor()
            }}
            actions={
              <>
                <Button
                  variant="primary"
                  onClick={() => {
                    const name = renaming.name.trim()
                    setRenaming(null)
                    if (name) void editor.run('object.rename', { object: renaming.id, name })
                    focusEditor()
                  }}
                  data-testid="scene-rename-run"
                >
                  {t('rename')}
                </Button>
                <Button
                  onClick={() => {
                    setRenaming(null)
                    focusEditor()
                  }}
                >
                  {t('close')}
                </Button>
              </>
            }
          >
            <input
              className="kui-field scene-rename"
              aria-label={t('name')}
              value={renaming.name}
              autoFocus
              onFocus={(event) => event.currentTarget.select()}
              onChange={(event) => setRenaming({ ...renaming, name: event.target.value })}
              onKeyDown={(event) => {
                if (event.key !== 'Enter') return
                event.preventDefault()
                const name = renaming.name.trim()
                setRenaming(null)
                if (name) void editor.run('object.rename', { object: renaming.id, name })
                focusEditor()
              }}
              data-testid="scene-rename-field"
            />
          </Dialog>
        ) : null}
        {exporting ? (
          <Dialog
            title={t('exportTitle')}
            testId="scene-export-dialog"
            onClose={() => setExporting(null)}
            actions={
              <>
                <Button
                  variant="primary"
                  onClick={() => {
                    const { format, selectedOnly } = exporting
                    setExporting(null)
                    void runExport(format, selectedOnly)
                  }}
                  data-testid="scene-export-run"
                >
                  {t('exportRun')}
                </Button>
                <Button onClick={() => setExporting(null)}>{t('close')}</Button>
              </>
            }
          >
            <div className="scene-export">
              <Choice
                label={t('exportFormat')}
                value={exporting.format}
                choices={EXPORT_FORMATS.map((format) => [format, t(`format${format[0].toUpperCase()}${format.slice(1)}`)] as const)}
                onChange={(format) => setExporting({ ...exporting, format })}
                testId="scene-export-format"
              />
              <Checkbox label={t('exportSelected')} checked={exporting.selectedOnly} disabled={exporting.format === 'png'} onChange={(selectedOnly) => setExporting({ ...exporting, selectedOnly })} />
            </div>
          </Dialog>
        ) : null}
      </div>
    </KitEditorProvider>
  )
}

