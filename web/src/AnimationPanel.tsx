import { PlusIcon, TrashIcon } from '@kition/tessera/icons/react'
import { Button, Choice, IconButton, MenuButton, MenuItem, PanelSection, useKitEditor, useReadOnly } from '@kition/sdk/editor'
import type { DriverInfo, DriverVariable } from './AnimationData'
import { Row, useRun } from './SidePanel'
import type { SceneInfo, SceneObject } from './scene'
import './AnimationPanel.css'

/** A text field that runs `onCommit` when it loses the focus or Enter is pressed. */
function TextField({ label, value, onCommit, testId, mono }: { label: string; value: string; onCommit: (value: string) => void; testId?: string; mono?: boolean }) {
  const readOnly = useReadOnly()
  return (
    <input
      key={value}
      className="kui-field scene-anim-text"
      data-mono={mono || undefined}
      aria-label={label}
      defaultValue={value}
      disabled={readOnly}
      spellCheck={false}
      onKeyDown={(event) => {
        event.stopPropagation()
        if (event.key === 'Enter') event.currentTarget.blur()
      }}
      onBlur={(event) => {
        const next = event.currentTarget.value
        if (next !== value) onCommit(next)
      }}
      data-testid={testId}
    />
  )
}

/** The properties a driver can drive on an object, with how many components each has. */
function drivable(object: SceneObject): [string, number][] {
  const out: [string, number][] = [
    ['location', 3],
    ['rotation', 3],
    ['scale', 3],
    ['hidden', 1],
  ]
  if (object.type === 'light') out.push(['power', 1], ['color', 3])
  if (object.type === 'camera') out.push(['lens', 1])
  if (object.material != null) out.push(['baseColor', 4])
  return out
}

const COMPONENT = ['X', 'Y', 'Z', 'W']
const CHANNELS = ['locationX', 'locationY', 'locationZ', 'rotationX', 'rotationY', 'rotationZ', 'scaleX', 'scaleY', 'scaleZ', 'scaleAverage']

/** A variable as `driver.set` takes it back. */
function variableParam(variable: DriverVariable): Record<string, unknown> {
  switch (variable.type) {
    case 'singleProperty':
      return { name: variable.name, type: variable.type, object: variable.object ?? null, property: variable.property ?? 'location', index: variable.index ?? 0, ...(variable.path ? { bone: variable.path } : {}) }
    case 'transform':
      return { name: variable.name, type: variable.type, object: variable.target?.object ?? null, bone: variable.target?.bone ?? undefined, channel: variable.channel ?? 'locationX', space: variable.space ?? 'world' }
    default:
      return { name: variable.name, type: variable.type, a: variable.a?.object ?? null, aBone: variable.a?.bone ?? undefined, b: variable.b?.object ?? null, bBone: variable.b?.bone ?? undefined }
  }
}

function VariableRow({ info, variable, index, onChange, onRemove, testId }: { info: SceneInfo; variable: DriverVariable; index: number; onChange: (next: Record<string, unknown>) => void; onRemove: () => void; testId: string }) {
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  const base = variableParam(variable)
  const objects: [string, string][] = [['', t('noTarget')], ...info.objects.map((object) => [String(object.id), object.name] as [string, string])]
  const objectChoice = (label: string, value: number | null | undefined, field: string) => (
    <Choice label={label} quiet value={value != null ? String(value) : ''} disabled={readOnly} choices={objects} onChange={(next) => onChange({ ...base, [field]: next ? Number(next) : null })} testId={`${testId}-${field}`} />
  )
  let fields = null
  switch (variable.type) {
    case 'singleProperty':
      fields = (
        <>
          {objectChoice(t('variableObject'), variable.object, 'object')}
          <Choice
            label={t('driverProperty')}
            quiet
            value={`${variable.property ?? 'location'}:${variable.index ?? 0}`}
            disabled={readOnly}
            choices={(['location', 'rotation', 'scale'] as const).flatMap((property) => [0, 1, 2].map((axis) => [`${property}:${axis}`, `${t(`prop_${property}`)} ${COMPONENT[axis]}`] as const))}
            onChange={(next) => {
              const [property, axis] = next.split(':')
              onChange({ ...base, property, index: Number(axis) })
            }}
          />
        </>
      )
      break
    case 'transform':
      fields = (
        <>
          {objectChoice(t('variableObject'), variable.target?.object, 'object')}
          <Choice
            label={t('variableChannel')}
            quiet
            value={variable.channel ?? 'locationX'}
            disabled={readOnly}
            choices={CHANNELS.map((channel) => [channel, channel === 'scaleAverage' ? `${t('prop_scale')} ⌀` : `${t(`prop_${channel.slice(0, -1)}`)} ${channel.slice(-1)}`] as const)}
            onChange={(channel) => onChange({ ...base, channel })}
            testId={`${testId}-channel`}
          />
          <Choice label={t('variableSpace')} quiet value={variable.space ?? 'world'} disabled={readOnly} choices={(['world', 'transform', 'local'] as const).map((space) => [space, t(`vspace_${space}`)] as const)} onChange={(space) => onChange({ ...base, space })} />
        </>
      )
      break
    default:
      fields = (
        <>
          {objectChoice(t('variableFirst'), variable.a?.object, 'a')}
          {objectChoice(t('variableSecond'), variable.b?.object, 'b')}
        </>
      )
  }
  return (
    <div className="scene-anim-variable" data-testid={testId}>
      <div className="scene-anim-variable-head">
        <TextField label={t('variableName')} value={variable.name} onCommit={(name) => onChange({ ...base, name })} testId={`${testId}-name`} mono />
        <Choice
          label={t('variableType')}
          quiet
          value={variable.type}
          disabled={readOnly}
          choices={(['singleProperty', 'transform', 'distance', 'rotationDifference'] as const).map((type) => [type, t(`var_${type}`)] as const)}
          onChange={(type) => onChange({ name: variable.name, type })}
          testId={`${testId}-type`}
        />
        <IconButton icon={TrashIcon} label={t('removeVariable')} disabled={readOnly} onClick={onRemove} data-testid={`${testId}-remove`} />
      </div>
      <div className="scene-anim-variable-fields" data-index={index}>
        {fields}
      </div>
    </div>
  )
}

function DriverCard({ info, driver, owner, index }: { info: SceneInfo; driver: DriverInfo; owner: SceneObject; index: number }) {
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const testId = `scene-driver-${index}`
  const where = { object: owner.id, property: driver.property, index: driver.index, ...(driver.path ? (driver.property === 'shapeKey' ? { shapeKey: driver.path } : { bone: driver.path }) : {}) }
  const set = (values: Record<string, unknown>) => run('driver.set', { ...where, ...values })
  const variables = driver.variables.map(variableParam)
  const label = `${t(`prop_${driver.property}`)}${driver.path ? ` (${driver.path})` : ''} ${COMPONENT[driver.index] ?? ''}`
  return (
    <div className="scene-modifier scene-anim-driver" data-testid={testId} data-error={driver.error ? true : undefined}>
      <div className="scene-modifier-header">
        <span className="scene-modifier-name">{label}</span>
        <span className="scene-modifier-actions">
          <IconButton icon={TrashIcon} label={t('removeVariable')} disabled={readOnly} onClick={() => run('driver.remove', where)} data-testid={`${testId}-remove`} />
        </span>
      </div>
      <div className="scene-modifier-body">
        <Row label={t('driverKind')}>
          <Choice label={t('driverKind')} quiet value={driver.kind} disabled={readOnly} choices={(['scripted', 'average', 'sum', 'min', 'max'] as const).map((kind) => [kind, t(`kind_${kind}`)] as const)} onChange={(kind) => set({ kind })} testId={`${testId}-kind`} />
        </Row>
        {driver.kind === 'scripted' ? (
          <Row label={t('expression')}>
            <TextField label={t('expression')} value={driver.expression} onCommit={(expression) => set({ expression })} testId={`${testId}-expression`} mono />
          </Row>
        ) : null}
        <div className="scene-anim-variables">
          {driver.variables.map((variable, at) => (
            <VariableRow
              key={`${at}-${variable.name}`}
              info={info}
              variable={variable}
              index={at}
              testId={`${testId}-variable-${at}`}
              onChange={(next) => set({ variables: variables.map((old, slot) => (slot === at ? next : old)) })}
              onRemove={() => set({ variables: variables.filter((_, slot) => slot !== at) })}
            />
          ))}
          <Button
            size="sm"
            disabled={readOnly}
            onClick={() => {
              const taken = new Set(driver.variables.map((variable) => variable.name))
              let name = 'var'
              for (let n = 1; taken.has(name); n += 1) name = `var${n}`
              set({ variables: [...variables, { name, type: 'singleProperty', object: null, property: 'location', index: 0 }] })
            }}
            data-testid={`${testId}-add-variable`}
          >
            {t('addVariable')}
          </Button>
        </div>
        <p className="kui-muted scene-anim-status" data-testid={`${testId}-status`}>
          {driver.error ? t('driverError', { error: driver.error }) : t('driverValue', { value: driver.value ?? 0 })}
        </p>
      </div>
    </div>
  )
}

/** The drivers on the active object's properties (and its material's), with the menu that adds one. */
export function DriversSection({ object, info }: { object: SceneObject; info: SceneInfo }) {
  const { phrase: p, t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const drivers = info.animation.drivers.filter((driver) => driver.object === object.id || (driver.material != null && driver.material === object.material))
  return (
    <PanelSection
      title={p('Drivers')}
      testId="scene-drivers"
      actions={
        <MenuButton
          align="end"
          trigger={({ open, toggle }) => <IconButton icon={PlusIcon} label={p('Add Driver')} aria-expanded={open} disabled={readOnly || info.mode !== 'object'} onClick={toggle} data-testid="scene-add-driver" />}
        >
          {drivable(object).flatMap(([property, components]) =>
            [...Array(components).keys()].map((axis) => (
              <MenuItem
                key={`${property}-${axis}`}
                label={components > 1 ? `${t(`prop_${property}`)} ${COMPONENT[axis]}` : t(`prop_${property}`)}
                onSelect={() => run('driver.add', { object: object.id, property, index: axis })}
                testId={`scene-add-driver-${property}-${axis}`}
              />
            )),
          )}
        </MenuButton>
      }
    >
      {drivers.length ? (
        <div className="scene-modifiers">
          {drivers.map((driver, index) => (
            <DriverCard key={`${driver.property}-${driver.index}-${driver.path ?? ''}`} info={info} driver={driver} owner={object} index={index} />
          ))}
        </div>
      ) : (
        <p className="kui-muted scene-empty">{t('noDrivers')}</p>
      )}
    </PanelSection>
  )
}

/** The scene's actions: stash the active object's keys in one, assign one to the selection, rename, remove. */
export function ActionsSection({ info }: { info: SceneInfo }) {
  const { phrase: p, t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const active = info.objects.find((object) => object.id === info.active)
  const actions = info.animation.actions
  return (
    <PanelSection
      title={p('Actions')}
      testId="scene-actions"
      actions={
        <IconButton
          icon={PlusIcon}
          label={t('stashAction')}
          disabled={readOnly || !active || active.animated.length === 0 || info.mode !== 'object'}
          onClick={() => active && run('action.new', { object: active.id, name: `${active.name}Action` })}
          data-testid="scene-stash-action"
        />
      }
    >
      {actions.length ? (
        <div className="scene-anim-list">
          {actions.map((action, index) => (
            <div className="scene-anim-item" key={action.name} data-testid={`scene-action-${index}`}>
              <TextField label={t('actionName')} value={action.name} onCommit={(to) => to.trim() && run('action.rename', { name: action.name, to: to.trim() })} testId={`scene-action-${index}-name`} />
              <span className="kui-muted scene-anim-meta">{action.range ? t('actionSummary', { channels: action.channels, start: action.range[0], end: action.range[1] }) : t('actionEmpty')}</span>
              <span className="scene-anim-actions">
                <Button size="sm" disabled={readOnly || info.selected.length === 0 || info.mode !== 'object'} onClick={() => run('action.assign', { action: action.name })} data-testid={`scene-action-${index}-assign`}>
                  {t('assignAction')}
                </Button>
                <IconButton icon={TrashIcon} label={p('Remove Action')} disabled={readOnly} onClick={() => run('action.remove', { name: action.name })} />
              </span>
            </div>
          ))}
        </div>
      ) : (
        <p className="kui-muted scene-empty">{t('noActions')}</p>
      )}
    </PanelSection>
  )
}

/** The scene's own keying sets: one made from the selection's location, rotation, and scale, and what each keys. */
export function KeyingSetsSection({ info }: { info: SceneInfo }) {
  const editor = useKitEditor()
  const { phrase: p, t } = editor
  const readOnly = useReadOnly()
  const run = useRun()
  const sets = info.animation.keyingSets.sets
  return (
    <PanelSection
      title={p('Keying Sets')}
      testId="scene-keying-sets"
      actions={
        <IconButton
          icon={PlusIcon}
          label={t('newKeyingSet')}
          disabled={readOnly || info.selected.length === 0 || info.mode !== 'object'}
          onClick={async () => {
            const made = (await editor.run('keyingSet.new', {})) as { name?: string } | undefined
            if (made?.name) await editor.run('keyingSet.addPaths', { name: made.name })
          }}
          data-testid="scene-new-keying-set"
        />
      }
    >
      {sets.length ? (
        <div className="scene-anim-list">
          {sets.map((set, index) => (
            <div className="scene-anim-set" key={set.name} data-testid={`scene-keying-set-${index}`} data-active={info.animation.keyingSets.active === set.name || undefined}>
              <div className="scene-anim-item">
                <span className="scene-anim-name">{set.name}</span>
                <span className="kui-muted scene-anim-meta">{t('keyingPaths', { count: set.paths.length })}</span>
                <IconButton icon={TrashIcon} label={p('Remove Keying Set')} disabled={readOnly} onClick={() => run('keyingSet.remove', { name: set.name })} />
              </div>
              {set.paths.map((path, at) => (
                <div className="scene-anim-path" key={`${path.target}-${path.property}-${path.path}`}>
                  <span>
                    {path.name} · {t(`prop_${path.property}`)}
                    {path.path ? ` (${path.path})` : ''}
                  </span>
                  <IconButton icon={TrashIcon} label={t('removePath')} disabled={readOnly} onClick={() => run('keyingSet.removePath', { name: set.name, index: at })} />
                </div>
              ))}
            </div>
          ))}
        </div>
      ) : (
        <p className="kui-muted scene-empty">{t('noKeyingSets')}</p>
      )}
    </PanelSection>
  )
}

/** The keying set Insert Keyframe uses, as the timeline offers it. */
export function KeyingSetChoice({ info }: { info: SceneInfo }) {
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const { builtins, sets, active } = info.animation.keyingSets
  return (
    <Choice
      label={t('keyingSet')}
      quiet
      value={active ?? ''}
      disabled={readOnly}
      choices={[['', t('defaultKeyingSet')], ...builtins.map((name) => [name, t(`ks_${name}`)] as const), ...sets.map((set) => [set.name, set.name] as const)]}
      onChange={(name) => run('keyingSet.setActive', { name: name || null })}
      testId="scene-keying-set"
    />
  )
}
