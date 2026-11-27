import { ContextMenu, MenuItem, MenuSeparator, SubMenu } from '@kition/sdk/editor'

/** A row of a menu at the pointer (with `children`, a submenu), or a line between rows. */
export type PointerMenuRow =
  | { id: string; label: string; onSelect: () => void; disabled?: boolean; shortcut?: string; checked?: boolean; children?: PointerMenuRow[] }
  | 'separator'

/** A menu that opens where the pointer is (Blender's X, M, P, Shift+A, Shift+S, the right button, and the pie keys' menus). */
export type PointerMenuState = { at: { x: number; y: number }; title: string; rows: PointerMenuRow[]; testId: string; kind?: string }

/** Where the pointer last was on the page, in client pixels: a key's menu opens there. */
const pointer = { x: 0, y: 0, seen: false }

if (typeof window !== 'undefined') {
  window.addEventListener(
    'pointermove',
    (event) => {
      pointer.x = event.clientX
      pointer.y = event.clientY
      pointer.seen = true
    },
    { capture: true, passive: true },
  )
}

/** Where a menu a key opens goes: the pointer, or the middle of `fallback` (the viewport) before the pointer moved. */
export function pointerPoint(fallback?: Element | null) {
  if (pointer.seen || !fallback) return { x: pointer.x, y: pointer.y }
  const box = fallback.getBoundingClientRect()
  return { x: box.left + box.width / 2, y: box.top + box.height / 2 }
}

/**
 * A menu's rows: items, separators and submenus (opened on hover, or the right arrow). `close`
 * runs before a row's action; test ids are `${prefix}-${row.id}`.
 */
export function MenuRows({ rows, prefix, close }: { rows: readonly PointerMenuRow[]; prefix: string; close?: () => void }) {
  return (
    <>
      {rows.map((row, index) =>
        row === 'separator' ? (
          <MenuSeparator key={`separator-${index}`} />
        ) : row.children ? (
          <SubMenu key={row.id} label={row.label} path={`${prefix}/${row.id}`} uiId={`control:${prefix}-${row.id}`} testId={`${prefix}-${row.id}`}>
            <MenuRows rows={row.children} prefix={prefix} close={close} />
          </SubMenu>
        ) : (
          <MenuItem
            key={row.id}
            label={row.label}
            shortcut={row.shortcut}
            disabled={row.disabled}
            checked={row.checked}
            onSelect={() => {
              close?.()
              row.onSelect()
            }}
            testId={`${prefix}-${row.id}`}
          />
        ),
      )}
    </>
  )
}

/**
 * A menu at the pointer: the frame's context menu (kept in the window, its first row focused, the
 * arrow keys and Enter move and choose, Escape or a press outside closes it), with a title row.
 * Each row runs its action and closes the menu.
 */
export function PointerMenu({ menu, onClose }: { menu: PointerMenuState | null; onClose: () => void }) {
  return (
    <ContextMenu at={menu?.at ?? null} onClose={onClose} label={menu?.title} testId={menu?.testId}>
      {menu ? (
        <>
          <div className="scene-pointer-menu-title" role="presentation" data-kind={menu.kind}>
            {menu.title}
          </div>
          <MenuRows rows={menu.rows} prefix="scene-popup" close={onClose} />
        </>
      ) : (
        <span />
      )}
    </ContextMenu>
  )
}
