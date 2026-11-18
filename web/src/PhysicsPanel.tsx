import { useCallback, useEffect, type ReactNode } from 'react'
import { TrashIcon } from '@kition/tessera-icons/react'
import { Button, Checkbox, Choice, IconButton, PanelSection, useKitEditor, useReadOnly } from '@kition/kit-editor'
import { NumberInput, Row, VectorRow, useRun } from './SidePanel'
import type { ClothInfo, CollisionInfo, PhysicsInfo, RigidBodyInfo, SoftBodyInfo } from './PhysicsInfo'
import { ConstraintBlock, FieldBlock } from './PhysicsLinks'
import type { SceneInfo, SceneObject } from './scene'
import './PhysicsPanel.css'
import { EffectorWeightsControls } from './EffectorWeights'

const SHAPES = ['box', 'sphere', 'capsule', 'cylinder', 'convexHull', 'mesh'] as const

/** A titled block of one kind of physics, with its remove button. */
function Block({ title, testId, onRemove, children }: { title: string; testId: string; onRemove: () => void; children: ReactNode }) {
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  return (
    <div className="scene-physics-block" data-testid={testId}>
      <div className="scene-physics-block-header">
        <span className="scene-physics-block-title">{title}</span>
        <IconButton icon={TrashIcon} label={`${title}: ${t('physicsRemove')}`} disabled={readOnly} onClick={onRemove} data-testid={`${testId}-remove`} />
      </div>
      {children}
    </div>
  )
}

function RigidBodyBlock({ object, body }: { object: SceneObject; body: RigidBodyInfo }) {
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const set = (values: Record<string, unknown>) => run('physics.rigidBody.set', { objects: [object.id], ...values })
  const number = (label: string, key: string, value: number, min: number, max: number, step: number, unit?: string) => (
    <Row label={label}>
      <NumberInput label={label} value={value} min={min} max={max} step={step} unit={unit} onCommit={(next) => set({ [key]: next })} testId={`scene-rigid-${key}`} />
    </Row>
  )
  return (
    <Block title={t('physicsRigidBody')} testId="scene-rigid" onRemove={() => run('physics.rigidBody.remove', { objects: [object.id] })}>
      <Row label={t('physicsType')}>
        <Choice
          label={t('physicsType')}
          quiet
          value={body.kind}
          disabled={readOnly}
          choices={[
            ['active', t('physicsActive')],
            ['passive', t('physicsPassive')],
          ]}
          onChange={(kind) => set({ type: kind })}
          testId="scene-rigid-type"
        />
      </Row>
      <Row label={t('physicsShape')}>
        <Choice
          label={t('physicsShape')}
          quiet
          value={body.shape}
          disabled={readOnly}
          choices={SHAPES.filter((shape) => shape !== 'mesh' || body.kind === 'passive').map((shape) => [shape, t(`shape_${shape}`)] as const)}
          onChange={(shape) => set({ shape })}
          testId="scene-rigid-shape"
        />
      </Row>
      {body.kind === 'active' ? (
        <>
          {number(t('physicsMass'), 'mass', body.mass, 0.001, 1e6, 0.1, 'kg')}
          <Row label={t('physicsDynamic')}>
            <Checkbox label={t('physicsDynamic')} checked={body.enabled} disabled={readOnly} onChange={(enabled) => set({ enabled })} />
          </Row>
        </>
      ) : (
        <Row label={t('physicsAnimated')}>
          <Checkbox label={t('physicsAnimated')} checked={body.animated} disabled={readOnly} onChange={(animated) => set({ animated })} />
        </Row>
      )}
      {number(t('physicsFriction'), 'friction', body.friction, 0, 10, 0.05)}
      {number(t('physicsBounciness'), 'restitution', body.restitution, 0, 1, 0.05)}
      {number(t('physicsLinearDamping'), 'linearDamping', body.linearDamping, 0, 1, 0.01)}
      {number(t('physicsAngularDamping'), 'angularDamping', body.angularDamping, 0, 1, 0.01)}
      {number(t('physicsMargin'), 'margin', body.margin, 0, 1, 0.01, 'm')}
      {body.kind === 'active' ? (
        <>
          <Row label={t('physicsDeactivation')}>
            <Checkbox label={t('physicsDeactivation')} checked={body.deactivation} disabled={readOnly} onChange={(deactivation) => set({ deactivation })} testId="scene-rigid-deactivation" />
          </Row>
          {body.deactivation ? (
            <>
              <Row label={t('physicsStartAsleep')}>
                <Checkbox label={t('physicsStartAsleep')} checked={body.startDeactivated} disabled={readOnly} onChange={(startDeactivated) => set({ startDeactivated })} testId="scene-rigid-asleep" />
              </Row>
              {number(t('physicsSleepSpeed'), 'deactivateLinear', body.deactivateLinear, 0, 1000, 0.05, 'm/s')}
              {number(t('physicsSleepSpin'), 'deactivateAngular', body.deactivateAngular, 0, 1000, 0.05, 'rad/s')}
            </>
          ) : null}
        </>
      ) : null}
    </Block>
  )
}

function ClothBlock({ object, cloth }: { object: SceneObject; cloth: ClothInfo }) {
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const set = (values: Record<string, unknown>) => run('physics.cloth.set', { objects: [object.id], ...values })
  const number = (label: string, key: string, value: number, min: number, max: number, step: number, unit?: string) => (
    <Row label={label}>
      <NumberInput label={label} value={value} min={min} max={max} step={step} unit={unit} onCommit={(next) => set({ [key]: key === 'quality' ? Math.round(next) : next })} testId={`scene-cloth-${key}`} />
    </Row>
  )
  return (
    <Block title={t('physicsCloth')} testId="scene-cloth" onRemove={() => run('physics.cloth.remove', { objects: [object.id] })}>
      {number(t('physicsQuality'), 'quality', cloth.quality, 1, 80, 1)}
      {number(t('physicsVertexMass'), 'mass', cloth.mass, 0.0001, 1000, 0.05, 'kg')}
      {number(t('physicsAirDamping'), 'airDamping', cloth.airDamping, 0, 10, 0.1)}
      {number(t('physicsTension'), 'tension', cloth.tension, 0, 10000, 1)}
      {number(t('physicsCompression'), 'compression', cloth.compression, 0, 10000, 1)}
      {number(t('physicsShear'), 'shear', cloth.shear, 0, 10000, 1)}
      {number(t('physicsBending'), 'bending', cloth.bending, 0, 10000, 0.1)}
      <Row label={t('physicsPinGroup')}>
        <Choice
          label={t('physicsPinGroup')}
          quiet
          value={cloth.pinGroup ?? ''}
          disabled={readOnly}
          choices={[['', t('physicsNoPin')], ...object.vertexGroups.map((group) => [group.name, group.name] as const)]}
          onChange={(pinGroup) => set({ pinGroup })}
          testId="scene-cloth-pin"
        />
      </Row>
      {number(t('physicsDistance'), 'collisionDistance', cloth.collisionDistance, 0, 1, 0.005, 'm')}
      <Row label={t('physicsSelfCollision')}>
        <Checkbox label={t('physicsSelfCollision')} checked={cloth.selfCollision} disabled={readOnly} onChange={(selfCollision) => set({ selfCollision })} testId="scene-cloth-self" />
      </Row>
      {cloth.selfCollision ? number(t('physicsSelfDistance'), 'selfDistance', cloth.selfDistance, 0.001, 1, 0.005, 'm') : null}
      <EffectorWeightsControls weights={cloth.effectorWeights} system="cloth" set={effectorWeights => set({ effectorWeights })} />
    </Block>
  )
}

function SoftBodyBlock({ object, soft }: { object: SceneObject; soft: SoftBodyInfo }) {
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const set = (values: Record<string, unknown>) => run('physics.softBody.set', { objects: [object.id], ...values })
  const number = (label: string, key: keyof SoftBodyInfo, min: number, max: number, step: number, unit?: string) => (
    <Row label={label}>
      <NumberInput label={label} value={Number(soft[key])} min={min} max={max} step={step} unit={unit} onCommit={(next) => set({ [key]: key === 'frameStart' || key === 'frameEnd' ? Math.round(next) : next })} testId={`scene-soft-${key}`} />
    </Row>
  )
  const toggle = (label: string, key: 'goal' | 'edges' | 'selfCollision') => (
    <Row label={label}>
      <Checkbox label={label} checked={soft[key]} disabled={readOnly} onChange={(on) => set({ [key]: on })} testId={`scene-soft-${key}`} />
    </Row>
  )
  return (
    <Block title={t('physicsSoftBody')} testId="scene-soft" onRemove={() => run('physics.softBody.remove', { objects: [object.id] })}>
      {number(t('physicsVertexMass'), 'mass', 0.001, 1000, 0.1, 'kg')}
      {number(t('physicsSoftFriction'), 'friction', 0, 50, 0.1)}
      <EffectorWeightsControls weights={soft.effectorWeights} system="soft" set={effectorWeights => set({ effectorWeights })} />
      {number(t('physicsSpeed'), 'speed', 0.01, 100, 0.1)}
      {toggle(t('physicsGoal'), 'goal')}
      {soft.goal ? (
        <>
          <Row label={t('physicsGoalGroup')}>
            <Choice
              label={t('physicsGoalGroup')}
              quiet
              value={soft.goalGroup ?? ''}
              disabled={readOnly}
              choices={[['', t('physicsGoalNone')], ...object.vertexGroups.map((group) => [group.name, group.name] as const)]}
              onChange={(goalGroup) => set({ goalGroup })}
              testId="scene-soft-goalGroup"
            />
          </Row>
          {number(t('physicsGoalDefault'), 'goalDefault', 0, 1, 0.05)}
          {soft.goalGroup ? number(t('physicsGoalMin'), 'goalMin', 0, 1, 0.05) : null}
          {soft.goalGroup ? number(t('physicsGoalMax'), 'goalMax', 0, 1, 0.05) : null}
          {number(t('physicsGoalStiffness'), 'goalStiffness', 0, 0.999, 0.05)}
          {number(t('physicsGoalDamping'), 'goalDamping', 0, 50, 0.1)}
        </>
      ) : null}
      {toggle(t('physicsEdges'), 'edges')}
      {soft.edges ? (
        <>
          {number(t('physicsPull'), 'pull', 0, 0.999, 0.05)}
          {number(t('physicsPush'), 'push', 0, 0.999, 0.05)}
          {number(t('physicsEdgeDamping'), 'damping', 0, 50, 0.1)}
          {number(t('physicsBending'), 'bending', 0, 10, 0.1)}
        </>
      ) : null}
      {toggle(t('physicsSelfCollision'), 'selfCollision')}
      {soft.selfCollision ? number(t('physicsBallSize'), 'ballSize', 0.001, 10, 0.05) : null}
      <Row label={t('physicsFrames')}>
        <div className="scene-vector scene-vector-2">
          <NumberInput label={t('physicsStart')} value={soft.frameStart} min={-100000} max={100000} step={1} onCommit={(frameStart) => set({ frameStart: Math.round(frameStart) })} testId="scene-soft-start" />
          <NumberInput label={t('physicsEnd')} value={soft.frameEnd} min={-100000} max={100000} step={1} onCommit={(frameEnd) => set({ frameEnd: Math.round(frameEnd) })} testId="scene-soft-end" />
        </div>
      </Row>
    </Block>
  )
}

function CollisionBlock({ object, collision }: { object: SceneObject; collision: CollisionInfo }) {
  const { t } = useKitEditor()
  const run = useRun()
  const set = (values: Record<string, unknown>) => run('physics.collision.add', { objects: [object.id], ...values })
  return (
    <Block title={t('physicsCollision')} testId="scene-collision" onRemove={() => run('physics.collision.remove', { objects: [object.id] })}>
      <Row label={t('physicsThickness')}>
        <NumberInput label={t('physicsThickness')} value={collision.thickness} min={0} max={1} step={0.005} unit="m" onCommit={(thickness) => set({ thickness })} testId="scene-collision-thickness" />
      </Row>
      <Row label={t('physicsFriction')}>
        <NumberInput label={t('physicsFriction')} value={collision.friction} min={0} max={100} step={0.5} onCommit={(friction) => set({ friction })} testId="scene-collision-friction" />
      </Row>
      <Row label={t('physicsCollisionDamping')}>
        <NumberInput label={t('physicsCollisionDamping')} value={collision.damping} min={0} max={1} step={0.05} onCommit={(damping) => set({ damping })} testId="scene-collision-damping" />
      </Row>
    </Block>
  )
}

/** The active object's physics: a rigid body, cloth or a soft body, and collision (meshes), a
 *  force field, and a rigid body constraint (any object), and their settings. */
export function PhysicsSection({ object, info }: { object: SceneObject; info: SceneInfo }) {
  const { phrase: p, t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const physics = info.physics.objects.find((item) => item.id === object.id)
  const add = (command: string) => run(command, { objects: [object.id] })
  const mesh = object.type === 'mesh'
  return (
    <PanelSection title={p('Physics')} defaultOpen={physics != null} testId="scene-physics">
      <div className="scene-physics-add">
        {mesh ? (
          <>
            <Button disabled={readOnly || physics?.rigidBody != null} onClick={() => add('physics.rigidBody.add')} data-testid="scene-physics-add-rigid">
              {t('physicsRigidBody')}
            </Button>
            <Button disabled={readOnly || physics?.cloth != null || physics?.softBody != null} onClick={() => add('physics.cloth.add')} data-testid="scene-physics-add-cloth">
              {t('physicsCloth')}
            </Button>
            <Button disabled={readOnly || physics?.softBody != null || physics?.cloth != null} onClick={() => add('physics.softBody.add')} data-testid="scene-physics-add-soft">
              {t('physicsSoftBody')}
            </Button>
            <Button disabled={readOnly || physics?.collision != null} onClick={() => add('physics.collision.add')} data-testid="scene-physics-add-collision">
              {t('physicsCollision')}
            </Button>
          </>
        ) : null}
        <Button disabled={readOnly || physics?.field != null} onClick={() => add('physics.field.add')} data-testid="scene-physics-add-field">
          {t('physicsField')}
        </Button>
        <Button disabled={readOnly || physics?.rigidConstraint != null} onClick={() => add('physics.constraint.add')} data-testid="scene-physics-add-constraint">
          {t('physicsConstraint')}
        </Button>
      </div>
      {physics ? null : <p className="kui-muted scene-physics-none">{t(mesh ? 'physicsNone' : 'physicsNoneEmpty')}</p>}
      {physics?.rigidBody ? <RigidBodyBlock object={object} body={physics.rigidBody} /> : null}
      {physics?.cloth ? <ClothBlock object={object} cloth={physics.cloth} /> : null}
      {physics?.softBody ? <SoftBodyBlock object={object} soft={physics.softBody} /> : null}
      {physics?.collision ? <CollisionBlock object={object} collision={physics.collision} /> : null}
      {physics?.field ? <FieldBlock object={object} field={physics.field} /> : null}
      {physics?.rigidConstraint ? <ConstraintBlock object={object} constraint={physics.rigidConstraint} info={info} /> : null}
    </PanelSection>
  )
}

/** Failed commands retain their typed error but do not publish fresh worker info. A single
 * successful no-op step reads the terminal failure state and enables an explicit retry. */
function useBakeCommand() {
  const { run } = useKitEditor()
  return useCallback(async (command: 'physics.bake' | 'physics.bakeStep', params: Record<string, unknown>) => {
    const result = await run(command, params)
    if (result === null) await run('physics.bakeStep', { frames: 1 })
  }, [run])
}

/** Steps a bake under way in the engine's worker, a few frames per call, until it is done. */
function useBakeSteps(physics: PhysicsInfo) {
  const run = useBakeCommand()
  const baking = physics.state === 'baking'
  useEffect(() => {
    if (!baking) return
    const timer = window.setTimeout(() => void run('physics.bakeStep', { frames: 4 }), 0)
    return () => window.clearTimeout(timer)
  }, [baking, physics.frames, run])
}

/** The scene's physics world, and Bake and Free with the bake's progress. */
export function PhysicsWorldSection({ info }: { info: SceneInfo }) {
  const { phrase: p, t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const bake = useBakeCommand()
  const physics = info.physics
  const world = physics.world
  useBakeSteps(physics)
  const set = (values: Record<string, unknown>) => run('physics.world.set', values)
  const fraction = physics.total > 0 ? Math.min(1, physics.frames / physics.total) : 0
  const hasPhysics = physics.bakeable
  return (
    <PanelSection title={p('Physics')} defaultOpen={false} testId="scene-physics-world">
      <VectorRow label={t('physicsGravity')} value={world.gravity} unit="m/s²" step={0.1} onCommit={(gravity) => set({ gravity })} testId="scene-physics-gravity" />
      <Row label={t('physicsSubsteps')}>
        <NumberInput label={t('physicsSubsteps')} value={world.substeps} min={1} max={100} step={1} onCommit={(substeps) => set({ substeps: Math.round(substeps) })} testId="scene-physics-substeps" />
      </Row>
      <Row label={t('physicsIterations')}>
        <NumberInput label={t('physicsIterations')} value={world.iterations} min={1} max={100} step={1} onCommit={(iterations) => set({ iterations: Math.round(iterations) })} testId="scene-physics-iterations" />
      </Row>
      <Row label={t('physicsSpeed')}>
        <NumberInput label={t('physicsSpeed')} value={world.timeScale} min={0.01} max={100} step={0.1} onCommit={(timeScale) => set({ timeScale })} testId="scene-physics-speed" />
      </Row>
      <Row label={t('physicsFrames')}>
        <div className="scene-vector scene-vector-2">
          <NumberInput label={t('physicsStart')} value={world.frameStart} min={-100000} max={100000} step={1} onCommit={(frameStart) => set({ frameStart: Math.round(frameStart) })} testId="scene-physics-start" />
          <NumberInput label={t('physicsEnd')} value={world.frameEnd} min={-100000} max={100000} step={1} onCommit={(frameEnd) => set({ frameEnd: Math.round(frameEnd) })} testId="scene-physics-end" />
        </div>
      </Row>
      <div className="scene-physics-bake">
        <Button variant="primary" disabled={readOnly || !hasPhysics || physics.state === 'baking'} onClick={() => void bake('physics.bake', { wait: false })} data-testid="scene-physics-bake">
          {t('physicsBake')}
        </Button>
        <Button disabled={physics.state === 'none'} onClick={() => run('physics.free', {})} data-testid="scene-physics-free">
          {t('physicsFree')}
        </Button>
      </div>
      <div className="scene-physics-progress" data-state={physics.state} data-testid="scene-physics-progress">
        <div className="scene-physics-progress-track" aria-hidden="true">
          <div className="scene-physics-progress-fill" style={{ width: `${Math.round(fraction * 100)}%` }} />
        </div>
        <span className="scene-physics-progress-label tabular-nums">{t(`physicsState_${physics.state}`, { frames: physics.frames, total: physics.total })}</span>
      </div>
      {physics.error ? <p className="scene-physics-failure" role="alert">{physics.error}</p> : null}
    </PanelSection>
  )
}
