// Run once with the published 0.15.0 package directory as the only argument.
// Keep the resulting fixture: future tests open the bytes the released engine wrote.
import { readFile, writeFile } from 'node:fs/promises'
import { resolve, join } from 'node:path'
import { pathToFileURL } from 'node:url'

if (!process.argv[2]) throw new Error('Pass the published Scene 0.15.0 package directory')
const directory = resolve(process.argv[2])
const manifest = JSON.parse(await readFile(join(directory, 'kit.json'), 'utf8'))
if (manifest.version !== '0.15.0') throw new Error('The fixture must come from package 0.15.0')
const { default: init, SceneDocument } = await import(pathToFileURL(join(directory, 'engine/kition_scene_kit.js')))
await init({ module_or_path: await readFile(join(directory, 'engine/kition_scene_kit_bg.wasm')) })
const doc = SceneDocument.create('Released.kscene', 640, 480)
try {
  const run = (command, params = {}) => JSON.parse(doc.execute(command, JSON.stringify(params)))
  run('select.objects', { objects: 'Cube' })
  run('modifier.add', { object: 'Cube', type: 'subdivision', levels: 1 })
  run('modifier.apply', { object: 'Cube', index: 0 })
  run('multires.subdivide')
  run('mode.sculpt')
  run('sculpt.stroke', { kind: 'inflate', points: [[0, 0, 0.7]], worldRadius: 1, strength: 0.5 })
  run('mode.set', { mode: 'object' })
  run('physics.rigidBody.add', { objects: 'Cube', mass: 2 })
  run('object.addCube', { name: 'Anchor', size: 1, location: [5, 0, 0] })
  run('physics.rigidBody.add', { objects: 'Anchor', type: 'passive' })
  run('physics.world.set', { gravity: [0, 0, 0], frameEnd: 12 })
  run('select.objects', { objects: 'Cube' })
  await writeFile(new URL('released-0.15.0.kscene', import.meta.url), doc.save())
  console.log(doc.inspect())
} finally {
  doc.free()
}
