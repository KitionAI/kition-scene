/**
 * Whether the left button is held on the viewport for the engine's tools (a box, a move, a stroke;
 * not navigation), and whether that press drives a transform (Move, Rotate or Scale in a mode they
 * work in). The viewport sets them; the editor's keys read them: a key pressed meanwhile runs no
 * shortcut, so X during a move deletes nothing; during a transform the keys go to it (X there
 * keeps the move to the X axis).
 */
export const toolGesture = { held: false, transform: false }

/** The modes the transform works in (sculpt, weight paint, Particle Edit and text editing have their own uses of the pointer and keys). */
export const TRANSFORM_MODES = ['object', 'edit', 'armatureEdit', 'pose', 'curveEdit', 'latticeEdit']

/** The tools whose press starts a transform. */
export const TRANSFORM_TOOLS = ['move', 'rotate', 'scale']
