import { useEffect, useRef, useState } from 'react'

/** One item of a pie: where it sits round the middle. */
export type PieItem = { id: string; label: string; side: 'left' | 'right' | 'bottom' | 'top'; active?: boolean; onSelect: () => void }

/** A pie open at a point; `key` is the code of the key that opened it (its release may choose). */
export type PieState = { at: { x: number; y: number }; title: string; items: PieItem[]; key: string; testId: string }

/** How far the pointer goes from the middle before a direction counts, and the items' distance. */
const DEAD_ZONE = 12
const RADIUS = 100

/** The item the direction from the middle to (dx, dy) points at (screen axes: y down). */
export function pieDirection(dx: number, dy: number): PieItem['side'] | null {
  if (Math.hypot(dx, dy) < DEAD_ZONE) return null
  if (Math.abs(dx) >= Math.abs(dy)) return dx < 0 ? 'left' : 'right'
  return dy < 0 ? 'top' : 'bottom'
}

const OFFSET: Record<PieItem['side'], [number, number]> = { left: [-RADIUS, 0], right: [RADIUS, 0], top: [0, -RADIUS * 0.8], bottom: [0, RADIUS * 0.8] }

/**
 * A pie menu (Blender's Z): items round the point it opened at; the direction of the pointer from
 * the middle lights one up. A click on an item chooses it; holding the key, moving toward an item
 * and letting go chooses that one; letting go without moving leaves the pie open. Escape, the
 * right button or a click elsewhere closes it.
 */
export function PieMenu({ pie, onClose }: { pie: PieState | null; onClose: () => void }) {
  const [toward, setToward] = useState<PieItem['side'] | null>(null)
  const latest = useRef({ pie, toward, onClose })
  latest.current = { pie, toward, onClose }
  useEffect(() => {
    setToward(null)
    if (!pie) return undefined
    const choose = (side: PieItem['side'] | null) => {
      const item = side ? latest.current.pie?.items.find((entry) => entry.side === side) : null
      if (!item) return false
      latest.current.onClose()
      item.onSelect()
      return true
    }
    const move = (event: PointerEvent) => setToward(pieDirection(event.clientX - pie.at.x, event.clientY - pie.at.y))
    const down = (event: PointerEvent) => {
      if (event.target instanceof Element && event.target.closest('[data-pie-item]')) return
      event.preventDefault()
      event.stopPropagation()
      // A click away from the middle chooses the item it points at, as Blender's does; the right button closes.
      if (event.button !== 0 || !choose(pieDirection(event.clientX - pie.at.x, event.clientY - pie.at.y))) latest.current.onClose()
    }
    const key = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault()
        event.stopPropagation()
        latest.current.onClose()
      }
    }
    const up = (event: KeyboardEvent) => {
      if (event.code !== pie.key) return
      choose(latest.current.toward)
    }
    const menu = (event: MouseEvent) => event.preventDefault()
    window.addEventListener('pointermove', move, true)
    window.addEventListener('pointerdown', down, true)
    window.addEventListener('keydown', key, true)
    window.addEventListener('keyup', up, true)
    window.addEventListener('contextmenu', menu, true)
    return () => {
      window.removeEventListener('pointermove', move, true)
      window.removeEventListener('pointerdown', down, true)
      window.removeEventListener('keydown', key, true)
      window.removeEventListener('keyup', up, true)
      window.removeEventListener('contextmenu', menu, true)
    }
  }, [pie])
  if (!pie) return null
  const angle = toward === 'left' ? 180 : toward === 'right' ? 0 : toward === 'top' ? -90 : 90
  return (
    <div className="scene-pie" role="menu" aria-label={pie.title} data-testid={pie.testId} data-toward={toward ?? undefined} style={{ left: pie.at.x, top: pie.at.y }}>
      <div className="scene-pie-centre" aria-hidden="true">
        {toward ? <span className="scene-pie-notch" style={{ transform: `rotate(${angle}deg)` }} /> : null}
      </div>
      {pie.items.map((item) => (
        <button
          key={item.id}
          type="button"
          role="menuitemradio"
          aria-checked={item.active === true}
          className="scene-pie-item"
          data-pie-item
          data-side={item.side}
          data-lit={toward === item.side || undefined}
          style={{ transform: `translate(calc(${OFFSET[item.side][0]}px - 50%), calc(${OFFSET[item.side][1]}px - 50%))` }}
          onClick={() => {
            onClose()
            item.onSelect()
          }}
          data-testid={`${pie.testId}-${item.id}`}
        >
          {item.label}
        </button>
      ))}
    </div>
  )
}
