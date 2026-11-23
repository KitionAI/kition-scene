// Writes the repository's .kscene fixtures with the engine in public/engine (schema freeze: files
// of the old shape no longer open), into test-results/regen/, and fails so CI keeps them as
// evidence. Temporary: removed once the fixtures are committed.
import { mkdir, readFile, writeFile } from 'node:fs/promises'

const engine = new URL('../public/engine/', import.meta.url)
const { default: init, SceneDocument } = await import(new URL('kition_scene_kit.js', engine).href)
await init({ module_or_path: await readFile(new URL('kition_scene_kit_bg.wasm', engine)) })
const out = new URL('../test-results/regen/', import.meta.url)
await mkdir(out, { recursive: true })

const RED_PNG = 'iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAEUlEQVR4nGP4z8DwH4QZYAwAR8oH+WdZbrcAAAAASUVORK5CYII='
const CORNERS = [[-1, -1, -1], [1, -1, -1], [1, 1, -1], [-1, 1, -1], [-1, -1, 1], [1, -1, 1], [1, 1, 1], [-1, 1, 1]]
const OUTWARD = [[0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [1, 2, 6, 5], [2, 3, 7, 6], [3, 0, 4, 7]]
const faces = (reversed) => OUTWARD.map((face, index) => (reversed(index) ? [...face].reverse() : face))

const problems = []
const make = (name, width, height, steps) => {
  const doc = SceneDocument.create(name, width, height)
  try {
    for (const [command, params] of steps) {
      try {
        JSON.parse(doc.execute(command, JSON.stringify(params ?? {})))
      } catch (error) {
        problems.push(`${name}: ${command}: ${error}`)
      }
    }
    return doc.save()
  } finally {
    doc.free()
  }
}
const conformance = (steps) => make('Scene.kscene', 1280, 720, steps)
const geometry = (reversed) => ['mesh.setGeometry', { object: 'Cube', positions: CORNERS, faces: faces(reversed) }]

const files = {
  'conformance/basic.kscene': conformance([]),
  'conformance/check-zeroScale.kscene': conformance([['object.transform', { objects: 'Cube', scale: [1, 1, 0] }]]),
  'conformance/check-nodesError.kscene': (() => {
    const doc = SceneDocument.create('Scene.kscene', 1280, 720)
    try {
      const made = JSON.parse(doc.execute('nodes.newGroup', JSON.stringify({ object: 'Cube' })))
      doc.execute('nodes.removeNodes', JSON.stringify({ nodes: [made.output] }))
      return doc.save()
    } finally {
      doc.free()
    }
  })(),
  'conformance/check-emptyMesh.kscene': conformance([['mesh.setGeometry', { object: 'Cube', positions: [], faces: [] }]]),
  'conformance/check-degenerateFaces.kscene': conformance([
    ['mesh.setGeometry', { object: 'Cube', positions: [[0, 0, 0], [1, 0, 0], [2, 0, 0], [0, 1, 0]], faces: [[0, 1, 2], [0, 1, 3]] }],
  ]),
  'conformance/check-insideOut.kscene': conformance([geometry(() => true)]),
  'conformance/check-mixedNormals.kscene': conformance([geometry((index) => index === 0)]),
  'conformance/check-textureWithoutUvs.kscene': conformance([
    ['material.setTexture', { material: 'Material', base64: RED_PNG }],
    ['uv.clear', { objects: 'Cube' }],
  ]),
  'conformance/check-noCamera.kscene': conformance([['edit.delete', { objects: 'Camera' }]]),
  'conformance/check-noLight.kscene': conformance([['edit.delete', { objects: 'Light' }]]),
  'conformance/check-outOfShot.kscene': conformance([['object.transform', { objects: 'Cube', location: [0, 0, 200] }]]),
  'conformance/check-unusedMaterial.kscene': conformance([['material.new', { name: 'Spare', assign: false }]]),
  'web/public/sample/Sample.kscene': make('Untitled scene.kscene', 1280, 720, []),
  'cases/scene.agent.context/three-objects-one-run/input/scene.kscene': make('scene.kscene', 640, 360, [
    ['add.cube', { name: 'Left', location: [-3, 0, 0] }],
    ['add.cube', { name: 'Right', location: [3, 0, 0] }],
    ['select.objects', { objects: 'Cube' }],
  ]),
  // The released files' scenes, made again with the commands that made them
  // (bindings/wasm/tests/fixtures/create-released-*.mjs), or, for files without a script, with
  // what the tests that open them use.
  'bindings/wasm/tests/fixtures/released-0.7.0.kscene': make('Room.kscene', 1280, 720, []),
  'bindings/wasm/tests/fixtures/released-0.8.0.kscene': make('Room.kscene', 1280, 720, [['add.uvSphere', { location: [3, 0, 1] }]]),
  'bindings/wasm/tests/fixtures/released-0.9.0.kscene': make('Room.kscene', 1280, 720, [
    ['add.uvSphere', { location: [3, 0, 1] }],
    ['nodes.newGroup', { object: 'Cube' }],
    ['add.lattice', { location: [-3, 0, 1] }],
  ]),
  'bindings/wasm/tests/fixtures/released-0.9.0-rig.kscene': make('Room.kscene', 1280, 720, [
    ['add.armature', { name: 'Rig', location: [-3, 0, 0] }],
    ['add.uvSphere', { name: 'Ball', location: [3, 0, 1] }],
    ['select.objects', { objects: 'Cube' }],
  ]),
  'bindings/wasm/tests/fixtures/released-0.11.0.kscene': make('Room.kscene', 1280, 720, [
    ['add.uvSphere', { name: 'Ball', location: [3, 0, 0] }],
    ['frame.set', { frame: 1 }],
    ['anim.insertKey', { objects: 'Ball', properties: ['location'] }],
    ['frame.set', { frame: 11 }],
    ['object.transform', { objects: 'Ball', location: [3, 0, 2] }],
    ['anim.insertKey', { objects: 'Ball', properties: ['location'] }],
    ['action.new', { name: 'Rise', object: 'Ball' }],
    ['anim.clear', { objects: 'Ball' }],
    ['object.transform', { objects: 'Ball', location: [3, 0, 0] }],
    ['frame.set', { frame: 1 }],
    ['anim.insertKey', { objects: 'Cube', properties: ['location'] }],
    ['frame.set', { frame: 11 }],
    ['object.transform', { objects: 'Cube', location: [0, 0, 1] }],
    ['anim.insertKey', { objects: 'Cube', properties: ['location'] }],
    ['frame.set', { frame: 1 }],
    ['add.empty', { location: [0, 3, 0] }],
    ['select.objects', { objects: 'Cube' }],
  ]),
  'bindings/wasm/tests/fixtures/released-0.12.0.kscene': make('Lab.kscene', 1280, 720, [
    ['add.grid', { name: 'Sheet', size: 2, location: [0, 0, 2] }],
    ['vertexGroup.assign', { object: 'Sheet', name: 'Pin', vertices: [0, 1, 2] }],
    ['add.plane', { name: 'Floor', size: 10 }],
    ['physics.collision.add', { objects: 'Floor' }],
    ['add.uvSphere', { name: 'Head', location: [3, 0, 2] }],
    ['particles.add', { object: 'Head', kind: 'hair' }],
    ['select.objects', { objects: 'Cube' }],
  ]),
  'bindings/wasm/tests/fixtures/released-0.13.0.kscene': make('Released.kscene', 640, 480, [
    ['anim.insertKey', { objects: 'Cube', properties: ['location'], frame: 1 }],
    ['frame.set', { frame: 11 }],
    ['object.transform', { objects: 'Cube', location: [10, 0, 0] }],
    ['anim.insertKey', { objects: 'Cube', properties: ['location'] }],
    ['anim.setInterpolation', { objects: 'Cube', interpolation: 'linear' }],
    ['nla.pushDown', { object: 'Cube', name: 'Move' }],
    ['nla.addStrip', { object: 'Cube', action: 'Move', track: 0, start: 21 }],
  ]),
  'bindings/wasm/tests/fixtures/released-0.15.0.kscene': make('Released.kscene', 640, 480, [
    ['select.objects', { objects: 'Cube' }],
    ['modifier.add', { object: 'Cube', type: 'subdivision', levels: 1 }],
    ['modifier.apply', { object: 'Cube', index: 0 }],
    ['multires.subdivide', {}],
    ['mode.sculpt', {}],
    ['sculpt.stroke', { kind: 'inflate', points: [[0, 0, 0.7]], worldRadius: 1, strength: 0.5 }],
    ['mode.set', { mode: 'object' }],
    ['physics.rigidBody.add', { objects: 'Cube', mass: 2 }],
    ['add.cube', { name: 'Anchor', size: 1, location: [5, 0, 0] }],
    ['physics.rigidBody.add', { objects: 'Anchor', type: 'passive' }],
    ['physics.world.set', { gravity: [0, 0, 0], frameEnd: 12 }],
    ['select.objects', { objects: 'Cube' }],
  ]),
  'bindings/wasm/tests/fixtures/released-0.16.0.kscene': make('Released.kscene', 640, 480, [
    ['render.set', { width: 200, height: 320 }],
    ['camera.set', { object: 'Camera', lens: 43, sensor: 32, clipStart: 0.2, clipEnd: 120 }],
    ['add.camera', { name: 'Ortho', location: [4, -6, 3] }],
    ['camera.set', { object: 'Ortho', orthographic: true, orthoScale: 7 }],
    ['object.lookAt', { object: 'Ortho', target: [0, 0, 0] }],
    ['particles.add', { object: 'Cube', kind: 'emitter', name: 'Flight', count: 6, frameStart: 1, frameEnd: 1, lifetime: 60, seed: 7, normalVelocity: 1, objectVelocity: [0.2, -0.1, 0.3], gravity: 0 }],
    ['add.empty', { name: 'Target', location: [4, 1, 2] }],
    ['select.objects', { objects: 'Cube' }],
    ['frame.set', { frame: 12 }],
  ]),
  'bindings/wasm/tests/fixtures/released-0.17.0.kscene': (() => {
    const doc = SceneDocument.create('Released.kscene', 640, 480)
    try {
      const run = (command, params = {}) => JSON.parse(doc.execute(command, JSON.stringify(params)))
      const group = run('nodes.newGroup', { object: 'Cube', name: 'ReleasedCurves' })
      const curve = run('nodes.addNode', { group: 'ReleasedCurves', kind: 'curveBezierSegment', values: {
        Resolution: 8, Start: [-1, 0, 0], 'Start Handle': [-0.6, 1, 0], 'End Handle': [0.5, 1, 0.4], End: [1, 0, 0.4],
      } })
      run('nodes.link', { group: 'ReleasedCurves', from: curve.node, fromSocket: 'Curve', to: group.output, toSocket: 'Geometry' })
      run('physics.world.set', { gravity: [0, 0, -9.81], frameStart: 1, frameEnd: 12, substeps: 2, iterations: 10 })
      run('add.grid', { name: 'Cloth', xSubdivisions: 2, ySubdivisions: 2, size: 2, location: [3, 0, 3] })
      run('physics.cloth.add', { objects: 'Cloth', mass: 0.3, airDamping: 1, quality: 5 })
      run('add.cube', { name: 'Soft', size: 1, location: [0, 3, 3] })
      run('physics.softBody.add', { objects: 'Soft', mass: 0.5, friction: 0.2, goal: false, frameStart: 1, frameEnd: 12 })
      run('physics.field.new', { name: 'Wind', type: 'wind', strength: 2, flow: 0 })
      run('select.objects', { objects: 'Cube' })
      run('frame.set', { frame: 7 })
      return doc.save()
    } finally {
      doc.free()
    }
  })(),
}
files['cases/scene.agent.conversation/modifier-stack/input/scene.kscene'] = files['conformance/basic.kscene']
files['cases/scene.agent.observe/under-the-light/input/scene.kscene'] = files['conformance/basic.kscene']
files['cases/scene.agent.act/red-material/input/scene.kscene'] = files['conformance/basic.kscene']
files['cases/scene.agent.recover/inside-out/input/cube.kscene'] = files['conformance/check-insideOut.kscene']

for (const [path, bytes] of Object.entries(files)) {
  const file = new URL(path.replaceAll('/', '__'), out)
  await writeFile(file, bytes)
  console.log(`wrote ${path} (${bytes.length} bytes)`)
}
for (const problem of problems) console.log(`PROBLEM ${problem}`)
console.log('FAIL regen: the fixtures are in the evidence (test-results/regen/)')
process.exit(1)
