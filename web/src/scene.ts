import type { KitEngineInfo } from '@kition/sdk/editor'
import { animationDataOf, nlaOf, type AnimationData, type ObjectNla } from './AnimationData'
import { physicsOf, type PhysicsInfo } from './PhysicsInfo'

export type Vec3 = [number, number, number]

export type Modifier = { name: string; enabled: boolean; type: string } & Record<string, unknown>

/** A constraint as inspect shows it: targets carry the object's name, angles are degrees. */
export type Constraint = {
  name: string
  type: 'trackTo' | 'dampedTrack' | 'copyLocation' | 'copyRotation' | 'limitRotation' | 'childOf' | 'ik' | 'stretchTo' | 'floor' | 'transform' | 'action' | 'armature' | 'splineIk'
  enabled: boolean
  influence: number
  target?: { object: number; name: string; bone: string | null }
  pole?: { object: number; name: string; bone: string | null }
} & Record<string, unknown>

/** The sculpt brush. */
export type SculptBrush = {
  kind: string
  radius: number
  strength: number
  falloff: string
  frontFacesOnly: boolean
  autoSmooth: number
  spacing: number
  subtract: boolean
  symmetry: boolean[]
  usePressure: boolean
}

/** Dynamic topology's settings in sculpt mode. */
export type Dyntopo = {
  enabled: boolean
  mode: 'relative' | 'constant' | 'brush'
  detailSize: number
  resolution: number
  percent: number
  refine: 'subdivideCollapse' | 'subdivide' | 'collapse'
}

/** Sculpt mode's state: the brush, the tool, dynamic topology, the level, face sets, hiding. */
export type SculptInfo = {
  brush: SculptBrush
  masked: number
  tool: string
  dyntopo: Dyntopo
  /** The multiresolution level sculpted (0: the mesh itself) and how many the mesh has. */
  level: number
  levels: number
  faceSets: number[]
  hiddenFaces: number
}

/** A particle system on a mesh object, as `inspect()` lists it. */
export type ParticleSystemInfo = {
  object: number
  index: number
  name: string
  kind: 'emitter' | 'hair'
  physics: 'newtonian' | 'boids'
  boids: BoidsInfo
  seed: number
  count: number
  frameStart: number
  frameEnd: number
  lifetime: number
  lifetimeRandom: number
  emitFrom: 'faces' | 'verts' | 'volume'
  distribution: 'jittered' | 'random'
  normalVelocity: number
  objectVelocity: Vec3
  randomVelocity: number
  gravity: number
  drag: number
  timestep: number
  colliders?: number[]
  /** The vertex group setting how densely particles start (none: everywhere alike). */
  densityGroup?: string
  bounce: number
  dieOnCollision: boolean
  renderAs: 'halo' | 'object' | 'none'
  instanceObject?: number
  size: number
  sizeRandom: number
  rotationRandom: number
  hairLength: number
  segments: number
  children: 'none' | 'simple' | 'interpolated'
  childCount: number
  childRadius: number
  strandWidth: number
  groomed: boolean
  /** Hair: simulated in the physics bake (absent: the strands hold their shape). */
  dynamics?: HairDynamicsInfo
}

export type BoidRule = { kind: 'goal' | 'avoid' | 'separate' | 'flock'; target?: number }
export type BoidsInfo = { airSpeedMax: number; airAccMax: number; airAngularMax: number; airPersonalSpace: number; rules: BoidRule[] }

/** Hair dynamics' settings. */
export type HairDynamicsInfo = {
  quality: number
  mass: number
  stiffness: number
  damping: number
  airDamping: number
  pinStiffness: number
  collision: boolean
  collisionDistance: number
}

/** Particle Edit's grooming brush. */
export type GroomBrush = { kind: 'comb' | 'cut' | 'length' | 'puff' | 'smooth'; radius: number; strength: number; subtract: boolean }

/** The weight paint brush. */
export type PaintBrush = { mode: 'draw' | 'add' | 'subtract' | 'blur'; weight: number; strength: number; radius: number; falloff: string; frontFacesOnly: boolean }
/** Weight paint's X mirror: on, by topology, and into the mirrored group names. */
export type PaintMirror = { x: boolean; topology: boolean; groups: boolean }
/** What the pointer does in weight paint. */
export type PaintTool = 'brush' | 'linearGradient' | 'radialGradient' | 'sampleWeight'
const PAINT_TOOLS: PaintTool[] = ['brush', 'linearGradient', 'radialGradient', 'sampleWeight']

export type SceneObject = {
  id: number
  name: string
  type: 'mesh' | 'light' | 'camera' | 'empty' | 'armature' | 'curve' | 'text' | 'lattice'
  location: Vec3
  rotation: Vec3
  scale: Vec3
  parent: number | null
  hidden: boolean
  material: number | null
  mesh: { vertices: number; edges: number; faces: number; smooth: boolean; smoothAngle?: number | null; evaluated: { vertices: number; faces: number }; multiresLevels?: number; faceSets?: number; hiddenFaces?: number } | null
  dimensions: Vec3 | null
  modifiers: Modifier[]
  light: { kind: string; color: Vec3; power: number; radius: number; spotSize: number } | null
  camera: {
    lens: number
    sensor: number
    sensorHeight: number
    sensorFit: 'auto' | 'horizontal' | 'vertical'
    clipStart: number
    clipEnd: number
    orthographic: boolean
    orthoScale: number
    depthOfField?: boolean
    fStop?: number
    focusDistance?: number
    focusObject?: number | null
    blades?: number
    bladeRotation?: number
    ratio?: number
  } | null
  /** Animated properties, and those with a key on the current frame. */
  animated: string[]
  keyedNow: string[]
  /** The object's NLA, when it has one. */
  nla: ObjectNla | null
  /** An armature's bones. */
  bones: SceneBone[]
  /** A mesh's vertex groups (with how many vertices are in each) and shape keys. */
  vertexGroups: { name: string; vertices: number }[]
  shapeKeys: { name: string; value: number }[]
  /** The object's constraint stack, and its bones' (by bone name). */
  constraints: Constraint[]
  boneConstraints: Record<string, Constraint[]>
  /** A curve's or a text's data, as inspect shows it. */
  curve: SceneCurve | null
  text: SceneText | null
  lattice: SceneLattice | null
}

export type SceneLattice = {
  resolution: [number, number, number]
  interpolation: 'bspline' | 'linear'
  pointCount: number
  /** The points off their rest places. */
  moved: number[]
}

export type SceneCurve = {
  dimensions: '2d' | '3d'
  fillMode: 'full' | 'half' | 'both' | 'front' | 'back' | 'none'
  bevelDepth: number
  bevelResolution: number
  extrude: number
  offset: number
  fillCaps: boolean
  resolution: number
  summary: { type: 'bezier' | 'poly' | 'nurbs'; points: number; cyclic: boolean }[]
  evaluated: { vertices: number; faces: number }
}

export type SceneText = {
  body: string
  size: number
  extrude: number
  bevelDepth: number
  align: 'left' | 'center' | 'right'
  lineSpacing: number
  characterSpacing: number
  wordSpacing: number
  offsetX: number
  offsetY: number
  shear: number
  evaluated: { vertices: number; faces: number }
}

export type SceneBone = {
  name: string
  head: Vec3
  tail: Vec3
  /** Degrees. */
  roll: number
  parent: string | null
  connected: boolean
  deform: boolean
  pose: { location: Vec3; rotation: Vec3; scale: Vec3 }
  /** The envelope: radii and distance in metres, the weight, and Multiply. */
  envelope: { headRadius: number; tailRadius: number; distance: number; weight: number; multiply: boolean }
  /** The custom shape (rotation in degrees), or `null` for the octahedron. */
  shape: { object: number; name: string | null; translation: Vec3; rotation: Vec3; scale: Vec3; boneSize: boolean; wire: boolean } | null
}

export type SceneMaterial = {
  id: number
  name: string
  baseColor: [number, number, number, number]
  metallic: number
  roughness: number
  users: number
  /** Image hashes in the material's slots. */
  baseColorTexture?: string | null
  metallicRoughnessTexture?: string | null
  normalTexture?: string | null
  normalStrength?: number
  transmission?: number
  ior?: number
  alphaMode?: 'opaque' | 'clip' | 'blend'
  alphaCutoff?: number
  /** The shader group giving the surface and volume in path-traced renders. */
  shader?: number | null
}

/** An image the scene keeps for its materials. */
export type SceneImage = { hash: string; name: string; format: string; bytes: number; materials: number[] }

/** What the editor reads from the engine's `inspect()`. */
export type SceneInfo = {
  objects: SceneObject[]
  materials: SceneMaterial[]
  images: SceneImage[]
  nodeGroups: import('./NodeEditorTypes').NodeGroupInfo[]
  camera: number | null
  /** The world: its colour or image, and the shader group whose volume fills it. */
  world: { color: Vec3; strength: number; image: string | null; rotation: number; volume: number | null }
  /** The render size, the compositing group renders pass through and whether they do, and
   *  motion blur. */
  render: {
    width: number
    height: number
    compositor: number | null
    compositing: boolean
    motionBlur: boolean
    shutter: number
    shutterPosition: 'center' | 'start' | 'end'
    /** Ambient occlusion in fast renders: on, distance in metres (null: from the scene's size),
     *  and strength. */
    ao: boolean
    aoDistance: number | null
    aoStrength: number
    /** Colour management: view transform, look, and exposure in stops. */
    view: 'standard' | 'agx' | 'khronos-pbr-neutral'
    look: 'none' | 'punchy' | 'greyscale' | 'high-contrast' | 'low-contrast'
    exposure: number
  }
  /** Object mode, a mesh's edit mode, or an armature's edit or pose mode. */
  mode: 'object' | 'edit' | 'armatureEdit' | 'pose' | 'weightPaint' | 'curveEdit' | 'textEdit' | 'latticeEdit' | 'sculpt' | 'particleEdit'
  /** The selected control points in a curve's or a lattice's edit mode, and the caret in a text's. */
  points: number[]
  caret: number
  /** Weight paint's group, brush, X mirror, and tool, in weight paint mode. */
  paint: { group: string | null; brush: PaintBrush; mirror: PaintMirror; tool: PaintTool } | null
  /** Sculpt's brush and how many vertices are masked, in sculpt mode. */
  sculpt: SculptInfo | null
  /** Every object's particle systems. */
  particles: ParticleSystemInfo[]
  /** The groomed system and the brush, in Particle Edit mode. */
  particleEdit: { system: number; brush: GroomBrush; stroking: boolean } | null
  editing: number | null
  /** The selected and active bones in an armature's edit or pose mode. */
  bones: string[]
  activeBone: string | null
  select: 'vertex' | 'edge' | 'face'
  /** Proportional editing, when on. */
  proportional: { radius: number; falloff: string; connected: boolean } | null
  selected: number[]
  active: number | null
  vertices: number[]
  faces: number[]
  shading: 'wireframe' | 'solid' | 'material' | 'rendered'
  grid: boolean
  perspective: boolean
  throughCamera: boolean
  viewWidth: number
  viewHeight: number
  physics: PhysicsInfo
  /** Drivers, actions, and keying sets. */
  animation: AnimationData
}

type Raw = Record<string, any>

const vec = (value: unknown): Vec3 => (Array.isArray(value) && value.length === 3 ? [Number(value[0]), Number(value[1]), Number(value[2])] : [0, 0, 0])

function objectOf(raw: Raw): SceneObject {
  return {
    id: Number(raw.id),
    name: String(raw.name ?? ''),
    type: raw.type ?? 'empty',
    location: vec(raw.location),
    rotation: vec(raw.rotation),
    scale: vec(raw.scale),
    parent: typeof raw.parent === 'number' ? raw.parent : null,
    hidden: raw.hidden === true,
    material: typeof raw.material === 'number' ? raw.material : null,
    mesh: raw.mesh ?? null,
    dimensions: raw.dimensions ? vec(raw.dimensions) : null,
    modifiers: Array.isArray(raw.modifiers) ? raw.modifiers : [],
    light: raw.light ?? null,
    camera: raw.camera ?? null,
    animated: Array.isArray(raw.animated) ? raw.animated.map(String) : [],
    keyedNow: Array.isArray(raw.keyedNow) ? raw.keyedNow.map(String) : [],
    nla: nlaOf(raw.nla),
    bones: Array.isArray(raw.armature?.bones)
      ? raw.armature.bones.map((bone: Raw) => ({
          name: String(bone.name ?? ''),
          head: vec(bone.head),
          tail: vec(bone.tail),
          roll: Number(bone.roll ?? 0),
          parent: typeof bone.parent === 'string' ? bone.parent : null,
          connected: bone.connected === true,
          deform: bone.deform !== false,
          pose: { location: vec(bone.pose?.location), rotation: vec(bone.pose?.rotation), scale: bone.pose?.scale ? vec(bone.pose.scale) : [1, 1, 1] },
          envelope: {
            headRadius: Number(bone.envelope?.headRadius ?? 0.1),
            tailRadius: Number(bone.envelope?.tailRadius ?? 0.05),
            distance: Number(bone.envelope?.distance ?? 0.25),
            weight: Number(bone.envelope?.weight ?? 1),
            multiply: bone.envelope?.multiply === true,
          },
          shape:
            bone.shape && typeof bone.shape === 'object'
              ? {
                  object: Number(bone.shape.object),
                  name: typeof bone.shape.name === 'string' ? bone.shape.name : null,
                  translation: vec(bone.shape.translation),
                  rotation: vec(bone.shape.rotation),
                  scale: bone.shape.scale ? vec(bone.shape.scale) : [1, 1, 1],
                  boneSize: bone.shape.boneSize !== false,
                  wire: bone.shape.wire === true,
                }
              : null,
        }))
      : [],
    vertexGroups: Array.isArray(raw.vertexGroups) ? raw.vertexGroups.map((group: Raw) => ({ name: String(group.name), vertices: Number(group.vertices ?? 0) })) : [],
    shapeKeys: Array.isArray(raw.shapeKeys) ? raw.shapeKeys.map((key: Raw) => ({ name: String(key.name), value: Number(key.value ?? 0) })) : [],
    constraints: Array.isArray(raw.constraints) ? raw.constraints : [],
    boneConstraints: raw.boneConstraints && typeof raw.boneConstraints === 'object' ? raw.boneConstraints : {},
    curve: raw.curve && typeof raw.curve === 'object' ? (raw.curve as SceneCurve) : null,
    text: raw.text && typeof raw.text === 'object' ? (raw.text as SceneText) : null,
    lattice: raw.lattice && typeof raw.lattice === 'object' ? (raw.lattice as SceneLattice) : null,
  }
}

export function describe(info: KitEngineInfo): SceneInfo {
  const raw = info.inspected as Raw
  const view = (raw.view ?? {}) as Raw
  const selection = (raw.selection ?? {}) as Raw
  const mode = (raw.mode ?? {}) as Raw
  return {
    objects: Array.isArray(raw.objects) ? raw.objects.map(objectOf) : [],
    materials: Array.isArray(raw.materials) ? raw.materials : [],
    images: Array.isArray(raw.images) ? raw.images : [],
    nodeGroups: Array.isArray(raw.nodeGroups) ? raw.nodeGroups : [],
    camera: typeof raw.camera === 'number' ? raw.camera : null,
    world: {
      color: vec(raw.world?.color),
      strength: Number(raw.world?.strength ?? 1),
      image: typeof raw.world?.image === 'string' ? raw.world.image : null,
      rotation: Number(raw.world?.rotation ?? 0),
      volume: typeof raw.world?.volume === 'number' ? raw.world.volume : null,
    },
    render: {
      width: Number(raw.render?.width ?? 1920),
      height: Number(raw.render?.height ?? 1080),
      compositor: typeof raw.render?.compositor === 'number' ? raw.render.compositor : null,
      compositing: raw.render?.compositing !== false,
      motionBlur: raw.render?.motionBlur === true,
      shutter: Number(raw.render?.shutter ?? 0.5),
      shutterPosition: raw.render?.shutterPosition === 'start' || raw.render?.shutterPosition === 'end' ? raw.render.shutterPosition : 'center',
      ao: raw.render?.ao !== false,
      aoDistance: typeof raw.render?.aoDistance === 'number' ? raw.render.aoDistance : null,
      aoStrength: Number(raw.render?.aoStrength ?? 1),
      view: raw.render?.view === 'agx' || raw.render?.view === 'khronos-pbr-neutral' ? raw.render.view : 'standard',
      look: ['punchy', 'greyscale', 'high-contrast', 'low-contrast'].includes(raw.render?.look) ? raw.render.look : 'none',
      exposure: Number(raw.render?.exposure ?? 0),
    },
    mode: ['edit', 'armatureEdit', 'pose', 'weightPaint', 'curveEdit', 'textEdit', 'latticeEdit', 'sculpt', 'particleEdit'].includes(mode.mode) ? mode.mode : 'object',
    points: Array.isArray(mode.points) ? mode.points.map(Number) : [],
    caret: Number(mode.caret ?? 0),
    paint:
      mode.paint && typeof mode.paint === 'object'
        ? {
            group: typeof mode.paint.group === 'string' ? mode.paint.group : null,
            brush: mode.paint.brush as PaintBrush,
            mirror: { x: mode.paint.mirror?.x === true, topology: mode.paint.mirror?.topology === true, groups: mode.paint.mirror?.groups !== false },
            tool: PAINT_TOOLS.includes(mode.paint.tool) ? (mode.paint.tool as PaintTool) : 'brush',
          }
        : null,
    sculpt:
      mode.sculpt && typeof mode.sculpt === 'object'
        ? {
            brush: mode.sculpt.brush as SculptBrush,
            masked: Number(mode.sculpt.masked ?? 0),
            tool: String(mode.sculpt.tool ?? 'brush'),
            dyntopo: mode.sculpt.dyntopo as Dyntopo,
            level: Number(mode.sculpt.level ?? 0),
            levels: Number(mode.sculpt.levels ?? 0),
            faceSets: Array.isArray(mode.sculpt.faceSets) ? mode.sculpt.faceSets.map(Number) : [],
            hiddenFaces: Number(mode.sculpt.hiddenFaces ?? 0),
          }
        : null,
    particles: Array.isArray(raw.particles) ? (raw.particles as ParticleSystemInfo[]) : [],
    particleEdit: mode.particleEdit && typeof mode.particleEdit === 'object' ? { system: Number(mode.particleEdit.system ?? 0), brush: mode.particleEdit.brush as GroomBrush, stroking: mode.particleEdit.stroking === true } : null,
    bones: Array.isArray(mode.bones) ? mode.bones.map(String) : [],
    activeBone: typeof mode.activeBone === 'string' ? mode.activeBone : null,
    editing: typeof mode.object === 'number' ? mode.object : null,
    select: mode.select === 'face' || mode.select === 'edge' ? mode.select : 'vertex',
    proportional: mode.proportional && typeof mode.proportional === 'object' ? { radius: Number(mode.proportional.radius ?? 1), falloff: String(mode.proportional.falloff ?? 'smooth'), connected: mode.proportional.connected === true } : null,
    selected: Array.isArray(selection.objects) ? selection.objects : [],
    active: typeof selection.active === 'number' ? selection.active : null,
    vertices: Array.isArray(selection.vertices) ? selection.vertices : [],
    faces: Array.isArray(selection.faces) ? selection.faces : [],
    shading: view.shading === 'wireframe' || view.shading === 'material' || view.shading === 'rendered' ? view.shading : 'solid',
    grid: view.grid !== false,
    perspective: view.perspective !== false,
    throughCamera: typeof view.camera === 'number',
    viewWidth: Number(view.width ?? info.width),
    viewHeight: Number(view.height ?? info.height),
    physics: physicsOf(raw.physics),
    animation: animationDataOf(raw.animation),
  }
}

/** File commands the host's own Save, Open, and Close already cover (none: Export stays). */
export const HOST_FILE_COMMANDS: string[] = []

/** The menu bar's menus, in order. */
export const MENU_ORDER = ['File', 'Edit', 'View', 'Add', 'Object', 'Mesh', 'Curve', 'Text', 'Armature', 'Pose', 'Weights', 'Sculpt', 'Modifier', 'Material', 'Render']

/** Menus kept in a read-only document. */
export const VIEW_MENUS = ['View', 'Render']

/** The menu tab Single line and Classic open on: adding objects is what a scene starts with. */
export const DEFAULT_TAB = 'Add'

/** The commands Single line's row shows, by menu (Undo and Redo are in the top bar). */
export const SCENE_PRIMARY: Record<string, readonly string[]> = {
  Edit: ['edit.duplicate', 'edit.delete', 'edit.selectAll', 'edit.deselectAll'],
  Add: ['object.addCube', 'object.addUvSphere', 'object.addCylinder', 'object.addPlane', 'object.addLight', 'object.addCamera'],
  Object: ['edit.duplicate', 'edit.delete', 'object.join', 'object.shadeSmooth', 'object.shadeFlat'],
  Mesh: ['mesh.extrude', 'mesh.inset', 'mesh.subdivide', 'mesh.merge'],
  View: ['view.frameAll', 'view.frameSelected', 'view.camera'],
  Render: ['render.image'],
}

/** Linear 0..1 to an sRGB byte. */
export function toByte(linear: number) {
  const value = Math.min(1, Math.max(0, linear))
  const srgb = value <= 0.0031308 ? value * 12.92 : 1.055 * value ** (1 / 2.4) - 0.055
  return Math.round(srgb * 255)
}

/** Linear RGB as `#rrggbb` (sRGB). */
export function linearHex(color: readonly number[]) {
  return `#${color
    .slice(0, 3)
    .map((channel) => toByte(channel).toString(16).padStart(2, '0'))
    .join('')}`
}

/** The children of `parent` (null: the top level), in scene order. */
export function childrenOf(objects: SceneObject[], parent: number | null) {
  return objects.filter((object) => object.parent === parent || (parent === null && object.parent !== null && !objects.some((other) => other.id === object.parent)))
}
