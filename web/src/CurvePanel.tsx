import { Button, Checkbox, Choice, PanelSection, Segmented, useKitEditor, useReadOnly } from '@kition/kit-editor'
import { NumberInput, Row, useRun } from './SidePanel'
import type { SceneInfo, SceneObject } from './scene'

/** Object or edit mode for the active curve or text (Tab), as one switch. */
function ModeSwitch({ object, info, edit }: { object: SceneObject; info: SceneInfo; edit: 'curveEdit' | 'textEdit' }) {
  const { t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const editing = info.mode === edit && info.editing === object.id
  return (
    <Row label={t('shapeMode')}>
      <Segmented
        label={t('shapeMode')}
        value={editing ? 'edit' : 'object'}
        disabled={readOnly || (info.mode !== 'object' && !editing)}
        segments={[
          { value: 'object', label: t('modeObject') },
          { value: 'edit', label: t('modeEdit') },
        ]}
        onChange={(value) => {
          if ((value === 'edit') !== editing) run('mode.shape', {})
        }}
      />
    </Row>
  )
}

/** A curve's sweep, fill, and resolution, and its edit mode's point actions. */
function CurveSection({ object, info }: { object: SceneObject; info: SceneInfo }) {
  const { phrase: p, t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const curve = object.curve
  if (!curve) return null
  const set = (params: Record<string, unknown>) => run('curve.set', { object: object.id, ...params })
  const flat = curve.dimensions === '2d'
  const editing = info.mode === 'curveEdit' && info.editing === object.id
  const points = curve.summary.reduce((sum, spline) => sum + spline.points, 0)
  const cyclic = curve.summary.length > 0 && curve.summary.every((spline) => spline.cyclic)
  return (
    <PanelSection title={p('Curve')} testId="scene-curve">
      <ModeSwitch object={object} info={info} edit="curveEdit" />
      <Row label={t('curveShape')}>
        <span className="scene-readout tabular-nums" data-testid="scene-curve-stats">
          {t('curveStats', { splines: curve.summary.length, points, vertices: curve.evaluated.vertices })}
        </span>
      </Row>
      <Row label={t('curveDimensions')}>
        <Segmented
          label={t('curveDimensions')}
          value={curve.dimensions}
          disabled={readOnly}
          segments={[
            { value: '2d', label: '2D' },
            { value: '3d', label: '3D' },
          ]}
          onChange={(dimensions) => set({ dimensions })}
        />
      </Row>
      <Row label={t('fillMode')}>
        <Choice
          label={t('fillMode')}
          quiet
          value={curve.fillMode}
          disabled={readOnly}
          choices={(flat ? ['both', 'front', 'back', 'none'] : ['full', 'half']).map((mode) => [mode, t(`fill_${mode}`)] as const)}
          onChange={(fillMode) => set({ fillMode })}
        />
      </Row>
      <Row label={t('bevelDepth')}>
        <NumberInput label={t('bevelDepth')} value={curve.bevelDepth} min={0} max={100} step={0.01} unit="m" onCommit={(bevelDepth) => set({ bevelDepth })} testId="scene-curve-bevel" />
      </Row>
      <Row label={t('bevelResolution')}>
        <NumberInput label={t('bevelResolution')} value={curve.bevelResolution} min={0} max={64} step={1} onCommit={(value) => set({ bevelResolution: Math.round(value) })} testId="scene-curve-bevel-resolution" />
      </Row>
      <Row label={t('extrude')}>
        <NumberInput label={t('extrude')} value={curve.extrude} min={0} max={100} step={0.01} unit="m" onCommit={(extrude) => set({ extrude })} testId="scene-curve-extrude" />
      </Row>
      <Row label={t('curveOffset')}>
        <NumberInput label={t('curveOffset')} value={curve.offset} min={-100} max={100} step={0.01} unit="m" onCommit={(offset) => set({ offset })} testId="scene-curve-offset" />
      </Row>
      <Row label={t('resolution')}>
        <NumberInput label={t('resolution')} value={curve.resolution} min={1} max={1024} step={1} onCommit={(value) => set({ resolution: Math.round(value) })} testId="scene-curve-resolution" />
      </Row>
      <Row label={t('curveOptions')}>
        <div className="scene-checks">
          <Checkbox label={t('cyclic')} checked={cyclic} disabled={readOnly} onChange={(value) => set({ cyclic: value })} testId="scene-curve-cyclic" />
          {!flat ? <Checkbox label={t('fillCaps')} checked={curve.fillCaps} disabled={readOnly} onChange={(fillCaps) => set({ fillCaps })} testId="scene-curve-caps" /> : null}
        </div>
      </Row>
      {editing ? (
        <>
          <Row label={t('handleType')}>
            <Choice
              label={t('handleType')}
              quiet
              value=""
              disabled={readOnly || info.points.length === 0}
              choices={[['', t('handleTypeChoose')], ...(['auto', 'vector', 'aligned', 'free'] as const).map((kind) => [kind, t(`handle_${kind}`)] as const)]}
              onChange={(type) => type && run('curve.setHandleType', { type })}
            />
          </Row>
          <div className="scene-bone-actions">
            <Button size="sm" disabled={readOnly || info.points.length === 0} onClick={() => run('curve.extrude', {})} data-testid="scene-curve-extrude-point">
              {p('Extrude Points')}
            </Button>
            <Button size="sm" disabled={readOnly || info.points.length < 2} onClick={() => run('curve.subdivide', {})} data-testid="scene-curve-subdivide">
              {p('Subdivide')}
            </Button>
            <Button size="sm" disabled={readOnly} onClick={() => run('curve.toggleCyclic', {})} data-testid="scene-curve-toggle-cyclic">
              {p('Toggle Cyclic')}
            </Button>
            <Button size="sm" disabled={readOnly || info.points.length === 0} onClick={() => run('curve.deletePoints', {})} data-testid="scene-curve-delete-points">
              {p('Delete Points')}
            </Button>
          </div>
        </>
      ) : (
        <ConvertButton object={object} info={info} />
      )}
    </PanelSection>
  )
}

function ConvertButton({ object, info }: { object: SceneObject; info: SceneInfo }) {
  const { phrase: p } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  return (
    <div className="scene-bone-actions">
      <Button size="sm" disabled={readOnly || info.mode !== 'object'} onClick={() => run('object.convertToMesh', { objects: [object.id] })} data-testid="scene-convert-mesh">
        {p('Convert to Mesh')}
      </Button>
    </div>
  )
}

/** A text's body, size, depth, and layout. */
function TextSection({ object, info }: { object: SceneObject; info: SceneInfo }) {
  const { phrase: p, t } = useKitEditor()
  const readOnly = useReadOnly()
  const run = useRun()
  const text = object.text
  if (!text) return null
  const set = (params: Record<string, unknown>) => run('text.set', { object: object.id, ...params })
  const editing = info.mode === 'textEdit' && info.editing === object.id
  return (
    <PanelSection title={p('Text')} testId="scene-text">
      <ModeSwitch object={object} info={info} edit="textEdit" />
      <Row label={t('textBody')}>
        <textarea
          key={text.body}
          className="kui-field scene-text-body"
          rows={Math.min(6, Math.max(2, text.body.split('\n').length))}
          defaultValue={text.body}
          disabled={readOnly || editing}
          onBlur={(event) => {
            const body = event.currentTarget.value
            if (body !== text.body) set({ body })
          }}
          data-testid="scene-text-body"
        />
      </Row>
      {editing ? <p className="kui-muted scene-text-hint">{t('textEditHint')}</p> : null}
      <Row label={t('textSize')}>
        <NumberInput label={t('textSize')} value={text.size} min={0.001} max={1000} step={0.1} onCommit={(size) => set({ size })} testId="scene-text-size" />
      </Row>
      <Row label={t('extrude')}>
        <NumberInput label={t('extrude')} value={text.extrude} min={0} max={100} step={0.01} unit="m" onCommit={(extrude) => set({ extrude })} testId="scene-text-extrude" />
      </Row>
      <Row label={t('bevelDepth')}>
        <NumberInput label={t('bevelDepth')} value={text.bevelDepth} min={0} max={100} step={0.005} unit="m" onCommit={(bevelDepth) => set({ bevelDepth })} testId="scene-text-bevel" />
      </Row>
      <Row label={t('textAlign')}>
        <Segmented
          label={t('textAlign')}
          value={text.align}
          disabled={readOnly}
          segments={(['left', 'center', 'right'] as const).map((align) => ({ value: align, label: t(`align_${align}`) }))}
          onChange={(align) => set({ align })}
        />
      </Row>
      <Row label={t('lineSpacing')}>
        <NumberInput label={t('lineSpacing')} value={text.lineSpacing} min={0} max={10} step={0.05} onCommit={(lineSpacing) => set({ lineSpacing })} testId="scene-text-line-spacing" />
      </Row>
      <Row label={t('characterSpacing')}>
        <NumberInput label={t('characterSpacing')} value={text.characterSpacing} min={0} max={10} step={0.05} onCommit={(characterSpacing) => set({ characterSpacing })} testId="scene-text-character-spacing" />
      </Row>
      <Row label={t('textOffset')}>
        <div className="scene-vector scene-vector-2">
          <NumberInput label="X" value={text.offsetX} min={-1000} max={1000} step={0.05} unit="m" onCommit={(offsetX) => set({ offsetX })} testId="scene-text-offset-x" />
          <NumberInput label="Y" value={text.offsetY} min={-1000} max={1000} step={0.05} unit="m" onCommit={(offsetY) => set({ offsetY })} testId="scene-text-offset-y" />
        </div>
      </Row>
      <Row label={t('textShear')}>
        <NumberInput label={t('textShear')} value={text.shear} min={-1} max={1} step={0.05} onCommit={(shear) => set({ shear })} testId="scene-text-shear" />
      </Row>
      {editing ? null : <ConvertButton object={object} info={info} />}
    </PanelSection>
  )
}

/** The active object's curve or text sections (nothing for other objects). */
export function ShapeSections({ object, info }: { object: SceneObject; info: SceneInfo }) {
  if (object.type === 'curve') return <CurveSection key={`curve-${object.id}`} object={object} info={info} />
  if (object.type === 'text') return <TextSection key={`text-${object.id}`} object={object} info={info} />
  return null
}
