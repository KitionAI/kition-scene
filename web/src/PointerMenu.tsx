import { ContextMenu, MenuItem, MenuSeparator } from '@kition/sdk/editor'

/** A row of a menu at the pointer, or a line between rows. */
export type PointerMenuRow = { id: string; label: string; onSelect: () => void; disabled?: boolean; shortcut?: string } | 'separator'

/** A menu that opens where the pointer is (Blender's X, M, P; later Shift+A, Shift+S and the pie keys' menus). */
export type PointerMenuState = { at: { x: number; y: number }; title: string; rows: PointerMenuRow[]; testId: string }

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
 * A menu at the pointer: the frame's context menu (kept in the window, its first row focused, the
 * arrow keys and Enter move and choose, Escape or a press outside closes it), with a title row.
 * Each row runs its action and closes the menu.
 */
export function PointerMenu({ menu, onClose }: { menu: PointerMenuState | null; onClose: () => void }) {
  return (
    <ContextMenu at={menu?.at ?? null} onClose={onClose} label={menu?.title} testId={menu?.testId}>
      {menu ? (
        <>
          <div className="scene-pointer-menu-title" role="presentation">
            {menu.title}
          </div>
          {menu.rows.map((row, index) =>
            row === 'separator' ? (
              <MenuSeparator key={`separator-${index}`} />
            ) : (
              <MenuItem
                key={row.id}
                label={row.label}
                shortcut={row.shortcut}
                disabled={row.disabled}
                onSelect={() => {
                  onClose()
                  row.onSelect()
                }}
                testId={`scene-popup-${row.id}`}
              />
            ),
          )}
        </>
      ) : (
        <span />
      )}
    </ContextMenu>
  )
}
