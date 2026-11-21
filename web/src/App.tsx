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

/** A shortcut without a modifier (Blender's single keys: X, A, E, Tab), which the frame leaves to the kit. */
function plainShortcut(shortcut: string | null | undefined) {
  return !!shortcut && !shortcut.includes('+') && !/^(Delete|Backspace|PageUp|PageDown|F[0-9]{1,2})$/.test(shortcut)
}

function keyMatches(shortcut: string, event: Pick<KeyboardEvent, 'code' | 'key' | 'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey'>) {
  if (event.metaKey || event.ctrlKey || event.altKey || event.shiftKey) return false
  if (/^[A-Za-z]$/.test(shortcut)) return event.code === `Key${shortcut.toUpperCase()}`
  if (/^[0-9]$/.test(shortcut)) return event.code === `Digit${shortcut}` || event.code === `Numpad${shortcut}`
  if (shortcut === '.') return event.code === 'Period' || event.code === 'NumpadDecimal'
  return event.key === shortcut
}

export function App({ host, phrase }: { host: HostConnection; phrase: Phrases }) {
  const t = useMemo(() => createKitStrings(CATALOGS, host.init.locale), [host])
  const manifest = useMemo(engineManifest, [])
  const session = useKitSession(host, manifest, describe)
  const editor = useKitEditorState(session, host, t, 'scene', phrase)
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
    const page = window as Window & { kitScene?: { run: (steps: Array<{ command: string; params?: Record<string, unknown> }>) => Promise<void> } }
    page.kitScene = {
      run: async (steps) => {
        for (const step of steps) await latest.current.editor.run(step.command, step.params ?? {})
      },
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
      toast('success', t('saved', { name: written.split('/').pop() ?? written }))
    } catch (error) {
      current.report(t('exportFailed'), error)
    }
  }, [base, t])

  /** Menu rows the editor runs itself: they answer files and pictures, not changes. */
  const onCommand = useCallback(
    (id: string) => {
      if (id === 'render.image' || id === 'render.trace.start') void renderImage()
      else if (id === 'file.export') setExporting({ format: 'glb', selectedOnly: false })
      else if (id === 'file.import') void importFile()
      else void latest.current.editor.run(id)
    },
    [importFile, renderImage],
  )

  // Export needs the kit's format dialog and the host's destination picker.
  const tabs = useMemo<FrameTab[]>(() => commandMenuTabs(commands, {
    order: MENU_ORDER,
    exclude: [...HOST_FILE_COMMANDS, 'file.export'],
    before: { File: [{ label: t('exportTitle'), testId: 'scene-cmd-file.export', onSelect: () => onCommand('file.export') }] },
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
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement || event.target instanceof HTMLSelectElement) return false
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
      if (!latestPlain.current(event)) return
      event.preventDefault()
      document.querySelector<HTMLElement>('.kui-app')?.focus({ preventScroll: true })
    }
    window.addEventListener('keydown', keyDown)
    return () => window.removeEventListener('keydown', keyDown)
  }, [])
  // F12 renders (the engine's command answers the picture).
  const frameKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
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
      {tools.status ? <span>{phrase(tools.status)}</span> : null}
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
      <div className="scene-editor-root" onKeyDownCapture={frameKeyDown}>
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
              <SceneViewport canvas={canvas} info={state} onRender={() => void renderImage()} presence={presence} />
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
        {rendering ? <RenderDialog
            host={host}
            base={base}
            aspect={state ? `${state.render.width} / ${state.render.height}` : undefined}
            compositing={state && state.render.compositor != null ? state.render.compositing : null}
            onClose={() => setRendering(false)}
          /> : null}
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

