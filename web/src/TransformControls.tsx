import { Choice, IconButton, useKitEditor } from '@kition/sdk/editor'
import { MagnetIcon } from '@kition/tessera/icons/react'
import { ORIENTATIONS, PIVOT_POINTS, SNAP_TARGETS, type PivotPoint, type SceneInfo, type SnapTarget, type TransformOrientation } from './scene'
import type { PointerMenuRow } from './PointerMenu'
import { focusEditor } from './NumberBox'

type Strings = (key: string) => string

/** The person's transform settings, set as view state (no undo step); the keys go back to the editor. */
const setTransform = (run: (id: string, params: Record<string, unknown>) => Promise<unknown>, params: Record<string, unknown>) => {
  void run('transform.set', params)
  focusEditor()
}

/** The pivot point: a drop-down (`.` opens the same list at the pointer). */
export function PivotControl({ info }: { info: SceneInfo }) {
  const editor = useKitEditor()
  const { t } = editor
  return (
    <Choice
      label={t('pivotPoint')}
      quiet
      value={info.transform.pivot}
      choices={PIVOT_POINTS.map((pivot) => [pivot, t(`pivot_${pivot}`)] as const)}
      onChange={(pivot: PivotPoint) => setTransform(editor.run, { pivot })}
      testId="scene-pivot"
    />
  )
}

/** The transform orientation: a drop-down (`,` opens the same list at the pointer). */
export function OrientationControl({ info }: { info: SceneInfo }) {
  const editor = useKitEditor()
  const { t } = editor
  return (
    <Choice
      label={t('orientation')}
      quiet
      value={info.transform.orientation}
      choices={ORIENTATIONS.map((orientation) => [orientation, t(`orientation_${orientation}`)] as const)}
      onChange={(orientation: TransformOrientation) => setTransform(editor.run, { orientation })}
      testId="scene-orientation"
    />
  )
}

/** Snapping: the magnet turns it on or off (Shift+Tab), the drop-down says what to. */
export function SnapControl({ info }: { info: SceneInfo }) {
  const editor = useKitEditor()
  const { t } = editor
  return (
    <span className="scene-snap">
      <IconButton
        icon={MagnetIcon}
        label={`${t('snap')} (Shift+Tab)`}
        pressed={info.transform.snap}
        onClick={() => setTransform(editor.run, { snap: !info.transform.snap })}
        data-testid="scene-snap"
      />
      <Choice
        label={t('snapTarget')}
        quiet
        value={info.transform.snapTarget}
        choices={SNAP_TARGETS.map((target) => [target, t(`snap_${target}`)] as const)}
        onChange={(snapTarget: SnapTarget) => setTransform(editor.run, { snapTarget })}
        testId="scene-snap-target"
      />
    </span>
  )
}

/**
 * The orientation, pivot point and snapping controls at the viewport's top left, until the
 * viewport has its own header (the three controls go there as they are).
 */
export function TransformStrip({ info }: { info: SceneInfo }) {
  return (
    <div className="scene-transform-strip" data-testid="scene-transform-strip" onPointerDown={(event) => event.stopPropagation()}>
      <OrientationControl info={info} />
      <PivotControl info={info} />
      <SnapControl info={info} />
    </div>
  )
}

/** The pivot point menu at the pointer (`.`). */
export function pivotMenuRows(info: SceneInfo, t: Strings, run: (id: string, params: Record<string, unknown>) => Promise<unknown>): PointerMenuRow[] {
  return PIVOT_POINTS.map((pivot) => ({
    id: `pivot-${pivot}`,
    label: `${info.transform.pivot === pivot ? '• ' : ''}${t(`pivot_${pivot}`)}`,
    onSelect: () => setTransform(run, { pivot }),
  }))
}

/** The transform orientation menu at the pointer (`,`). */
export function orientationMenuRows(info: SceneInfo, t: Strings, run: (id: string, params: Record<string, unknown>) => Promise<unknown>): PointerMenuRow[] {
  return ORIENTATIONS.map((orientation) => ({
    id: `orientation-${orientation}`,
    label: `${info.transform.orientation === orientation ? '• ' : ''}${t(`orientation_${orientation}`)}`,
    onSelect: () => setTransform(run, { orientation }),
  }))
}

/**
 * The Snap menu at the pointer (Shift+S): the cursor to the world origin, to the selection, to the
 * active object; the selection to the cursor (one undo step: in object mode each object goes to the
 * cursor, or with a parent among them the selection's middle does; in edit mode the selected
 * vertices all go to it).
 */
export function snapMenuRows(info: SceneInfo, t: Strings, run: (id: string, params: Record<string, unknown>) => Promise<unknown>): PointerMenuRow[] {
  const rows: PointerMenuRow[] = [
    { id: 'cursor-origin', label: t('cursorToOrigin'), onSelect: () => void run('cursor.reset', {}) },
    { id: 'cursor-selected', label: t('cursorToSelected'), onSelect: () => void run('cursor.moveToSelected', {}) },
  ]
  if (info.mode === 'object' && info.active != null) rows.push({ id: 'cursor-active', label: t('cursorToActive'), onSelect: () => void run('cursor.moveToActive', {}) })
  const toCursor = selectionToCursor(info)
  if (toCursor) rows.push('separator', { id: 'selection-cursor', label: t('selectionToCursor'), onSelect: () => void run(toCursor.command, toCursor.params) })
  return rows
}

/** The command that moves the selection to the cursor now, or null (nothing selected, another mode). */
export function selectionToCursor(info: SceneInfo): { command: string; params: Record<string, unknown> } | null {
  const cursor = info.transform.cursor
  if (info.mode === 'object' && info.selected.length) {
    const chosen = info.objects.filter((object) => info.selected.includes(object.id))
    if (chosen.every((object) => object.parent == null)) return { command: 'object.transform', params: { objects: chosen.map((object) => object.id), location: cursor } }
    const median = info.transform.median
    if (!median) return null
    return { command: 'object.translate', params: { objects: chosen.map((object) => object.id), offset: [cursor[0] - median[0], cursor[1] - median[1], cursor[2] - median[2]] } }
  }
  if (info.mode === 'edit' && info.vertices.length && info.transform.cursorLocal) return { command: 'mesh.scale', params: { factor: 0, pivot: info.transform.cursorLocal } }
  return null
}
