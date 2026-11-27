import { useEffect, useState } from 'react'
import type { SceneInfo } from './scene'

type Held = 'none' | 'shift' | 'ctrl'

/** Which modifier is held now (Shift before Ctrl/Cmd), following the keyboard. */
function useHeld(): Held {
  const [held, setHeld] = useState<Held>('none')
  useEffect(() => {
    const read = (event: KeyboardEvent) => setHeld(event.shiftKey ? 'shift' : event.ctrlKey || event.metaKey ? 'ctrl' : 'none')
    const clear = () => setHeld('none')
    window.addEventListener('keydown', read, true)
    window.addEventListener('keyup', read, true)
    window.addEventListener('blur', clear)
    return () => {
      window.removeEventListener('keydown', read, true)
      window.removeEventListener('keyup', read, true)
      window.removeEventListener('blur', clear)
    }
  }, [])
  return held
}

/** The three buttons' meaning and a key or two, by mode, tool and modifier: strings keys, with their keys shown. */
export function statusHints(info: SceneInfo, tool: string, held: Held): Array<[string, string]> {
  const paint = info.mode === 'sculpt' || info.mode === 'weightPaint' || info.mode === 'particleEdit'
  if (held === 'shift') return paint ? [['LMB', 'hintStrokeInvert'], ['MMB', 'hintPan'], ['RMB', 'hintCursor']] : [['LMB', 'hintSelectToggle'], ['MMB', 'hintPan'], ['RMB', 'hintCursor']]
  if (held === 'ctrl') return paint ? [['LMB', 'hintStrokeInvert'], ['MMB', 'hintZoom']] : [['LMB', 'hintDeselect'], ['MMB', 'hintZoom']]
  if (paint) return [['LMB', 'hintStroke'], ['MMB', 'hintRotate'], ['RMB', 'hintOptions'], ['F3', 'hintSearch']]
  const left = tool === 'move' || tool === 'rotate' || tool === 'scale' ? 'hintHandle' : tool === 'loopCut' ? 'hintLoopCut' : tool === 'knife' ? 'hintKnife' : tool === 'slide' ? 'hintSlide' : 'hintSelect'
  const keys: Array<[string, string]> = [['LMB', left], ['MMB', 'hintRotate'], ['RMB', 'hintOptions']]
  if (info.mode !== 'textEdit') keys.push(['G R S', 'hintTransform'])
  if (info.mode === 'object') keys.push(['⇧A', 'hintAdd'])
  keys.push(['F3', 'hintSearch'])
  return keys
}

/** The status bar's hints: what the mouse buttons and the main keys do now (Blender's left end of the status bar). */
export function StatusHints({ info, tool, t }: { info: SceneInfo; tool: string; t: (key: string) => string }) {
  const held = useHeld()
  const hints = statusHints(info, tool, held)
  return (
    <span className="scene-hints" data-testid="scene-status-hints" data-held={held}>
      {hints.map(([key, hint]) => (
        <span key={hint}>
          <kbd>{key}</kbd>
          {t(hint)}
        </span>
      ))}
    </span>
  )
}
