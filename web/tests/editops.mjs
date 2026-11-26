// Edit mode's operations by hand and Adjust Last Operation (plan scene-usable-by-hand, phase 3):
// node tests/editops.mjs [chromium|webkit]
// Real mouse and keyboard events on the development host, read back through the engine: E extrudes
// and moves along the normal (typed values, negative too; the right button keeps the extrusion), I
// insets by the pointer (Ctrl: depth), Ctrl/Cmd+B bevels by the pointer with the wheel's segments,
// Ctrl/Cmd+R shows the loop, cuts it on a click and slides it, the menus X, M and P, Adjust Last
// Operation's panel (a field changed runs the operation again, the undo stack keeps its depth), and
// the walkthrough's mug: a cylinder's top face inset, then extruded down into a hollow.
//
// The GPU viewport is asked for (`kition.scene.gl` = `on`); KITION_SCENE_GL=off runs it on the
// software viewport. KITION_SCENE_SNAPSHOT=<dir> saves screenshots there.
import assert from 'node:assert/strict'
import { chromium, webkit } from 'playwright'
import { createServer } from 'vite'

const which = process.argv[2] ?? 'chromium'
const shots = process.env.KITION_SCENE_SNAPSHOT
const server = await createServer({ root: new URL('..', import.meta.url).pathname, configFile: new URL('../vite.config.ts', import.meta.url).pathname, server: { port: 0, host: '127.0.0.1' }, logLevel: 'error' })
await server.listen()
const gpuArgs = process.platform === 'darwin' ? ['--use-angle=metal', '--enable-gpu', '--ignore-gpu-blocklist'] : ['--enable-unsafe-swiftshader', '--ignore-gpu-blocklist']
const browser = await (which === 'webkit' ? webkit : chromium).launch(which === 'webkit' ? {} : { args: gpuArgs })
const context = await browser.newContext({ viewport: { width: 1440, height: 900 } })
await context.addInitScript((setting) => window.localStorage.setItem('kition.scene.gl', setting), process.env.KITION_SCENE_GL === 'off' ? 'off' : 'on')
const page = await context.newPage()
const errors = []
page.on('pageerror', (error) => errors.push(error.message))
const until = async (check, message, timeout = 15000) => {
  const end = Date.now() + timeout
  while (Date.now() < end) {
    if (await check().catch(() => false)) return
    await page.waitForTimeout(50)
  }
  assert.fail(typeof message === 'function' ? await message() : message)
}
const passed = (name) => console.log(`PASS ${name} (${which})`)

try {
  await page.goto(`http://127.0.0.1:${server.httpServer.address().port}/dev.html`)
  await page.waitForFunction(() => document.getElementById('status')?.textContent?.includes(' open in '), null, { timeout: 60000 })
  const kit = page.frameLocator('#kit')
  const frame = page.frames().find((item) => item.url().includes('index.html'))
  const viewport = kit.getByTestId('scene-viewport')
  await kit.locator('[data-testid="scene-viewport"][data-ready="true"]').waitFor({ timeout: 60000 })
  await until(async () => (await kit.locator('.kui-tool-rail button').count()) > 0, "the engine's tools are on the rail")
  const box = await viewport.boundingBox()
  const gl = await frame.evaluate(() => window.sceneGl ?? null)
  console.log(`viewport: ${gl?.active ? 'GPU' : `software (${gl?.reason ?? 'no GPU probe'})`}`)
  const mac = await page.evaluate(() => /Mac|iPhone|iPad/i.test(navigator.platform))
  const mod = mac ? 'Meta' : 'Control'
  const run = (steps) => frame.evaluate((steps) => window.kitScene.run(steps), steps)
  /** The person's scene, as the editor shows it. */
  const state = () => frame.evaluate(() => window.kitScene.state())
  const modal = () => frame.evaluate(() => window.sceneModal?.answer ?? { active: false })
  const header = () => kit.getByTestId('scene-modal-header').textContent().catch(() => '')
  const active = async () => (await modal()).active === true
  const ended = async () => !(await active()) && (await kit.getByTestId('scene-modal-header').count()) === 0
  const details = async () => (await page.evaluate(async () => window.devHost.connection().ai.inspect())).details
  const undo = async () => (await details()).undo
  const vertexCount = async (name = 'Cube') => (await state()).objects.find((item) => item.name === name)?.mesh?.vertices
  const operation = async () => (await state()).operation
  /** The meshes' vertices as the engine exports them (OBJ, world space written Y-up), back in Z-up world space. */
  const exported = async () => {
    const text = await frame.evaluate(async () => {
      const answer = await window.kitScene.engine().execute('file.export', { format: 'obj', selectedOnly: false })
      return atob(JSON.parse(answer).base64)
    })
    return text
      .split('\n')
      .filter((line) => line.startsWith('v '))
      .map((line) => line.split(/\s+/).slice(1, 4).map(Number))
      .map(([x, y, z]) => [x, -z, y])
  }
  const back = async (label) => {
    await page.keyboard.press(`${mod}+KeyZ`)
    await until(async () => (await undo()) !== label, `undo takes the ${label} back`)
  }
  const glide = async (to, steps = 8) => {
    await page.mouse.move(to[0], to[1], { steps })
    await page.waitForTimeout(150)
  }
  /** A world point on the screen (the engine's view, as basics.mjs reads it). */
  const project = async (point) => {
    const v = (await details()).view
    const rad = Math.PI / 180
    const [sx, cxx, sz, cz] = [Math.sin(v.tilt * rad), Math.cos(v.tilt * rad), Math.sin(v.turn * rad), Math.cos(v.turn * rad)]
    const r = [[cz, -sz * cxx, sz * sx], [sz, cz * cxx, -cz * sx], [0, sx, cxx]]
    const e = [v.target[0] + r[0][2] * v.distance, v.target[1] + r[1][2] * v.distance, v.target[2] + r[2][2] * v.distance]
    const d = [point[0] - e[0], point[1] - e[1], point[2] - e[2]]
    const eye = [0, 1, 2].map((col) => r[0][col] * d[0] + r[1][col] * d[1] + r[2][col] * d[2])
    const aspect = box.width / box.height
    const fx = 2 * Math.atan(72 / (2 * 50))
    const fy = aspect >= 1 ? 2 * Math.atan(Math.tan(fx / 2) / aspect) : fx
    let nx
    let ny
    if (v.perspective) {
      const f = 1 / Math.tan(fy / 2)
      nx = ((f / aspect) * eye[0]) / -eye[2]
      ny = (f * eye[1]) / -eye[2]
    } else {
      const hh = v.distance * Math.tan(fy / 2)
      nx = eye[0] / (hh * aspect)
      ny = eye[1] / hh
    }
    return [box.x + (nx * 0.5 + 0.5) * box.width, box.y + (0.5 - ny * 0.5) * box.height]
  }
  const shot = async (name) => {
    if (shots && which === 'chromium') await page.screenshot({ path: `${shots}/${name}.png` })
  }

  // The cube in edit mode, its top face selected (the default cube's face 5 is its top).
  await run([
    { command: 'select.objects', params: { objects: ['Cube'] } },
    { command: 'mode.set', params: { mode: 'edit' } },
  ])
  await until(async () => (await state()).mode === 'edit', 'edit mode')
  assert.equal(await vertexCount(), 8, 'the default cube')
  const selectTop = () => run([{ command: 'mesh.select', params: { faces: [5] } }])
  const top = await project([0, 0, 1])
  await selectTop()
  const base = await undo()

  // E: extruded at once and moved along the normal; typed -0.5 puts the top inside; one step.
  await page.mouse.move(top[0], top[1])
  await page.keyboard.press('KeyE')
  await until(active, 'E starts the extrusion')
  await until(async () => (await vertexCount()) === 12, 'the extrusion shows at once')
  await glide([top[0], top[1] - 60])
  await until(async () => / normal$/.test(await header()), async () => `the header reads D: … normal (${await header()})`)
  assert.equal(await undo(), base, 'no step while it moves')
  for (const key of ['Minus', 'Digit0', 'Period', 'Digit5']) await page.keyboard.press(key)
  await until(async () => (await header()).includes('[-(0.5|)] = -0.5000 m'), async () => `the header shows the typed -0.5 (${await header()})`)
  await shot('01-extrude-typed')
  await page.keyboard.press('Enter')
  await until(async () => (await undo()) === 'Extrude Faces', async () => `one Extrude Faces step (${await undo()})`)
  await until(ended, 'the modal ended')
  const extruded = await exported()
  assert.ok(extruded.some(([, , z]) => Math.abs(z - 0.5) < 1e-3), 'the top went 0.5 m in')
  await until(async () => (await operation())?.command === 'mesh.extrude', 'Adjust Last Operation shows the extrusion')
  assert.equal((await operation()).params.distance, -0.5)
  await back('Extrude Faces')
  await until(async () => (await vertexCount()) === 8, 'one undo takes the whole extrusion back')
  passed('E extrudes along the normal, typed values negative too')

  // The right button stops the move and keeps the extrusion where it was made.
  await selectTop()
  await page.mouse.move(top[0], top[1])
  await page.keyboard.press('KeyE')
  await until(active, 'E starts the extrusion')
  await glide([top[0], top[1] - 60])
  await page.mouse.click(top[0], top[1] - 60, { button: 'right' })
  await until(async () => (await undo()) === 'Extrude Faces', 'the extrusion is a step')
  await until(ended, 'the modal ended')
  assert.equal(await vertexCount(), 12, 'the extruded geometry stays')
  assert.equal(await kit.getByTestId('scene-context-menu').count(), 0, 'the right button opened no menu')
  await back('Extrude Faces')
  await until(async () => (await vertexCount()) === 8, 'undo takes it back')
  passed('the right button keeps the extrusion')

  // I: the pointer towards the centre insets; Ctrl moves to the depth; Enter confirms.
  await selectTop()
  const right = [top[0] + 160, top[1]]
  await page.mouse.move(right[0], right[1])
  await page.keyboard.press('KeyI')
  await until(async () => (await modal()).kind === 'inset', 'I starts the inset')
  await glide([top[0] + 100, top[1]])
  await until(async () => (await modal()).values?.thickness > 0.05, async () => `the pointer sizes the thickness (${await header()})`)
  assert.match(await header(), /^Thickness: .* m, Depth: 0\.0000 m$/)
  await page.keyboard.down(mod)
  await glide([top[0] + 60, top[1]])
  await until(async () => Math.abs((await modal()).values?.depth ?? 0) > 0.05, async () => `Ctrl sizes the depth (${await header()})`)
  await page.keyboard.up(mod)
  await shot('02-inset-depth')
  await page.keyboard.press('Enter')
  await until(async () => (await undo()) === 'Inset Faces', async () => `one Inset Faces step (${await undo()})`)
  assert.equal(await vertexCount(), 12)
  // Escape: no inset, no step.
  await selectTop()
  await page.mouse.move(right[0], right[1])
  await page.keyboard.press('KeyI')
  await until(async () => (await modal()).kind === 'inset', 'I starts the inset')
  await glide([top[0] + 90, top[1]])
  await page.keyboard.press('Escape')
  await until(ended, 'Escape ends it')
  assert.equal(await vertexCount(), 12, 'nothing more inset')
  assert.equal(await undo(), 'Inset Faces', 'no new step')
  await back('Inset Faces')
  await until(async () => (await vertexCount()) === 8, 'undo takes the inset back')
  passed('I insets by the pointer, Ctrl for depth')

  // Ctrl/Cmd+B on the front top edge: the pointer sizes it, the wheel adds a segment.
  await run([{ command: 'mesh.select', params: { vertices: [1, 5] } }])
  const edge = await project([0, -1, 1])
  await page.mouse.move(edge[0], edge[1] - 20)
  await page.keyboard.press(`${mod}+KeyB`)
  await until(async () => (await modal()).kind === 'bevel', `${mod}+B starts the bevel`)
  await glide([edge[0], edge[1] - 70])
  await until(async () => (await modal()).values?.width > 0.02, async () => `the pointer sizes the width (${await header()})`)
  const zoom = (await details()).view.distance
  await page.mouse.wheel(0, -100)
  await until(async () => /Segments: 2$/.test(await header()), async () => `the wheel adds a segment (${await header()})`)
  assert.equal((await details()).view.distance, zoom, 'the wheel did not zoom')
  await shot('03-bevel-segments')
  await page.mouse.click(edge[0], edge[1] - 70)
  await until(async () => (await undo()) === 'Bevel Edges', async () => `one Bevel Edges step (${await undo()})`)
  assert.equal((await operation())?.params.segments, 2, 'the panel has its segments')
  await back('Bevel Edges')
  await until(async () => (await vertexCount()) === 8, 'undo takes the bevel back')
  passed('Ctrl/Cmd+B bevels, the wheel sets segments')

  // Ctrl/Cmd+R over a side edge: the loop shows; the wheel makes two; a click cuts and slides; the right button centres.
  await run([{ command: 'edit.deselectAll', params: {} }])
  const side = await project([1, -1, 0])
  await page.mouse.move(side[0], side[1])
  await page.keyboard.press(`${mod}+KeyR`)
  await until(async () => (await modal()).kind === 'loopCut', `${mod}+R starts the loop cut (the page did not reload)`)
  await glide([side[0] + 1, side[1] + 1], 2)
  await until(async () => Array.isArray((await modal()).edge), 'the edge under the pointer is found')
  const yellow = await frame.evaluate(() => {
    const canvas = document.querySelector('[data-testid="scene-tool-overlay"]')
    if (!canvas) return -1
    const data = canvas.getContext('2d').getImageData(0, 0, canvas.width, canvas.height).data
    let count = 0
    for (let index = 0; index < data.length; index += 4) if (data[index + 3] > 0 && data[index] > 200 && data[index + 1] > 170 && data[index + 2] < 80) count += 1
    return count
  })
  assert.ok(yellow > 50 || yellow === -1, `the yellow loop shows over the picture (${yellow})`)
  await page.mouse.wheel(0, -100)
  await until(async () => (await header()) === 'Cuts: 2', async () => `the wheel makes two cuts (${await header()})`)
  await shot('04-loop-cut-preview')
  await page.mouse.click(side[0] + 1, side[1] + 1)
  await until(async () => (await modal()).kind === 'slide', 'the click cuts and slides')
  await until(async () => (await vertexCount()) === 16, 'two loops of four')
  await glide([side[0] + 1, side[1] - 40])
  await page.mouse.click(side[0] + 1, side[1] - 40, { button: 'right' })
  await until(async () => (await undo()) === 'Loop Cut', 'one Loop Cut step')
  await until(ended, 'the slide ended')
  assert.equal(await vertexCount(), 16, 'the right button keeps the cut')
  await back('Loop Cut')
  await until(async () => (await vertexCount()) === 8, 'undo takes the cut back')
  // Escape before the click cuts nothing.
  await page.keyboard.press(`${mod}+KeyR`)
  await until(async () => (await modal()).kind === 'loopCut', 'the loop cut again')
  await page.keyboard.press('Escape')
  await until(ended, 'Escape ends it')
  assert.equal(await vertexCount(), 8)
  passed('Ctrl/Cmd+R shows the loop, cuts, slides')

  // Mesh ▸ Loop Cut opens no form: it starts the same modal.
  await kit.getByTestId('scene-menu-Mesh').click()
  await kit.getByTestId('scene-cmd-mesh.loopCut').click()
  await until(async () => (await modal()).kind === 'loopCut', 'the menu row starts the loop cut')
  assert.equal(await kit.locator('[role="dialog"]').count(), 0, 'no form of raw parameters')
  await page.keyboard.press('Escape')
  await until(ended, 'Escape ends it')
  passed('Mesh ▸ Loop Cut starts the modal, no form')

  // X opens the Delete menu at the pointer; Faces deletes the selected face. M and P open theirs.
  await selectTop()
  await page.mouse.move(top[0], top[1])
  await page.keyboard.press('KeyX')
  await kit.getByTestId('scene-delete-menu').waitFor()
  assert.ok(await kit.getByTestId('scene-popup-mesh.dissolveFaces').isVisible(), 'Dissolve Faces is in it')
  await kit.getByTestId('scene-popup-mesh.deleteFaces').click()
  await until(async () => (await undo()) === 'Delete Faces', 'Faces deletes the face')
  await until(async () => String(await frame.evaluate(() => document.activeElement?.className ?? '')).includes('kui-app'), 'the editor has the keyboard back')
  await back('Delete Faces')
  await page.mouse.move(top[0], top[1])
  await page.keyboard.press('KeyM')
  await kit.getByTestId('scene-merge-menu').waitFor()
  await page.keyboard.press('Escape')
  await page.keyboard.press('KeyP')
  await kit.getByTestId('scene-separate-menu').waitFor()
  assert.ok(await kit.getByTestId('scene-popup-mesh.separate').isVisible(), 'Selection is in it')
  await page.keyboard.press('Escape')
  await until(async () => (await kit.getByTestId('scene-separate-menu').count()) === 0, 'Escape closes it')
  assert.equal(await vertexCount(), 8, 'the menus changed nothing')
  passed('X, M and P open their menus at the pointer')

  // The walkthrough's mug: a cylinder, its top face inset 0.1 m and extruded 1.8 m down into a hollow.
  await run([{ command: 'mode.set', params: { mode: 'object' } }, { command: 'object.addCylinder', params: { location: [4, 0, 0] } }])
  await until(async () => (await state()).objects.some((item) => item.name === 'Cylinder'), 'the cylinder is there')
  // The panel offers the cylinder's size: 16 vertices around instead of 32, from its field.
  await until(async () => (await operation())?.title === 'Cylinder', 'Adjust Last Operation shows the cylinder')
  if ((await kit.getByTestId('scene-adjust-last').getAttribute('data-open')) !== 'true') await kit.getByTestId('scene-adjust-last-title').click()
  const verticesField = kit.getByTestId('scene-adjust-vertices').locator('input')
  await verticesField.fill('16')
  await verticesField.press('Enter')
  await until(async () => (await vertexCount('Cylinder')) === 32, async () => `the field remade the cylinder with 16 around (${await vertexCount('Cylinder')})`)
  assert.equal((await state()).objects.filter((item) => item.name.startsWith('Cylinder')).length, 1, 'one cylinder, not two')
  await verticesField.fill('32')
  await verticesField.press('Enter')
  await until(async () => (await vertexCount('Cylinder')) === 64, 'back to 32 around')
  await run([
    { command: 'select.objects', params: { objects: ['Cylinder'] } },
    { command: 'view.frameSelected', params: {} },
    { command: 'mode.set', params: { mode: 'edit' } },
    { command: 'mesh.select', params: { faces: [33] } },
  ])
  await until(async () => (await state()).mode === 'edit' && (await state()).faces.length === 1, 'the top face is selected')
  const mugTop = await project([4, 0, 1])
  await page.mouse.move(mugTop[0] + 200, mugTop[1])
  await page.keyboard.press('KeyI')
  await until(async () => (await modal()).kind === 'inset', 'I starts the inset')
  await glide([mugTop[0] + 170, mugTop[1]])
  for (const key of ['Digit0', 'Period', 'Digit1', 'Enter']) await page.keyboard.press(key)
  await until(async () => (await undo()) === 'Inset Faces', 'the inset is a step')
  await page.mouse.move(mugTop[0], mugTop[1])
  await page.keyboard.press('KeyE')
  await until(async () => (await modal()).extrude === true, 'E starts the extrusion')
  for (const key of ['Minus', 'Digit1', 'Period', 'Digit8']) await page.keyboard.press(key)
  await until(async () => (await header()).includes('-1.8000 m'), async () => `the header shows -1.8 (${await header()})`)
  await page.keyboard.press('Enter')
  await until(async () => (await undo()) === 'Extrude Faces', 'the extrusion is a step')
  let mug = (await exported()).filter(([x]) => x > 2)
  assert.ok(mug.some(([x, y, z]) => Math.abs(z + 0.8) < 1e-3 && Math.hypot(x - 4, y) > 0.85 && Math.hypot(x - 4, y) < 0.95), 'the hollow goes down to -0.8, 0.1 m in from the wall')
  await shot('05-mug-hollow')
  // Adjust Last Operation: a shallower hollow from the panel, still one step.
  await until(async () => (await operation())?.command === 'mesh.extrude', 'the panel shows the extrusion')
  const depthBefore = (await page.evaluate(async () => window.devHost.connection().ai.inspect())).details.undo
  const distance = kit.getByTestId('scene-adjust-distance').locator('input')
  if (!(await distance.isVisible())) await kit.getByTestId('scene-adjust-last-title').click()
  await distance.fill('-1.5')
  await distance.press('Enter')
  await until(async () => (await operation())?.params.distance === -1.5, 'the panel ran it again')
  mug = (await exported()).filter(([x]) => x > 2)
  assert.ok(mug.some(([, , z]) => Math.abs(z + 0.5) < 1e-3) && !mug.some(([, , z]) => Math.abs(z + 0.8) < 1e-3), 'the hollow is 1.5 m deep now')
  assert.equal(await undo(), depthBefore, 'the same step on top')
  await shot('06-mug-adjusted')
  await back('Extrude Faces')
  assert.equal(await undo(), 'Inset Faces', 'one undo took the adjusted extrusion back, the inset is next')
  assert.equal(await operation(), null, 'and the panel is gone')
  passed("the walkthrough's mug: inset, extrude down, adjust")

  // F9 opens the panel's fields; selecting something else ends it.
  await page.keyboard.press(`${mod}+Shift+KeyZ`)
  await until(async () => (await undo()) === 'Extrude Faces', 'redo')
  await run([{ command: 'mode.set', params: { mode: 'object' } }, { command: 'select.objects', params: { objects: ['Cube'] } }])
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
  await page.keyboard.press('KeyG')
  await until(active, 'G')
  for (const key of ['KeyZ', 'Digit1', 'Enter']) await page.keyboard.press(key)
  await until(async () => (await operation())?.title === 'Move', 'the panel shows the move')
  const open = await kit.getByTestId('scene-adjust-last').getAttribute('data-open')
  await page.keyboard.press('F9')
  await until(async () => (await kit.getByTestId('scene-adjust-last').getAttribute('data-open')) !== open, 'F9 opens or closes the fields')
  assert.equal((await operation()).params.axis, 'z')
  await run([{ command: 'edit.deselectAll', params: {} }])
  await until(async () => (await kit.getByTestId('scene-adjust-last').count()) === 0, 'another change ends the panel')
  passed('F9, and the panel goes with the next change')

  assert.deepEqual(errors, [], 'no page errors')
  console.log(`editops: all passed (${which})`)
} finally {
  await browser.close()
  await server.close()
}
