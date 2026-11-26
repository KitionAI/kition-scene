import { useRef, type PointerEvent as ReactPointerEvent, type ReactNode } from 'react'
import { CameraIcon, Grid2x2Icon, HandIcon, ZoomInIcon } from '@kition/tessera/icons/react'
import type { Vec3 } from './gl/camera'
import type { AxisView } from './navigation'

/** The gizmo's size in CSS pixels (Blender's navigation gizmo is 80). */
const SIZE = 80
const CENTRE = SIZE / 2
/** How far from the centre an axis's ball sits. */
const REACH = 27
/** A drag starts after this many pixels; less is a click. */
const DRAG = 3

/** The six balls: the world axis each stands for, and the view a click on it gives. */
const BALLS: Array<{ axis: 0 | 1 | 2; sign: 1 | -1; view: AxisView; label: string }> = [
  { axis: 0, sign: 1, view: 'right', label: 'X' },
  { axis: 1, sign: 1, view: 'back', label: 'Y' },
  { axis: 2, sign: 1, view: 'top', label: 'Z' },
  { axis: 0, sign: -1, view: 'left', label: '' },
  { axis: 1, sign: -1, view: 'front', label: '' },
  { axis: 2, sign: -1, view: 'bottom', label: '' },
]

const AXIS_CLASS = ['x', 'y', 'z']

export type NavDrag = 'orbit' | 'zoom' | 'pan'

/**
 * The navigation gizmo at the viewport's top right (Blender's): the world axes as the view shows
 * them, a solid ball with its letter at each positive end and a ring at each negative end, the
 * nearer drawn over the farther. A click on a ball looks along that axis (again: from the other
 * side); a drag anywhere on it orbits. Below it: zoom and pan by dragging, the camera view, and
 * perspective / orthographic. Drawn here (SVG, the theme's axis colours), not by the engine.
 */
export function NavGizmo({
  basis,
  onAxis,
  onDrag,
  camera,
  orthographic,
  cameraEnabled,
  onCamera,
  onPerspective,
  label,
}: {
  /** The view's right, up and back directions in the world. */
  basis: [Vec3, Vec3, Vec3]
  onAxis: (view: AxisView) => void
  /** A drag on the gizmo or a button: `phase` begin, then steps of CSS pixels, then end. */
  onDrag: (kind: NavDrag, phase: 'begin' | 'move' | 'end', dx: number, dy: number) => void
  camera: boolean
  orthographic: boolean
  cameraEnabled: boolean
  onCamera: () => void
  onPerspective: () => void
  label: (key: string) => string
}) {
  const [right, up, back] = basis
  const balls = BALLS.map((ball) => {
    const x = CENTRE + REACH * ball.sign * right[ball.axis]
    const y = CENTRE - REACH * ball.sign * up[ball.axis]
    return { ...ball, x, y, depth: ball.sign * back[ball.axis] }
  }).sort((a, b) => a.depth - b.depth)
  return (
    <div className="scene-nav" data-testid="scene-nav" role="toolbar" aria-label={label('navigateGizmo')}>
      <Dragger kind="orbit" onDrag={onDrag} onClick={(target) => {
        const view = target?.closest('[data-view]')?.getAttribute('data-view') as AxisView | null
        if (view) onAxis(view)
      }}>
        <svg className="scene-gizmo" width={SIZE} height={SIZE} viewBox={`0 0 ${SIZE} ${SIZE}`} data-testid="scene-gizmo" role="img" aria-label={label('navigateGizmo')}>
          <title>{label('navigateGizmo')}</title>
          <circle className="scene-gizmo-disc" cx={CENTRE} cy={CENTRE} r={CENTRE - 1} />
          {balls.map((ball) => (
            <g key={`${ball.view}`} className={`scene-gizmo-ball scene-gizmo-${AXIS_CLASS[ball.axis]}`} data-view={ball.view} data-depth={ball.depth.toFixed(3)}>
              {ball.sign > 0 ? <line className="scene-gizmo-arm" x1={CENTRE} y1={CENTRE} x2={ball.x} y2={ball.y} /> : null}
              <circle className={ball.sign > 0 ? 'scene-gizmo-head' : 'scene-gizmo-tail'} cx={ball.x} cy={ball.y} r={ball.sign > 0 ? 8 : 7} data-testid={`scene-nav-view.${ball.view}`} />
              {ball.label ? (
                <text className="scene-gizmo-letter" x={ball.x} y={ball.y} dy="0.35em" textAnchor="middle">
                  {ball.label}
                </text>
              ) : null}
            </g>
          ))}
        </svg>
      </Dragger>
      <div className="scene-nav-buttons">
        <Dragger kind="zoom" onDrag={onDrag} title={label('navigateZoom')} testId="scene-nav-zoom">
          <ZoomInIcon size={16} aria-hidden />
        </Dragger>
        <Dragger kind="pan" onDrag={onDrag} title={label('navigatePan')} testId="scene-nav-pan">
          <HandIcon size={16} aria-hidden />
        </Dragger>
        <button type="button" className="scene-nav-round" title={label('viewCamera')} aria-label={label('viewCamera')} aria-pressed={camera} disabled={!cameraEnabled} onClick={onCamera} data-testid="scene-nav-view.camera">
          <CameraIcon size={16} aria-hidden />
        </button>
        <button type="button" className="scene-nav-round" title={label('navigatePerspective')} aria-label={label('navigatePerspective')} aria-pressed={orthographic} onClick={onPerspective} data-testid="scene-nav-view.perspective">
          <Grid2x2Icon size={16} aria-hidden />
        </button>
      </div>
    </div>
  )
}

/** Something to drag (the gizmo, the zoom and pan buttons): a press with no drag is a click. */
function Dragger({
  kind,
  onDrag,
  onClick,
  title,
  testId,
  children,
}: {
  kind: NavDrag
  onDrag: (kind: NavDrag, phase: 'begin' | 'move' | 'end', dx: number, dy: number) => void
  onClick?: (target: Element | null) => void
  title?: string
  testId?: string
  children: ReactNode
}) {
  const press = useRef<{ id: number; x: number; y: number; dragging: boolean; target: Element | null } | null>(null)
  const down = (event: ReactPointerEvent<HTMLElement>) => {
    if (event.button !== 0) return
    event.preventDefault()
    event.stopPropagation()
    press.current = { id: event.pointerId, x: event.clientX, y: event.clientY, dragging: false, target: event.target instanceof Element ? event.target : null }
    event.currentTarget.setPointerCapture(event.pointerId)
  }
  const move = (event: ReactPointerEvent<HTMLElement>) => {
    const held = press.current
    if (!held || held.id !== event.pointerId) return
    const [dx, dy] = [event.clientX - held.x, event.clientY - held.y]
    if (!held.dragging) {
      if (Math.hypot(dx, dy) < DRAG) return
      held.dragging = true
      onDrag(kind, 'begin', 0, 0)
    }
    held.x = event.clientX
    held.y = event.clientY
    onDrag(kind, 'move', dx, dy)
  }
  const up = (event: ReactPointerEvent<HTMLElement>) => {
    const held = press.current
    if (!held || held.id !== event.pointerId) return
    press.current = null
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId)
    if (held.dragging) onDrag(kind, 'end', 0, 0)
    else onClick?.(held.target)
  }
  const cancel = (event: ReactPointerEvent<HTMLElement>) => {
    const held = press.current
    if (!held || held.id !== event.pointerId) return
    press.current = null
    if (held.dragging) onDrag(kind, 'end', 0, 0)
  }
  return (
    <div
      className={kind === 'orbit' ? 'scene-gizmo-area' : 'scene-nav-round'}
      role={kind === 'orbit' ? undefined : 'button'}
      title={title}
      aria-label={title}
      onPointerDown={down}
      onPointerMove={move}
      onPointerUp={up}
      onPointerCancel={cancel}
      data-testid={testId}
      data-drag={kind}
    >
      {children}
    </div>
  )
}
