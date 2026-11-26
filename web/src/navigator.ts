/**
 * The viewport's navigation keys and menu, for the editor's key handler: the keypad's views and
 * steps (Blender's), Alt with the number row for keyboards without a keypad, Ctrl/Cmd+= and − to
 * zoom, Home and `.` with a smooth change of view. The viewport (`Viewport.tsx`) gives it its hooks:
 * what the view is, how to show another, how to run a view command.
 */
import type { OrbitView } from './gl/camera'
import { AXIS_VIEWS, ZOOM_STEP, axisOf, axisView, oppositeView, orbitStep, panStep, type AxisView, type Direction } from './navigation'

export type NavigatorHooks = {
  /** The orbit view now (as drawn, or as the engine keeps it); null before the document shows. */
  current: () => OrbitView | null
  /** Looking through the scene's camera. */
  throughCamera: () => boolean
  /** Shows `view` (smoothly when asked), and gives the engine the view it ends at. */
  show: (view: OrbitView, smooth: boolean) => void
  /** Runs a view command; those that move the view (Home, `.`) change to it smoothly. */
  command: (id: string, smooth: boolean) => void
  /** Zooms the camera's frame by `factor` (above 1: larger) and moves it by half viewports. */
  cameraFrame: (factor: number, offset: [number, number]) => void
  /** Puts the camera's frame back to fill the viewport. */
  resetCameraFrame: () => void
  /** Zooms the orbit view by `factor` (below 1: closer), within the zoom's limits. */
  zoom: (factor: number) => void
  /** The viewport's height in CSS pixels. */
  height: () => number
  /** Moves the scene's camera to where the eye is, and looks through it. */
  alignCamera: () => void
}

const KEYPAD_DIRECTIONS: Record<string, Direction> = { Numpad4: 'left', Numpad6: 'right', Numpad8: 'up', Numpad2: 'down' }

type Key = { code: string; key: string; ctrlKey: boolean; metaKey: boolean; altKey: boolean; shiftKey: boolean }

class Navigator {
  hooks: NavigatorHooks | null = null

  /** Looks along `axis`; a second time from the other side (the navigation gizmo's balls). */
  toggleAxis(axis: AxisView) {
    const view = this.hooks?.current()
    if (!view || !this.hooks) return
    const now = this.hooks.throughCamera() ? null : axisOf(view)
    this.hooks.show(axisView(view, now === axis && !view.perspective ? AXIS_VIEWS[axis].opposite : axis), true)
  }

  /** Looks along `axis`, orthographically. */
  axis(axis: AxisView) {
    const view = this.hooks?.current()
    if (view) this.hooks?.show(axisView(view, axis), true)
  }

  /**
   * A navigation key: answers whether it was taken. The keypad's keys are read by their place
   * (`code`), so they answer with Num Lock off too; Alt does not change them (Blender lets Alt
   * navigate during a transform).
   */
  key(event: Key): boolean {
    const hooks = this.hooks
    if (!hooks) return false
    const command = event.ctrlKey || event.metaKey
    const camera = hooks.throughCamera()
    const view = hooks.current()
    const height = hooks.height()
    switch (event.code) {
      case 'Numpad1':
      case 'Numpad3':
      case 'Numpad7': {
        if (event.shiftKey) return false
        const views: Record<string, [AxisView, AxisView]> = { Numpad1: ['front', 'back'], Numpad3: ['right', 'left'], Numpad7: ['top', 'bottom'] }
        const [plain, other] = views[event.code]
        this.axis(command ? other : plain)
        return true
      }
      case 'Numpad9':
        if (event.shiftKey || command) return false
        if (view) hooks.show(oppositeView(view), true)
        return true
      case 'Numpad5':
        if (event.shiftKey || command) return false
        hooks.command('view.perspective', false)
        return true
      case 'Numpad0':
        if (event.shiftKey) return false
        if (command && event.altKey) hooks.alignCamera()
        else if (!command) hooks.command('view.camera', false)
        else return false
        return true
      case 'Numpad2':
      case 'Numpad4':
      case 'Numpad6':
      case 'Numpad8': {
        if (event.shiftKey) return false
        const direction = KEYPAD_DIRECTIONS[event.code]
        if (camera) {
          // In the camera view, a pan step moves the camera's frame; an orbit step does nothing.
          if (command) {
            const [x, y] = direction === 'left' ? [1, 0] : direction === 'right' ? [-1, 0] : direction === 'up' ? [0, -1] : [0, 1]
            hooks.cameraFrame(1, [(x * 64) / Math.max(1, height), (y * 50) / Math.max(1, height)])
          }
          return true
        }
        if (view) hooks.show(command ? panStep(view, direction, height) : orbitStep(view, direction), !command)
        return true
      }
      case 'NumpadAdd':
      case 'NumpadSubtract': {
        if (event.shiftKey || command) return false
        const closer = event.code === 'NumpadAdd'
        if (camera) hooks.cameraFrame(closer ? ZOOM_STEP : 1 / ZOOM_STEP, [0, 0])
        else hooks.zoom(closer ? 1 / ZOOM_STEP : ZOOM_STEP)
        return true
      }
      case 'Equal':
      case 'Minus': {
        // Ctrl/Cmd+= and Ctrl/Cmd+−: zoom (the 2D canvas's zoom keys do nothing for a 3D view).
        if (!command || event.altKey) return false
        const closer = event.code === 'Equal'
        if (camera) hooks.cameraFrame(closer ? ZOOM_STEP : 1 / ZOOM_STEP, [0, 0])
        else hooks.zoom(closer ? 1 / ZOOM_STEP : ZOOM_STEP)
        return true
      }
      case 'Digit1':
      case 'Digit3':
      case 'Digit7': {
        // Without a keypad: Alt+1 / 3 / 7, and with Ctrl/Cmd the views from the other side.
        if (!event.altKey || event.shiftKey) return false
        const views: Record<string, [AxisView, AxisView]> = { Digit1: ['front', 'back'], Digit3: ['right', 'left'], Digit7: ['top', 'bottom'] }
        const [plain, other] = views[event.code]
        this.axis(command ? other : plain)
        return true
      }
      case 'Home':
        if (event.shiftKey || command || event.altKey) return false
        if (camera) hooks.resetCameraFrame()
        else hooks.command('view.frameAll', true)
        return true
      // The keypad's point (the main keyboard's is the pivot point menu).
      case 'NumpadDecimal':
        if (event.shiftKey || command || event.altKey) return false
        hooks.command('view.frameSelected', true)
        return true
      default:
        return false
    }
  }
}

/** The page's one navigator (the viewport sets its hooks). */
export const viewNavigator = new Navigator()

/** The rows of the View menu that `` ` `` opens at the pointer (Blender's view pie, as a list). */
export function viewMenuRows(t: (key: string) => string) {
  const axis = (view: AxisView, label: string) => ({ id: `view.${view}`, label: t(label), onSelect: () => viewNavigator.axis(view) })
  return [
    axis('left', 'viewLeft'),
    axis('right', 'viewRight'),
    axis('bottom', 'viewBottom'),
    axis('top', 'viewTop'),
    axis('front', 'viewFront'),
    axis('back', 'viewBack'),
    'separator' as const,
    { id: 'view.camera', label: t('viewCamera'), onSelect: () => viewNavigator.hooks?.command('view.camera', false) },
    { id: 'view.frameSelected', label: t('viewSelected'), onSelect: () => viewNavigator.hooks?.command('view.frameSelected', true) },
    { id: 'view.frameAll', label: t('viewAll'), onSelect: () => viewNavigator.hooks?.command('view.frameAll', true) },
  ]
}
