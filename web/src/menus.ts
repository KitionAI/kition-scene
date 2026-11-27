/**
 * The editor's own menus, by mode: the viewport header's menus, the Add menu (Shift+A), the
 * right-click menus and the mode list (the header's drop-down, Ctrl+Tab). Each is data: rows that
 * name an engine command (with fixed parameters, or none), an editor action, or a submenu; the
 * builder drops rows whose command the engine does not have and the separators they leave.
 * Labels are English and go through the editor's phrases.
 */
import type { KitCommand } from '@kition/sdk/editor'
import type { PointerMenuRow } from './PointerMenu'
import type { SceneInfo } from './scene'

export type SceneMode = SceneInfo['mode']

/** A row of one of the editor's menus. */
export type MenuSpec =
  | { command: string; label?: string; params?: Record<string, unknown>; shortcut?: string; id?: string }
  | { action: string; label: string; shortcut?: string; checked?: boolean }
  | { submenu: string; label: string; items: MenuSpec[] }
  | '|'

/** What the builder needs: the engine's commands, the phrases, how a row runs. */
export type MenuContext = {
  commands: ReadonlyMap<string, KitCommand>
  phrase: (english: string) => string
  runCommand: (id: string, params?: Record<string, unknown>) => void
  action: (name: string) => void
  /** Rows the editor's list says cannot run now (header menus); absent: every row is offered. */
  disabled?: (id: string) => boolean
}

const MODIFIER_KINDS: Array<[string, string]> = [
  ['mirror', 'Mirror'], ['array', 'Array'], ['subdivision', 'Subdivision'], ['solidify', 'Solidify'], ['bevel', 'Bevel'],
  ['boolean', 'Boolean'], ['screw', 'Screw'], ['triangulate', 'Triangulate'], ['weld', 'Weld'], ['smooth', 'Smooth'],
  ['decimate', 'Decimate'], ['wireframe', 'Wireframe'], ['displace', 'Displace'], ['simpleDeform', 'SimpleDeform'], ['cast', 'Cast'],
  ['shrinkwrap', 'Shrinkwrap'], ['lattice', 'Lattice'], ['multires', 'Multires'], ['geometryNodes', 'Geometry Nodes'],
]
const CONSTRAINT_KINDS: Array<[string, string]> = [
  ['copyLocation', 'Copy Location'], ['copyRotation', 'Copy Rotation'], ['limitRotation', 'Limit Rotation'], ['transform', 'Transformation'],
  ['childOf', 'Child Of'], ['floor', 'Floor'], ['action', 'Action'], ['armature', 'Armature'],
  ['trackTo', 'Track To'], ['dampedTrack', 'Damped Track'], ['stretchTo', 'Stretch To'],
]
const INTERPOLATIONS: Array<[string, string]> = [['bezier', 'Bezier'], ['linear', 'Linear'], ['constant', 'Constant']]
const HANDLE_TYPES: Array<[string, string]> = [['auto', 'Automatic'], ['vector', 'Vector'], ['aligned', 'Aligned'], ['free', 'Free']]

/** A command run with one parameter set to each of `kinds`: the form a kind chose before. */
const kinds = (command: string, key: string, list: Array<[string, string]>): MenuSpec[] =>
  list.map(([value, label]) => ({ command, label, params: { [key]: value }, id: `${command}-${value}` }))

/** Commands that ask for a kind first, as submenus of the kinds (the frame's form asked for it raw). */
export const KIND_SUBMENUS: Record<string, { label: string; items: MenuSpec[] }> = {
  'modifier.add': { label: 'Add Modifier', items: kinds('modifier.add', 'type', MODIFIER_KINDS) },
  'constraint.add': { label: 'Add Constraint', items: kinds('constraint.add', 'type', CONSTRAINT_KINDS) },
  'animation.setInterpolation': { label: 'Interpolation Mode', items: kinds('animation.setInterpolation', 'interpolation', INTERPOLATIONS) },
  'curve.setHandleType': { label: 'Set Handle Type', items: kinds('curve.setHandleType', 'type', HANDLE_TYPES) },
}
const kindMenu = (command: string): MenuSpec => ({ submenu: command, ...KIND_SUBMENUS[command] })

/** The Add menu of object mode (Shift+A, the header's Add): what the engine can add, in Blender's order. */
export const ADD_MENU: MenuSpec[] = [
  {
    submenu: 'mesh',
    label: 'Mesh',
    items: [
      { command: 'object.addPlane' }, { command: 'object.addCube' }, { command: 'object.addCircle' }, { command: 'object.addUvSphere' },
      { command: 'object.addIcoSphere' }, { command: 'object.addCylinder' }, { command: 'object.addCone' }, { command: 'object.addTorus' },
      '|', { command: 'object.addGrid' }, { command: 'object.addBackdrop' },
    ],
  },
  { submenu: 'curve', label: 'Curve', items: [{ command: 'object.addCurveBezier' }, { command: 'object.addCurveCircle' }, '|', { command: 'object.addCurvePath' }] },
  { command: 'object.addText' },
  '|',
  { command: 'object.addArmature' },
  { command: 'object.addLattice' },
  '|',
  { command: 'object.addEmpty' },
  '|',
  {
    submenu: 'light',
    label: 'Light',
    items: [
      { command: 'object.addLight', label: 'Point', params: { kind: 'point' }, id: 'object.addLight-point' },
      { command: 'object.addLight', label: 'Sun', params: { kind: 'sun' }, id: 'object.addLight-sun' },
      { command: 'object.addLight', label: 'Spot', params: { kind: 'spot' }, id: 'object.addLight-spot' },
      { command: 'object.addLight', label: 'Area', params: { kind: 'area' }, id: 'object.addLight-area' },
    ],
  },
  '|',
  { command: 'object.addCamera' },
]

/** The Snap submenu: the rows of Shift+S's menu. */
const SNAP: MenuSpec = {
  submenu: 'snap',
  label: 'Snap',
  items: [
    { action: 'selectionToCursor', label: 'Selection to Cursor' },
    '|',
    { action: 'cursorOrigin', label: 'Cursor to World Origin' },
    { action: 'cursorSelected', label: 'Cursor to Selected' },
    { action: 'cursorActive', label: 'Cursor to Active' },
  ],
}

const VIEW_MENU: MenuSpec[] = [
  { action: 'toggleToolbar', label: 'Toolbar', shortcut: 'T' },
  { action: 'toggleSidebar', label: 'Sidebar', shortcut: 'N' },
  { action: 'adjustLast', label: 'Adjust Last Operation', shortcut: 'F9' },
  '|',
  { command: 'view.frameSelected', label: 'Frame Selected', shortcut: 'Numpad .' },
  { command: 'view.frameAll', label: 'Frame All', shortcut: 'Home' },
  { command: 'view.perspective', label: 'Perspective/Orthographic', shortcut: 'Numpad 5' },
  '|',
  { submenu: 'cameras', label: 'Cameras', items: [{ command: 'object.setCamera', label: 'Set Active Object as Camera' }, { command: 'view.camera', label: 'Active Camera', shortcut: 'Numpad 0' }] },
  {
    submenu: 'viewpoint',
    label: 'Viewpoint',
    items: [
      { command: 'view.camera', label: 'Camera', shortcut: 'Numpad 0' },
      '|', { command: 'view.top', label: 'Top', shortcut: 'Numpad 7' }, { command: 'view.bottom', label: 'Bottom', shortcut: 'Ctrl+Numpad 7' },
      '|', { command: 'view.front', label: 'Front', shortcut: 'Numpad 1' }, { command: 'view.back', label: 'Back', shortcut: 'Ctrl+Numpad 1' },
      '|', { command: 'view.right', label: 'Right', shortcut: 'Numpad 3' }, { command: 'view.left', label: 'Left', shortcut: 'Ctrl+Numpad 3' },
    ],
  },
  {
    submenu: 'align',
    label: 'Align View',
    items: [{ action: 'alignCamera', label: 'Align Active Camera to View', shortcut: 'Ctrl+Alt+Numpad 0' }, '|', { action: 'centerCursor', label: 'Center Cursor and Frame All', shortcut: 'Shift+C' }],
  },
  '|',
  { action: 'search', label: 'Menu Search', shortcut: 'F3' },
]

const SELECT_MENU: MenuSpec[] = [
  { command: 'edit.selectAll', label: 'All', params: { action: 'select' }, shortcut: 'A', id: 'select-all' },
  { command: 'edit.selectAll', label: 'None', params: { action: 'deselect' }, shortcut: 'Alt+A', id: 'select-none' },
  { command: 'edit.selectAll', label: 'Invert', params: { action: 'invert' }, shortcut: 'Ctrl+I', id: 'select-invert' },
]

/** The region selects of object and edit mode (B, C; the lasso is Ctrl+right drag). */
const REGION_ROWS: MenuSpec[] = [
  { action: 'selectBox', label: 'Box Select', shortcut: 'B' },
  { action: 'selectCircle', label: 'Circle Select', shortcut: 'C' },
]

/** Object mode's Select menu. */
const OBJECT_SELECT_MENU: MenuSpec[] = [...SELECT_MENU, '|', ...REGION_ROWS]

/** Edit mode's Select menu: with more and less, linked parts and loops. */
const EDIT_SELECT_MENU: MenuSpec[] = [
  ...SELECT_MENU,
  '|',
  ...REGION_ROWS,
  '|',
  {
    submenu: 'moreLess',
    label: 'Select More/Less',
    items: [
      { command: 'select.more', label: 'More', shortcut: 'Ctrl+Numpad +' },
      { command: 'select.less', label: 'Less', shortcut: 'Ctrl+Numpad −' },
    ],
  },
  { submenu: 'linked', label: 'Select Linked', items: [{ command: 'select.linked', label: 'Linked', shortcut: 'Ctrl+L' }] },
  {
    submenu: 'loops',
    label: 'Select Loops',
    items: [
      { command: 'select.loop', label: 'Edge Loops', shortcut: 'Alt+Click' },
      { command: 'select.ring', label: 'Edge Rings', shortcut: 'Ctrl+Alt+Click' },
    ],
  },
]

const OBJECT_MENU: MenuSpec[] = [
  { submenu: 'transform', label: 'Transform', items: [{ command: 'object.translate', label: 'Move', shortcut: 'G' }, { command: 'object.rotate', label: 'Rotate', shortcut: 'R' }, { command: 'object.scale', label: 'Scale', shortcut: 'S' }] },
  { submenu: 'origin', label: 'Set Origin', items: [{ command: 'object.originToGeometry' }] },
  {
    submenu: 'clear',
    label: 'Clear',
    items: [
      { action: 'clearLocation', label: 'Location', shortcut: 'Alt+G' },
      { action: 'clearRotation', label: 'Rotation', shortcut: 'Alt+R' },
      { action: 'clearScale', label: 'Scale', shortcut: 'Alt+S' },
    ],
  },
  { submenu: 'apply', label: 'Apply', items: [{ command: 'object.applyTransform', label: 'All Transforms', shortcut: 'Ctrl+A' }] },
  SNAP,
  '|',
  { action: 'duplicate', label: 'Duplicate Objects', shortcut: 'Shift+D' },
  { command: 'object.join', shortcut: 'Ctrl+J' },
  '|',
  { submenu: 'parent', label: 'Parent', items: [{ action: 'parentMenu', label: 'Object', shortcut: 'Ctrl+P' }, { command: 'object.parentArmature' }, '|', { command: 'object.clearParent', shortcut: 'Alt+P' }] },
  { submenu: 'modifiers', label: 'Modifiers', items: [kindMenu('modifier.add')] },
  { submenu: 'constraints', label: 'Constraints', items: [kindMenu('constraint.add')] },
  { submenu: 'link', label: 'Link/Transfer Data', items: [{ action: 'linkMaterials', label: 'Link Materials', shortcut: 'Ctrl+L' }] },
  '|',
  { command: 'object.shadeSmooth' },
  { command: 'object.shadeSmoothByAngle', label: 'Shade Auto Smooth' },
  { command: 'object.shadeFlat' },
  '|',
  {
    submenu: 'animation',
    label: 'Animation',
    items: [{ command: 'animation.insertKey', shortcut: 'I' }, { command: 'animation.deleteKey', shortcut: 'Alt+I' }, { command: 'animation.clear', shortcut: 'Shift+Alt+I' }, '|', kindMenu('animation.setInterpolation')],
  },
  '|',
  { submenu: 'convert', label: 'Convert', items: [{ command: 'object.convertToMesh', label: 'Mesh' }] },
  '|',
  {
    submenu: 'showHide',
    label: 'Show/Hide',
    items: [
      { command: 'object.reveal', shortcut: 'Alt+H' },
      { command: 'object.hide', label: 'Hide Selected', shortcut: 'H' },
      { action: 'hideUnselected', label: 'Hide Unselected', shortcut: 'Shift+H' },
    ],
  },
  '|',
  { action: 'rename', label: 'Rename Active Object', shortcut: 'F2' },
  { command: 'edit.delete', shortcut: 'X' },
]

const MESH_MENU: MenuSpec[] = [
  { submenu: 'transform', label: 'Transform', items: [{ command: 'mesh.translate', label: 'Move', shortcut: 'G' }, { command: 'mesh.rotate', label: 'Rotate', shortcut: 'R' }, { command: 'mesh.scale', label: 'Scale', shortcut: 'S' }] },
  { submenu: 'mirror', label: 'Mirror', items: [{ command: 'mesh.symmetrize' }] },
  SNAP,
  '|',
  { action: 'duplicate', label: 'Duplicate', shortcut: 'Shift+D' },
  { submenu: 'extrude', label: 'Extrude', items: [{ command: 'mesh.extrude', label: 'Extrude Faces', shortcut: 'E' }, { command: 'mesh.extrudeIndividual' }] },
  '|',
  { submenu: 'merge', label: 'Merge', items: [{ command: 'mesh.merge', label: 'At Center' }] },
  { submenu: 'separate', label: 'Separate', items: [{ command: 'mesh.separate', label: 'Selection' }] },
  '|',
  { command: 'mesh.bisect' },
  { command: 'mesh.knife', label: 'Knife Topology Tool', shortcut: 'K' },
  '|',
  { submenu: 'normals', label: 'Normals', items: [{ command: 'mesh.flip' }, { command: 'mesh.recalculateNormals', shortcut: 'Shift+N' }] },
  { command: 'mesh.proportional', shortcut: 'O' },
  '|',
  {
    submenu: 'delete',
    label: 'Delete',
    items: [{ command: 'mesh.deleteVertices', label: 'Vertices' }, { command: 'mesh.deleteFaces', label: 'Faces' }, '|', { command: 'mesh.dissolveVertices' }, { command: 'mesh.dissolveEdges' }, { command: 'mesh.dissolveFaces' }],
  },
]

const VERTEX_MENU: MenuSpec[] = [
  { command: 'mesh.extrude', label: 'Extrude Vertices', shortcut: 'E', id: 'mesh.extrude-vertices' },
  { command: 'mesh.bevelVertices', shortcut: 'Ctrl+Shift+B' },
  '|',
  { command: 'mesh.fill', label: 'New Edge/Face from Vertices', shortcut: 'F' },
  '|',
  { command: 'mesh.merge', label: 'Merge Vertices', shortcut: 'M', id: 'mesh.merge-vertices' },
  { command: 'mesh.separate', label: 'Separate', shortcut: 'P', id: 'mesh.separate-vertices' },
  { command: 'mesh.dissolveVertices' },
  { command: 'mesh.deleteVertices' },
]

const EDGE_MENU: MenuSpec[] = [
  { command: 'mesh.extrude', label: 'Extrude Edges', shortcut: 'E', id: 'mesh.extrude-edges' },
  { command: 'mesh.bevel', label: 'Bevel Edges', shortcut: 'Ctrl+B' },
  { command: 'mesh.bridge' },
  '|',
  { command: 'mesh.subdivide' },
  '|',
  { action: 'edgeSlide', label: 'Edge Slide', shortcut: 'Shift+G' },
  { command: 'mesh.loopCut', label: 'Loop Cut and Slide', shortcut: 'Ctrl+R' },
  '|',
  { command: 'mesh.markSeam' },
  { command: 'mesh.clearSeam' },
  '|',
  { command: 'mesh.dissolveEdges' },
]

const FACE_MENU: MenuSpec[] = [
  { command: 'mesh.extrude', label: 'Extrude Faces', shortcut: 'E', id: 'mesh.extrude-faces' },
  { command: 'mesh.extrudeIndividual', label: 'Extrude Individual Faces' },
  '|',
  { command: 'mesh.inset', label: 'Inset Faces', shortcut: 'I' },
  '|',
  { command: 'mesh.fill', label: 'Fill', shortcut: 'F', id: 'mesh.fill-faces' },
  { command: 'mesh.spin' },
  '|',
  { command: 'mesh.dissolveFaces' },
  { command: 'mesh.deleteFaces' },
]

const UV_MENU: MenuSpec[] = [
  { command: 'uv.unwrap', shortcut: 'U' },
  { command: 'uv.smartProject' },
  '|',
  { command: 'uv.cubeProject' },
  { command: 'uv.cylinderProject' },
  { command: 'uv.sphereProject' },
  { command: 'uv.planarProject' },
  '|',
  { command: 'uv.reset' },
  { command: 'uv.clear' },
  '|',
  { command: 'mesh.markSeam' },
  { command: 'mesh.clearSeam' },
]

const SCULPT_MENU: MenuSpec[] = [{ command: 'sculpt.remesh' }, { command: 'sculpt.detailFlood' }, '|', { command: 'sculpt.showAllFaces', shortcut: 'Alt+H' }]
const MASK_MENU: MenuSpec[] = [{ command: 'sculpt.maskInvert' }, { command: 'sculpt.maskFill' }, { command: 'sculpt.maskClear' }, '|', { command: 'sculpt.maskSmooth' }]
const FACE_SETS_MENU: MenuSpec[] = [
  { command: 'sculpt.faceSetsFromMask' }, { command: 'sculpt.faceSetsFromVisible' }, { command: 'sculpt.faceSetsFromLooseParts' }, '|',
  { command: 'sculpt.hideFaceSet', shortcut: 'H' }, { command: 'sculpt.isolateFaceSet', shortcut: 'Shift+H' }, { command: 'sculpt.showAllFaces', shortcut: 'Alt+H' }, '|',
  { command: 'sculpt.faceSetsClear' },
]
const POSE_MENU: MenuSpec[] = [{ command: 'pose.clear', label: 'Clear User Transforms' }, '|', { command: 'pose.insertKey', shortcut: 'I' }, { command: 'pose.deleteKey', shortcut: 'Alt+I' }]
const ARMATURE_MENU: MenuSpec[] = [{ command: 'bone.extrude', shortcut: 'E' }, { command: 'bone.add' }]
const CURVE_MENU: MenuSpec[] = [
  { submenu: 'transform', label: 'Transform', items: [{ command: 'curve.translate', label: 'Move', shortcut: 'G' }] },
  SNAP,
  '|',
  { command: 'curve.extrude', shortcut: 'E' },
  { command: 'curve.subdivide' },
  '|',
  { command: 'curve.toggleCyclic', shortcut: 'Alt+C' },
  kindMenu('curve.setHandleType'),
  '|',
  { command: 'curve.deletePoints', label: 'Delete Point' },
]
const TEXT_MENU: MenuSpec[] = [{ command: 'text.delete' }]

/** One of the header's menus: its name (the test id's) and rows. */
export type HeaderMenu = { name: string; label: string; items: MenuSpec[] }

/** The header's menus in `mode`, in Blender's order, those the engine has rows for. */
export function headerMenus(mode: SceneMode): HeaderMenu[] {
  const view: HeaderMenu = { name: 'View', label: 'View', items: VIEW_MENU }
  const select: HeaderMenu = { name: 'Select', label: 'Select', items: mode === 'edit' ? EDIT_SELECT_MENU : mode === 'object' ? OBJECT_SELECT_MENU : SELECT_MENU }
  const add: HeaderMenu = { name: 'Add', label: 'Add', items: ADD_MENU }
  switch (mode) {
    case 'edit':
      return [view, select, { name: 'Mesh', label: 'Mesh', items: MESH_MENU }, { name: 'Vertex', label: 'Vertex', items: VERTEX_MENU }, { name: 'Edge', label: 'Edge', items: EDGE_MENU }, { name: 'Face', label: 'Face', items: FACE_MENU }, { name: 'UV', label: 'UV', items: UV_MENU }]
    case 'sculpt':
      return [view, { name: 'Sculpt', label: 'Sculpt', items: SCULPT_MENU }, { name: 'Mask', label: 'Mask', items: MASK_MENU }, { name: 'FaceSets', label: 'Face Sets', items: FACE_SETS_MENU }]
    case 'weightPaint':
      return [view, { name: 'Weights', label: 'Weights', items: [{ command: 'paint.normalizeAll' }] }]
    case 'pose':
      return [view, select, { name: 'Pose', label: 'Pose', items: POSE_MENU }]
    case 'armatureEdit':
      return [view, select, { name: 'Armature', label: 'Armature', items: ARMATURE_MENU }]
    case 'curveEdit':
      return [view, select, { name: 'Curve', label: 'Curve', items: CURVE_MENU }]
    case 'textEdit':
      return [view, { name: 'Text', label: 'Text', items: TEXT_MENU }]
    case 'latticeEdit':
    case 'particleEdit':
      return [view, select]
    default:
      return [view, select, add, { name: 'Object', label: 'Object', items: OBJECT_MENU }]
  }
}

/**
 * The right-click menu in `info`'s mode: a title and rows (the object menu with something
 * selected, the empty space's menu with nothing, edit mode's by select mode, pose mode's, and the
 * edit modes' and paint modes' own).
 */
export function contextMenu(info: SceneInfo): { title: string; kind: string; items: MenuSpec[] } {
  switch (info.mode) {
    case 'edit': {
      const kind = info.select
      const title = kind === 'vertex' ? 'Vertex' : kind === 'edge' ? 'Edge' : 'Face'
      const common: MenuSpec[] = ['|', SNAP, '|', { command: 'mesh.merge', label: 'Merge at Center', id: 'mesh.merge-context' }, { command: 'mesh.separate', label: 'Separate Selection', id: 'mesh.separate-context' }]
      const own: MenuSpec[] =
        kind === 'vertex'
          ? [{ command: 'mesh.subdivide' }, '|', { command: 'mesh.extrude', label: 'Extrude Vertices', shortcut: 'E' }, { command: 'mesh.bevelVertices', shortcut: 'Ctrl+Shift+B' }, '|', { command: 'mesh.fill', label: 'New Edge/Face from Vertices', shortcut: 'F' }, ...common, { command: 'mesh.dissolveVertices' }, { command: 'mesh.deleteVertices' }]
          : kind === 'edge'
            ? [{ command: 'mesh.subdivide' }, '|', { command: 'mesh.extrude', label: 'Extrude Edges', shortcut: 'E' }, { command: 'mesh.bevel', label: 'Bevel Edges', shortcut: 'Ctrl+B' }, { command: 'mesh.bridge' }, '|', { command: 'mesh.loopCut', label: 'Loop Cut and Slide', shortcut: 'Ctrl+R' }, { action: 'edgeSlide', label: 'Edge Slide', shortcut: 'Shift+G' }, '|', { command: 'mesh.markSeam' }, { command: 'mesh.clearSeam' }, ...common, { command: 'mesh.dissolveEdges' }]
            : [{ command: 'mesh.extrude', label: 'Extrude Faces', shortcut: 'E' }, { command: 'mesh.extrudeIndividual', label: 'Extrude Individual Faces' }, { command: 'mesh.inset', label: 'Inset Faces', shortcut: 'I' }, '|', { command: 'mesh.subdivide' }, { command: 'mesh.fill', label: 'Fill', shortcut: 'F' }, '|', { command: 'mesh.flip' }, ...common, { command: 'mesh.dissolveFaces' }, { command: 'mesh.deleteFaces' }]
      return { title, kind: `edit-${kind}`, items: own }
    }
    case 'pose':
      return { title: 'Pose', kind: 'pose', items: [{ command: 'pose.insertKey', shortcut: 'I' }, { command: 'pose.deleteKey', shortcut: 'Alt+I' }, '|', { command: 'pose.clear', label: 'Clear User Transforms' }] }
    case 'curveEdit':
      return { title: 'Curve', kind: 'curve', items: [{ command: 'curve.subdivide' }, { command: 'curve.extrude', label: 'Extrude Curve and Move', shortcut: 'E' }, '|', SNAP, '|', kindMenu('curve.setHandleType'), { command: 'curve.toggleCyclic', shortcut: 'Alt+C' }, '|', { command: 'curve.deletePoints', label: 'Delete Point' }] }
    case 'armatureEdit':
      return { title: 'Armature', kind: 'armature', items: [{ command: 'bone.extrude', shortcut: 'E' }, { command: 'bone.add' }] }
    case 'sculpt':
      return { title: 'Sculpt', kind: 'sculpt', items: [...MASK_MENU, '|', { command: 'sculpt.showAllFaces', shortcut: 'Alt+H' }] }
    case 'weightPaint':
      return { title: 'Weights', kind: 'weights', items: [{ command: 'paint.normalizeAll' }] }
    case 'object':
      if (!info.selected.length) {
        return {
          title: 'Scene',
          kind: 'empty',
          items: [
            { submenu: 'add', label: 'Add', items: ADD_MENU },
            '|',
            { command: 'edit.selectAll', label: 'Select All', params: { action: 'select' }, shortcut: 'A' },
            { command: 'object.reveal', shortcut: 'Alt+H' },
            '|',
            { action: 'cursorOrigin', label: 'Cursor to World Origin' },
            { command: 'view.frameAll', shortcut: 'Home' },
            '|',
            { action: 'render', label: 'Render Image', shortcut: 'F12' },
          ],
        }
      }
      return {
        title: 'Object',
        kind: 'object',
        items: [
          { command: 'object.shadeSmooth' },
          { command: 'object.shadeSmoothByAngle', label: 'Shade Auto Smooth' },
          { command: 'object.shadeFlat' },
          '|',
          { submenu: 'convert', label: 'Convert To', items: [{ command: 'object.convertToMesh', label: 'Mesh' }] },
          { submenu: 'origin', label: 'Set Origin', items: [{ command: 'object.originToGeometry' }] },
          '|',
          { action: 'duplicate', label: 'Duplicate Objects', shortcut: 'Shift+D' },
          '|',
          { action: 'rename', label: 'Rename Active Object', shortcut: 'F2' },
          '|',
          SNAP,
          { submenu: 'parent', label: 'Parent', items: [{ action: 'parentMenu', label: 'Object', shortcut: 'Ctrl+P' }, '|', { command: 'object.clearParent', shortcut: 'Alt+P' }] },
          '|',
          { command: 'animation.insertKey', shortcut: 'I' },
          '|',
          { command: 'edit.delete', shortcut: 'X' },
        ],
      }
    default:
      return { title: 'View', kind: 'view', items: [{ command: 'view.frameSelected' }, { command: 'view.frameAll' }] }
  }
}

/** The modes the active object can be in, as the header's drop-down and Ctrl+Tab list them. */
export function modesFor(info: SceneInfo): SceneMode[] {
  const target = info.objects.find((object) => object.id === (info.editing ?? info.active))
  if (!target) return ['object']
  switch (target.type) {
    case 'mesh': {
      const hair = info.particles.some((system) => system.object === target.id && system.kind === 'hair')
      return hair ? ['object', 'edit', 'sculpt', 'weightPaint', 'particleEdit'] : ['object', 'edit', 'sculpt', 'weightPaint']
    }
    case 'armature':
      return ['object', 'armatureEdit', 'pose']
    case 'curve':
      return ['object', 'curveEdit']
    case 'text':
      return ['object', 'textEdit']
    case 'lattice':
      return ['object', 'latticeEdit']
    default:
      return ['object']
  }
}

/** The strings key naming a mode. */
export const MODE_LABELS: Record<SceneMode, string> = {
  object: 'objectMode',
  edit: 'editMode',
  sculpt: 'sculptMode',
  weightPaint: 'weightPaintMode',
  particleEdit: 'particleEditMode',
  armatureEdit: 'armatureEditMode',
  pose: 'poseMode',
  curveEdit: 'curveEditMode',
  textEdit: 'textEditMode',
  latticeEdit: 'latticeEditMode',
}

const LEAVE: Record<SceneMode, [string, Record<string, unknown>] | null> = {
  object: null,
  edit: ['mode.set', { mode: 'object' }],
  sculpt: ['mode.sculpt', { on: false }],
  weightPaint: ['mode.weightPaint', { on: false }],
  particleEdit: ['mode.particleEdit', { on: false }],
  armatureEdit: ['mode.armature', {}],
  pose: ['mode.pose', { on: false }],
  curveEdit: ['mode.shape', {}],
  textEdit: ['mode.shape', {}],
  latticeEdit: ['mode.shape', {}],
}
const ENTER: Record<SceneMode, [string, Record<string, unknown>] | null> = {
  object: null,
  edit: ['mode.set', { mode: 'edit' }],
  sculpt: ['mode.sculpt', { on: true }],
  weightPaint: ['mode.weightPaint', { on: true }],
  particleEdit: ['mode.particleEdit', { on: true }],
  armatureEdit: ['mode.armature', {}],
  pose: ['mode.pose', { on: true }],
  curveEdit: ['mode.shape', {}],
  textEdit: ['mode.shape', {}],
  latticeEdit: ['mode.shape', {}],
}

/** The mode commands that take the editor from `from` to `to`: out to object mode first, then in. */
export function modeSteps(from: SceneMode, to: SceneMode): Array<[string, Record<string, unknown>]> {
  if (from === to) return []
  const steps: Array<[string, Record<string, unknown>]> = []
  const leave = LEAVE[from]
  if (leave) steps.push(leave)
  const enter = ENTER[to]
  if (enter) steps.push(enter)
  return steps
}

/** The frame's top-level menus each mode uses; others are hidden in it (Blender's header does the same). */
const MODE_MENUS: Record<string, readonly SceneMode[]> = {
  Object: ['object'],
  Mesh: ['edit'],
  Curve: ['curveEdit'],
  Text: ['textEdit'],
  Armature: ['object', 'armatureEdit', 'pose'],
  Pose: ['pose'],
  Weights: ['weightPaint'],
  Sculpt: ['sculpt'],
}

/** Whether the frame shows its menu `id` in `mode` (an armature's menu in object mode only with one active). */
export function frameMenuShown(id: string, info: SceneInfo | null) {
  const modes = MODE_MENUS[id]
  if (!modes || !info) return true
  if (!modes.includes(info.mode)) return false
  if (id === 'Armature' && info.mode === 'object') return info.objects.find((object) => object.id === info.active)?.type === 'armature'
  return true
}

/** `specs` as rows of a menu: commands the engine lacks dropped, with the separators they leave. */
export function buildRows(specs: readonly MenuSpec[], context: MenuContext, prefix = ''): PointerMenuRow[] {
  const rows: PointerMenuRow[] = []
  for (const spec of specs) {
    if (spec === '|') {
      if (rows.length && rows[rows.length - 1] !== 'separator') rows.push('separator')
      continue
    }
    if ('submenu' in spec) {
      const children = buildRows(spec.items, context, `${prefix}${spec.submenu}-`)
      if (children.length) rows.push({ id: `${prefix}${spec.submenu}`, label: context.phrase(spec.label), onSelect: () => undefined, children })
      continue
    }
    if ('action' in spec) {
      rows.push({ id: spec.action, label: context.phrase(spec.label), shortcut: spec.shortcut, checked: spec.checked, onSelect: () => context.action(spec.action) })
      continue
    }
    const command = context.commands.get(spec.command)
    if (!command) continue
    const id = spec.id ?? spec.command
    rows.push({
      id,
      label: context.phrase(spec.label ?? command.title),
      shortcut: spec.shortcut ?? command.shortcut ?? undefined,
      disabled: context.disabled?.(spec.command) ?? false,
      onSelect: () => context.runCommand(spec.command, spec.params),
    })
  }
  while (rows.length && rows[rows.length - 1] === 'separator') rows.pop()
  return rows
}
