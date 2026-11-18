import type { Vec3 } from './scene'

/** A rigid body's settings, as the engine reports them. */
export type RigidBodyInfo = {
  kind: 'active' | 'passive'
  enabled: boolean
  animated: boolean
  mass: number
  friction: number
  restitution: number
  linearDamping: number
  angularDamping: number
  shape: 'box' | 'sphere' | 'capsule' | 'cylinder' | 'convexHull' | 'mesh'
  margin: number
  deactivation: boolean
  startDeactivated: boolean
  deactivateLinear: number
  deactivateAngular: number
}

export type EffectorWeightsInfo = { all: number; gravity: number; force: number; wind: number; vortex: number; turbulence: number; drag: number }

export type ClothInfo = {
  effectorWeights: EffectorWeightsInfo
  quality: number
  mass: number
  airDamping: number
  tension: number
  compression: number
  shear: number
  bending: number
  pinGroup?: string
  collisionDistance: number
  selfCollision: boolean
  selfDistance: number
}

/** A soft body's settings, as the engine reports them. */
export type SoftBodyInfo = {
  effectorWeights: EffectorWeightsInfo
  mass: number
  friction: number
  speed: number
  goal: boolean
  goalGroup?: string
  goalDefault: number
  goalMin: number
  goalMax: number
  goalStiffness: number
  goalDamping: number
  edges: boolean
  pull: number
  push: number
  damping: number
  bending: number
  selfCollision: boolean
  ballSize: number
  frameStart: number
  frameEnd: number
}

export type CollisionInfo = { thickness: number; friction: number; damping: number }

export const FIELD_KINDS = ['force', 'wind', 'vortex', 'turbulence', 'drag'] as const
export type FieldKind = (typeof FIELD_KINDS)[number]

/** A force field's settings (distances absent when unlimited). */
export type FieldInfo = {
  kind: FieldKind
  strength: number
  flow: number
  falloffPower: number
  maxDistance?: number
  minDistance?: number
  size: number
  seed: number
  linearDrag: number
  quadraticDrag: number
}

export const JOINT_KINDS = ['fixed', 'point', 'hinge', 'slider', 'genericSpring', 'motor'] as const
export type JointKind = (typeof JOINT_KINDS)[number]

type Range = [number, number] | null

/** A rigid body constraint's settings (angles in radians, as the engine keeps them). */
export type ConstraintInfo = {
  kind: JointKind
  enabled: boolean
  object1?: number
  object2?: number
  disableCollisions: boolean
  breakable: boolean
  breakingThreshold: number
  angleLimit?: [number, number]
  distanceLimit?: [number, number]
  linearLimits: [Range, Range, Range]
  angularLimits: [Range, Range, Range]
  linearSprings: [number | null, number | null, number | null]
  angularSprings: [number | null, number | null, number | null]
  springDamping: number
  linearMotor: boolean
  angularMotor: boolean
  linearTargetVelocity: number
  angularTargetVelocity: number
  linearMaxImpulse: number
  angularMaxImpulse: number
}

export type ObjectPhysicsInfo = {
  id: number
  rigidBody?: RigidBodyInfo
  cloth?: ClothInfo
  softBody?: SoftBodyInfo
  collision?: CollisionInfo
  field?: FieldInfo
  rigidConstraint?: ConstraintInfo
}

/** The physics world, each object's physics, and the bake's state. */
export type PhysicsInfo = {
  world: { gravity: Vec3; substeps: number; iterations: number; timeScale: number; frameStart: number; frameEnd: number }
  objects: ObjectPhysicsInfo[]
  /** Something can be baked: an object with physics, or dynamic hair. */
  bakeable: boolean
  state: 'none' | 'baking' | 'baked' | 'outdated' | 'failed'
  error?: string
  frames: number
  total: number
}

type Raw = Record<string, any>

/** The `physics` inspect reports. */
export function physicsOf(raw: unknown): PhysicsInfo {
  const value = (raw && typeof raw === 'object' ? raw : {}) as Raw
  const world = (value.world ?? {}) as Raw
  const gravity = Array.isArray(world.gravity) && world.gravity.length === 3 ? (world.gravity.map(Number) as Vec3) : ([0, 0, -9.81] as Vec3)
  return {
    world: {
      gravity,
      substeps: Number(world.substeps ?? 10),
      iterations: Number(world.iterations ?? 10),
      timeScale: Number(world.timeScale ?? 1),
      frameStart: Number(world.frameStart ?? 1),
      frameEnd: Number(world.frameEnd ?? 250),
    },
    objects: Array.isArray(value.objects) ? (value.objects as ObjectPhysicsInfo[]) : [],
    bakeable: value.bakeable === true,
    state: ['baking', 'baked', 'outdated', 'failed'].includes(value.state) ? value.state : 'none',
    error: typeof value.error === 'string' ? value.error : undefined,
    frames: Number(value.frames ?? 0),
    total: Number(value.total ?? 0),
  }
}
