/**
 * Changes the editor makes of several commands that are one undo step (plan scene-usable-by-hand,
 * phase 8): a batch of the object model's `command` operations is one step of the person's, named
 * by its first command. Auto-keying a field's change, a material copied for one object, a new
 * material given to one object.
 */
import type { useKitEditor } from '@kition/sdk/editor'

type Editor = ReturnType<typeof useKitEditor>
type Step = { command: string; params: Record<string, unknown> }

/** Runs `steps` as one undo step; one step alone runs as itself. A refusal is reported, and changes nothing. */
export async function runAsOne(editor: Editor, title: string, steps: Step[]) {
  if (steps.length === 1 && steps[0]) {
    await editor.run(steps[0].command, steps[0].params)
    return
  }
  try {
    await editor.session.model.batch({ ops: steps.map((step) => ({ op: 'command', id: step.command, params: step.params })) })
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    editor.report(title, new Error(message))
  }
}

/**
 * `command` and, with auto-keying on, keys of `properties` on `objects` at the current frame,
 * in the same step.
 */
export function runKeyed(editor: Editor, autoKey: boolean, command: string, params: Record<string, unknown>, objects: number[], properties: string[]) {
  const steps: Step[] = [{ command, params }]
  if (autoKey && objects.length && properties.length) steps.push({ command: 'animation.insertKey', params: { objects, properties } })
  return runAsOne(editor, command, steps)
}

/** `name`, or `name.001`, `name.002`, … when one of `taken` has it (the engine names a new material the same way). */
export function uniqueName(name: string, taken: readonly string[]) {
  const base = name.trim() || 'Material'
  if (!taken.includes(base)) return base
  const match = /^(.*)\.(\d{3})$/.exec(base)
  const stem = match ? match[1] : base
  for (let number = 1; number < 100000; number += 1) {
    const candidate = `${stem}.${String(number).padStart(3, '0')}`
    if (!taken.includes(candidate)) return candidate
  }
  return stem
}
