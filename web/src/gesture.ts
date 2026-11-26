/**
 * Whether the left button is held on the viewport for the engine's tools (a box, a move, a stroke;
 * not navigation). The viewport sets it; the editor's keys read it: a key pressed meanwhile runs no
 * shortcut, so X during a move deletes nothing (Blender's X there constrains the move instead).
 */
export const toolGesture = { held: false }
