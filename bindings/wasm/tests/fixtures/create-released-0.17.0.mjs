// Run once with the published Scene 0.17.0 package directory as the only argument.
// The fixture preserves an existing curve node tree and independent cloth/soft-body settings.
import assert from 'node:assert/strict'
import { readFile, writeFile } from 'node:fs/promises'
import { resolve, join } from 'node:path'
import { pathToFileURL } from 'node:url'

if (!process.argv[2]) throw new Error('Pass the published Scene 0.17.0 package directory')
const directory = resolve(process.argv[2])
const manifest = JSON.parse(await readFile(join(directory, 'kit.json'), 'utf8'))
assert.equal(manifest.version, '0.17.0')
const { default: init, SceneDocument } = await import(pathToFileURL(join(directory, 'engine/kition_scene_kit.js')))
await init({ module_or_path: await readFile(join(directory, 'engine/kition_scene_kit_bg.wasm')) })
const doc = SceneDocument.create('Released.kscene', 640, 480)
try {
  const run = (command, params = {}) => JSON.parse(doc.execute(command, JSON.stringify(params)))
  const group = run('node.newGroup', { object: 'Cube', name: 'ReleasedCurves' })
  const curve = run('node.addNode', { group: 'ReleasedCurves', kind: 'curveBezierSegment', values: {
    Resolution: 8, Start: [-1, 0, 0], 'Start Handle': [-0.6, 1, 0],
    'End Handle': [0.5, 1, 0.4], End: [1, 0, 0.4],
  } })
  run('node.link', { group: 'ReleasedCurves', from: curve.node, fromSocket: 'Curve',
    to: group.output, toSocket: 'Geometry' })
  run('physics.world.set', { gravity: [0, 0, -9.81], frameStart: 1, frameEnd: 12,
    substeps: 2, iterations: 10 })
  run('object.addGrid', { name: 'Cloth', xSubdivisions: 2, ySubdivisions: 2, size: 2, location: [3, 0, 3] })
  run('physics.cloth.add', { objects: 'Cloth', mass: 0.3, airDamping: 1, quality: 5 })
  run('object.addCube', { name: 'Soft', size: 1, location: [0, 3, 3] })
  run('physics.softBody.add', { objects: 'Soft', mass: 0.5, friction: 0.2, goal: false,
    frameStart: 1, frameEnd: 12 })
  run('physics.field.new', { name: 'Wind', type: 'wind', strength: 2, flow: 0 })
  run('select.objects', { objects: 'Cube' })
  run('frame.set', { frame: 7 })
  await writeFile(new URL('released-0.17.0.kscene', import.meta.url), doc.save())
  console.log(doc.inspect())
} finally {
  doc.free()
}
