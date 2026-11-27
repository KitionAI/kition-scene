import { Checkbox, Choice, IconButton, MenuButton, Popover, PopoverContent, PopoverTrigger, useKitEditor, type IconComponent } from '@kition/sdk/editor'
import { CircleDashedIcon, CircleIcon, GlobeIcon, LayersIcon, SunIcon } from '@kition/tessera/icons/react'
import { MenuRows } from './PointerMenu'
import { buildRows, headerMenus, MODE_LABELS, modesFor, type MenuContext, type SceneMode } from './menus'
import { OrientationControl, PivotControl, SnapControl } from './TransformControls'
import { TRANSFORM_MODES } from './gesture'
import { focusEditor } from './NumberBox'
import type { SceneInfo } from './scene'

/** Edit mode's select-mode icons, drawn here: a square's corners, one bold side, the square filled. */
function selectIcon(kind: 'vertex' | 'edge' | 'face'): IconComponent {
  return function SelectModeIcon({ size = 16, className, style }) {
    return (
      <svg width={size} height={size} viewBox="0 0 16 16" className={className} style={style} aria-hidden="true">
        <rect x="3" y="3" width="10" height="10" fill={kind === 'face' ? 'currentColor' : 'none'} fillOpacity={kind === 'face' ? 0.55 : 0} stroke="currentColor" strokeWidth="1" strokeOpacity="0.6" />
        {kind === 'edge' ? <line x1="3" y1="13" x2="13" y2="13" stroke="currentColor" strokeWidth="2.5" /> : null}
        {kind === 'vertex' ? [3, 13].flatMap((x) => [3, 13].map((y) => <rect key={`${x}-${y}`} x={x - 1.75} y={y - 1.75} width="3.5" height="3.5" fill="currentColor" />)) : null}
      </svg>
    )
  }
}
const SELECT_ICONS = { vertex: selectIcon('vertex'), edge: selectIcon('edge'), face: selectIcon('face') }

const SHADING: Array<{ mode: SceneInfo['shading']; key: string; icon: IconComponent }> = [
  { mode: 'wireframe', key: 'wireframe', icon: CircleDashedIcon },
  { mode: 'solid', key: 'solid', icon: CircleIcon },
  { mode: 'material', key: 'materialPreview', icon: GlobeIcon },
  { mode: 'rendered', key: 'rendered', icon: SunIcon },
]

/**
 * The 3D viewport's header along its top edge (it stops before the navigation gizmo): the mode
 * drop-down, edit mode's vertex / edge / face buttons and the mode's menus on the left; the
 * transform orientation, pivot point and snapping, the overlays' popover and the four shading
 * buttons on the right. The bar itself lets the pointer through to the picture; its controls do not.
 */
export function ViewportHeader({ info, context, onMode, readOnly }: { info: SceneInfo; context: MenuContext; onMode: (mode: SceneMode) => void; readOnly: boolean }) {
  const editor = useKitEditor()
  const { t, phrase } = editor
  const run = (id: string, params: Record<string, unknown>) => {
    void editor.run(id, params)
    focusEditor()
  }
  const modes = modesFor(info)
  const menus = readOnly ? headerMenus(info.mode).filter((menu) => menu.name === 'View') : headerMenus(info.mode)
  return (
    <div className="scene-header" data-testid="scene-header" data-mode={info.mode} onPointerDown={(event) => event.target !== event.currentTarget && event.stopPropagation()}>
      <div className="scene-header-start">
        <Choice
          label={t('modeChoice')}
          quiet
          value={info.mode}
          disabled={readOnly || modes.length < 2}
          choices={(modes.includes(info.mode) ? modes : [info.mode, ...modes]).map((mode) => [mode, t(MODE_LABELS[mode])] as const)}
          onChange={(mode: SceneMode) => {
            onMode(mode)
            focusEditor()
          }}
          testId="scene-mode-select"
        />
        {info.mode === 'edit' ? (
          <span className="scene-header-group" role="group" aria-label={t('selectMode')}>
            {(['vertex', 'edge', 'face'] as const).map((kind, index) => (
              <IconButton
                key={kind}
                icon={SELECT_ICONS[kind]}
                label={`${t(`${kind}Mode`)} (${index + 1})`}
                pressed={info.select === kind}
                onClick={() => run('mesh.selectMode', { mode: kind })}
                data-testid={`scene-select-${kind}`}
              />
            ))}
          </span>
        ) : null}
        <span className="scene-header-menus" role="menubar" aria-label={t('headerMenus')}>
          {menus.map((menu) => (
            <MenuButton
              key={menu.name}
              trigger={({ open, toggle }) => (
                <button type="button" className="scene-header-menu" aria-expanded={open} aria-haspopup="menu" onClick={toggle} data-testid={`scene-header-menu-${menu.name}`}>
                  {phrase(menu.label)}
                </button>
              )}
            >
              <MenuRows rows={buildRows(menu.items, context)} prefix="scene-header" />
            </MenuButton>
          ))}
        </span>
      </div>
      <div className="scene-header-end">
        {!readOnly && TRANSFORM_MODES.includes(info.mode) ? (
          <>
            <OrientationControl info={info} />
            <PivotControl info={info} />
            <SnapControl info={info} />
          </>
        ) : null}
        {/* Phase 6 puts the X-ray toggle here, before the overlays. */}
        <Popover>
          <PopoverTrigger asChild>
            <button type="button" className="kui-icon-button scene-header-icon" aria-label={t('overlays')} title={t('overlays')} data-testid="scene-overlays">
              <LayersIcon size={15} strokeWidth={1.75} aria-hidden="true" />
            </button>
          </PopoverTrigger>
          <PopoverContent align="end" className="scene-overlays-popover">
            <div className="scene-overlays" data-testid="scene-overlays-panel">
              <Checkbox label={t('grid')} checked={info.grid} onChange={(on) => run('view.grid', { on })} testId="scene-grid" />
            </div>
          </PopoverContent>
        </Popover>
        <span className="scene-header-group" role="group" aria-label={t('shadingMode')}>
          {SHADING.map((item) => (
            <IconButton key={item.mode} icon={item.icon} label={t(item.key)} pressed={info.shading === item.mode} onClick={() => run('view.shading', { mode: item.mode })} data-testid={`scene-shading-${item.mode}`} />
          ))}
        </span>
      </div>
    </div>
  )
}
