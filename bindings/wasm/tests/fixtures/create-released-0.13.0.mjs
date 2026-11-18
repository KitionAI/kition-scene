// Run with the published 0.13.0 package directory as the only argument.
import { readFile, writeFile } from 'node:fs/promises'
import { resolve, join } from 'node:path'
import { pathToFileURL } from 'node:url'
const directory = resolve(process.argv[2])
const manifest = JSON.parse(await readFile(join(directory, 'kit.json'), 'utf8'))
if (manifest.version !== '0.13.0') throw new Error('The fixture must come from package 0.13.0')
const { default: init, SceneDocument } = await import(pathToFileURL(join(directory, 'engine/kition_scene_kit.js')))
await init({ module_or_path: await readFile(join(directory, 'engine/kition_scene_kit_bg.wasm')) })
const doc = SceneDocument.create('Released.kscene', 640, 480)
const run = (command, params) => doc.execute(command, JSON.stringify(params))
run('anim.insertKey', { objects: 'Cube', properties: ['location'], frame: 1 })
run('frame.set', { frame: 11 })
run('object.transform', { objects: 'Cube', location: [10, 0, 0] })
run('anim.insertKey', { objects: 'Cube', properties: ['location'] })
run('anim.setInterpolation', { objects: 'Cube', interpolation: 'linear' })
run('nla.pushDown', { object: 'Cube', name: 'Move' })
run('nla.addStrip', { object: 'Cube', action: 'Move', track: 0, start: 21 })
await writeFile(new URL('released-0.13.0.kscene', import.meta.url), doc.save())
doc.free()
