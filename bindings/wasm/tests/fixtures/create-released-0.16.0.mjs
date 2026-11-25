// Run once with the published Scene 0.16.0 package directory as the only argument.
// The resulting fixture records the previous engine's camera and particle settings.
import assert from 'node:assert/strict'
import { readFile, writeFile } from 'node:fs/promises'
import { resolve, join } from 'node:path'
import { pathToFileURL } from 'node:url'

if (!process.argv[2]) throw new Error('Pass the published Scene 0.16.0 package directory')
const directory = resolve(process.argv[2])
const manifest = JSON.parse(await readFile(join(directory, 'kit.json'), 'utf8'))
assert.equal(manifest.version, '0.16.0')
const { default: init, SceneDocument } = await import(pathToFileURL(join(directory, 'engine/kition_scene_kit.js')))
await init({ module_or_path: await readFile(join(directory, 'engine/kition_scene_kit_bg.wasm')) })
const doc = SceneDocument.create('Released.kscene', 640, 480)
try {
  const run = (command, params = {}) => JSON.parse(doc.execute(command, JSON.stringify(params)))
  run('render.set', { width: 200, height: 320 })
  run('camera.set', { object: 'Camera', lens: 43, sensor: 32, clipStart: 0.2, clipEnd: 120 })
  run('object.addCamera', { name: 'Ortho', location: [4, -6, 3] })
  run('camera.set', { object: 'Ortho', orthographic: true, orthoScale: 7 })
  run('object.lookAt', { object: 'Ortho', target: [0, 0, 0] })
  run('particle.add', { object: 'Cube', kind: 'emitter', name: 'Flight', count: 6, frameStart: 1,
    frameEnd: 1, lifetime: 60, seed: 7, normalVelocity: 1, objectVelocity: [0.2, -0.1, 0.3], gravity: 0 })
  run('object.addEmpty', { name: 'Target', location: [4, 1, 2] })
  run('select.objects', { objects: 'Cube' })
  run('frame.set', { frame: 12 })
  await writeFile(new URL('released-0.16.0.kscene', import.meta.url), doc.save())
  console.log(doc.inspect())
} finally {
  doc.free()
}
