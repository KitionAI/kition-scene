// Selecting by hand (plan scene-usable-by-hand, phase 6): node tests/select.mjs [chromium|webkit]
// Real mouse and keyboard events on the development host, read back through the editor's state:
// a click and a box (the default tool: Shift adds, Ctrl/Cmd takes out) pick without a software
// picture while the GPU viewport draws; A, A twice, Ctrl/Cmd+I; W cycles Select Box, Circle and
// Lasso; B a box with another tool; C paints with a circle (the wheel sizes it, the right button
// ends it); Ctrl+right drag a lasso; Alt+Z X-ray (a box takes what is hidden; the header's
// button); in edit mode the seven vertices of the cube that show and all eight with X-ray, Alt+click
// a loop, Ctrl+Alt+click a ring, L and Ctrl/Cmd+L, Ctrl/Cmd+keypad + and −, Ctrl/Cmd+click a
// shortest path; an undo keeps the selection; Shift+H hides what is not selected; the outliner
// and the viewport select the same. No selection is an undo step or an unsaved change.
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
const said = []
page.on('console', (message) => said.push(`${message.type()}: ${message.text()}`))
page.on('pageerror', (error) => said.push(`pageerror: ${error.message}\n${error.stack ?? ''}`))
const until = async (check, message, timeout = 20000) => {
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
  await page.waitForFunction(() => document.getElementById('status')?.textContent?.includes(' open in '), null, { timeout: 60000 }).catch(async (error) => {
    console.log(`status: ${await page.evaluate(() => document.getElementById('status')?.textContent).catch(() => '?')}`)
    console.log(said.slice(-60).join('\n'))
    throw error
  })
  const kit = page.frameLocator('#kit')
  const frame = page.frames().find((item) => item.url().includes('index.html'))
  const viewport = kit.getByTestId('scene-viewport')
  await kit.locator('[data-testid="scene-viewport"][data-ready="true"]').waitFor({ timeout: 60000 })
  await until(async () => (await kit.locator('.kui-tool-rail button').count()) > 0, "the engine's tools are on the rail")
  const box = await viewport.boundingBox()
  const gl = await frame.evaluate(() => window.sceneGl ?? null)
  const gpu = gl?.active === true
  console.log(`viewport: ${gpu ? 'GPU' : `software (${gl?.reason ?? 'no GPU probe'})`}`)
  const mac = await page.evaluate(() => /Mac|iPhone|iPad/i.test(navigator.platform))
  const mod = mac ? 'Meta' : 'Control'
  const dpr = await frame.evaluate(() => window.devicePixelRatio || 1)
  const run = (steps) => frame.evaluate((steps) => window.kitScene.run(steps), steps)
  const details = async () => (await page.evaluate(async () => window.devHost.connection().ai.inspect())).details
  // The person's own editor state.
  const state = () => frame.evaluate(() => window.kitScene.state())
  const named = async (name) => (await state()).objects.find((item) => item.name === name)
  const selectedNames = async () => {
    const shown = await state()
    return shown.selected.map((id) => shown.objects.find((item) => item.id === id)?.name).sort()
  }
  const tool = async () => (await details()).tool.id
  const undo = async () => (await details()).undo
  const softwareFrames = () => frame.evaluate(() => window.kitScene.engine().invoke('softwareFrames'))
  const onPage = (point) => [box.x + point[0] / dpr, box.y + point[1] / dpr]
  /** Where a world point shows on the page, by the engine's own projection. */
  const project = async (point) => {
    const shown = await state()
    const m = shown.viewMatrix
    const [x, y, z] = point
    const clip = [0, 1, 2, 3].map((row) => m[row] * x + m[4 + row] * y + m[8 + row] * z + m[12 + row])
    return onPage([((clip[0] / clip[3]) * 0.5 + 0.5) * shown.viewWidth, (0.5 - (clip[1] / clip[3]) * 0.5) * shown.viewHeight])
  }
  /** The document pixel of a page point. */
  const toDocument = (at) => [(at[0] - box.x) * dpr, (at[1] - box.y) * dpr]
  const shot = async (name) => {
    if (shots) await page.screenshot({ path: `${shots}/${name}.png` })
  }
  const dragBox = async (from, to, modifier) => {
    if (modifier) await page.keyboard.down(modifier)
    await page.mouse.move(from[0], from[1])
    await page.mouse.down()
    await page.mouse.move((from[0] + to[0]) / 2, (from[1] + to[1]) / 2, { steps: 3 })
    await page.mouse.move(to[0], to[1], { steps: 3 })
    await page.mouse.up()
    if (modifier) await page.keyboard.up(modifier)
  }
  const empty = [box.x + 40, box.y + box.height - 40]
  const base = await undo()
  const dirty = async () => (await page.evaluate(() => window.devHost.connection().ai.inspect().then((answer) => answer.dirty ?? null)))
  const dirtyBefore = await dirty()

  // Everything in view (the light is above the opening view).
  await run([{ command: 'view.frameAll', params: {} }])
  await page.waitForTimeout(500)
  // A click selects, a click on nothing clears; while the GPU viewport draws, neither draws a
  // software picture (picking is by rays).
  const cube = await project([0, 0, 0])
  await page.mouse.move(cube[0], cube[1])
  const framesBefore = await softwareFrames()
  await page.mouse.click(cube[0], cube[1])
  await until(async () => (await selectedNames()).join() === 'Cube', 'a click selects the cube')
  await page.mouse.click(empty[0], empty[1])
  await until(async () => (await selectedNames()).length === 0, 'a click on nothing clears the selection')
  // The default tool is Select Box: a drag is a new selection.
  assert.equal(await tool(), 'select', 'Select Box is the default tool')
  await dragBox([cube[0] - 60, cube[1] - 60], [cube[0] + 60, cube[1] + 60])
  await until(async () => (await selectedNames()).join() === 'Cube', 'a box takes the cube')
  await page.waitForTimeout(300)
  if (gpu) assert.equal(await softwareFrames(), framesBefore, 'the clicks and the box drew no software picture')
  passed(`a click and a box select${gpu ? ' without a software picture' : ''}`)

  // Shift+box adds, Ctrl/Cmd+box takes out.
  const light = await project((await named('Light')).location)
  await dragBox([light[0] - 30, light[1] - 30], [light[0] + 30, light[1] + 30], 'Shift')
  await until(async () => (await selectedNames()).join() === 'Cube,Light', async () => `Shift+box adds the light (${await selectedNames()}; light at ${light}; tool ${await tool()})`)
  await dragBox([cube[0] - 40, cube[1] - 40], [cube[0] + 40, cube[1] + 40], mod)
  await until(async () => (await selectedNames()).join() === 'Light', 'Ctrl/Cmd+box takes the cube out')
  passed('Shift+box adds, Ctrl/Cmd+box takes out')

  // A selects everything, A twice quickly nothing, Ctrl/Cmd+I inverts.
  await page.mouse.move(cube[0], cube[1])
  await page.keyboard.press('KeyA')
  await until(async () => (await selectedNames()).length === 3, 'A selects all')
  // Past the double press's time, so the next two presses are a pair.
  await page.waitForTimeout(600)
  await page.keyboard.press('KeyA')
  await page.keyboard.press('KeyA')
  await until(async () => (await selectedNames()).length === 0, 'A twice deselects all')
  await page.mouse.click(cube[0], cube[1])
  await until(async () => (await selectedNames()).join() === 'Cube', 'the cube again')
  await page.keyboard.press(`${mod}+KeyI`)
  await until(async () => (await selectedNames()).join() === 'Camera,Light', 'Ctrl/Cmd+I inverts')
  assert.equal(await undo(), base, 'selecting makes no undo step')
  if (dirtyBefore != null) assert.equal(await dirty(), dirtyBefore, 'selecting leaves the document as it was')
  passed('A, A twice and Ctrl/Cmd+I')

  // W cycles the select tools; B makes the next drag a box whatever the tool.
  await page.keyboard.press('KeyW')
  await until(async () => (await tool()) === 'selectCircle', 'W: Select Circle')
  await page.keyboard.press('KeyW')
  await until(async () => (await tool()) === 'selectLasso', 'W again: Select Lasso')
  await page.mouse.move(cube[0] - 50, cube[1] - 50)
  await page.mouse.down()
  for (const [dx, dy] of [[50, -50], [50, 50], [-50, 50], [-50, -50]]) await page.mouse.move(cube[0] + dx, cube[1] + dy, { steps: 4 })
  await page.mouse.up()
  await until(async () => (await selectedNames()).join() === 'Cube', 'Select Lasso takes what it goes round')
  await page.keyboard.press('KeyW')
  await until(async () => (await tool()) === 'select', 'W again: Select Box')
  await kit.getByTestId('scene-tool-move').click()
  await until(async () => (await tool()) === 'move', 'the move tool')
  await page.mouse.move(light[0], light[1])
  await page.keyboard.press('KeyB')
  await until(async () => (await tool()) === 'select', 'B picks the box')
  await dragBox([light[0] - 30, light[1] - 30], [light[0] + 30, light[1] + 30])
  await until(async () => (await selectedNames()).join() === 'Light', 'the box selects the light')
  await until(async () => (await tool()) === 'move', 'the move tool comes back after the box')
  await kit.getByTestId('scene-tool-select').click()
  await until(async () => (await tool()) === 'select', 'Select Box again')
  passed('W cycles the select tools; B boxes once')

  // C paints with a circle: the left button adds, the wheel sizes it, the right button ends it.
  await page.mouse.move(cube[0], cube[1])
  await page.keyboard.press('KeyC')
  await until(() => frame.evaluate(() => window.sceneSelection?.circle != null), 'C starts the circle')
  const radius = () => frame.evaluate(() => window.sceneSelection?.circle?.radius ?? 0)
  const before = await radius()
  await page.mouse.wheel(0, -200)
  await until(async () => (await radius()) > before, 'the wheel grows the circle')
  await shot('select-circle')
  await page.mouse.down()
  await page.mouse.move(cube[0] + 10, cube[1] + 5, { steps: 3 })
  await page.mouse.up()
  await until(async () => (await selectedNames()).join() === 'Cube,Light', 'the circle adds the cube')
  await page.mouse.click(cube[0], cube[1], { button: 'right' })
  await until(() => frame.evaluate(() => window.sceneSelection?.circle == null), 'the right button ends the circle')
  await page.waitForTimeout(200)
  assert.equal(await kit.getByTestId('scene-context-menu').count(), 0, 'and opens no menu')
  await page.keyboard.press('KeyC')
  await until(() => frame.evaluate(() => window.sceneSelection?.circle != null), 'C again')
  await page.keyboard.down('Shift')
  await page.mouse.down()
  await page.mouse.move(cube[0] + 4, cube[1] + 4, { steps: 2 })
  await page.mouse.up()
  await page.keyboard.up('Shift')
  await until(async () => (await selectedNames()).join() === 'Light', 'Shift paints the cube out')
  await page.keyboard.press('Escape')
  await until(() => frame.evaluate(() => window.sceneSelection?.circle == null), 'Escape ends the circle')
  passed('C paints with a circle')

  // Ctrl+right drag: a lasso (adds).
  await page.keyboard.down('Control')
  await page.mouse.move(cube[0] - 40, cube[1] - 40)
  await page.mouse.down({ button: 'right' })
  for (const [dx, dy] of [[40, -40], [40, 40], [-40, 40]]) await page.mouse.move(cube[0] + dx, cube[1] + dy, { steps: 4 })
  await page.mouse.up({ button: 'right' })
  await page.keyboard.up('Control')
  await until(async () => (await selectedNames()).join() === 'Cube,Light', 'Ctrl+right drag lassos the cube in')
  await page.waitForTimeout(200)
  assert.equal(await kit.getByTestId('scene-context-menu').count(), 0, 'the lasso opens no menu')
  passed('Ctrl+right drag lassos')

  // X-ray: a small cube hidden behind the big one; a box takes it only with X-ray (Alt+Z).
  const eyeBack = await frame.evaluate(() => {
    const view = window.kitScene.state().orbit
    return [Math.sin(view.turn) * Math.sin(view.tilt), -Math.cos(view.turn) * Math.sin(view.tilt), Math.cos(view.tilt)]
  })
  await run([{ command: 'object.addCube', params: { name: 'Behind', size: 0.5, location: eyeBack.map((value) => -value * 6) } }, { command: 'edit.selectAll', params: { action: 'deselect' } }])
  await until(async () => (await named('Behind')) != null, 'the hidden cube is there')
  const behindUndo = await undo()
  await dragBox([cube[0] - 50, cube[1] - 50], [cube[0] + 50, cube[1] + 50])
  await until(async () => (await selectedNames()).join() === 'Cube', 'without X-ray the box leaves the hidden cube')
  await page.keyboard.press('Alt+KeyZ')
  await until(async () => (await state()).xray === true, 'Alt+Z turns X-ray on')
  await until(async () => (await kit.getByTestId('scene-xray').getAttribute('aria-pressed')) === 'true', 'the header shows X-ray on')
  await dragBox([cube[0] - 50, cube[1] - 50], [cube[0] + 50, cube[1] + 50])
  await until(async () => (await selectedNames()).join() === 'Behind,Cube', 'with X-ray the box takes the hidden cube')
  await shot('select-xray')
  await kit.getByTestId('scene-xray').click()
  await until(async () => (await state()).xray === false, "the header's button turns it off")
  assert.equal(await undo(), behindUndo, 'X-ray is no undo step')
  passed('Alt+Z X-ray selects through')

  // Shift+H hides what is not selected (one step); undo shows it again.
  await page.mouse.click(cube[0], cube[1])
  await until(async () => (await selectedNames()).join() === 'Cube', 'the cube')
  await page.keyboard.press('Shift+KeyH')
  await until(async () => (await named('Light')).hidden && (await named('Camera')).hidden && (await named('Behind')).hidden, 'Shift+H hides the others')
  assert.equal((await named('Cube')).hidden, false, 'the selected cube stays')
  await page.keyboard.press(`${mod}+KeyZ`)
  await until(async () => !(await named('Light')).hidden, 'undo shows them again')
  await until(async () => (await selectedNames()).join() === 'Cube', 'and the cube stays selected')
  passed('Shift+H hides the unselected; undo keeps the selection')

  // The outliner and the viewport select the same.
  await kit.locator('[data-testid="scene-outliner-item"][data-name="Light"] .scene-outliner-name').click()
  await until(async () => (await selectedNames()).join() === 'Light' && (await state()).active === (await named('Light')).id, 'the outliner selects the light')
  await page.mouse.move(cube[0], cube[1])
  await dragBox([cube[0] - 50, cube[1] - 50], [cube[0] + 50, cube[1] + 50])
  await until(async () => (await kit.locator('[data-testid="scene-outliner-item"][data-name="Cube"]').getAttribute('data-selected')) != null, 'the box shows in the outliner')
  assert.equal(await kit.locator('[data-testid="scene-outliner-item"][data-name="Light"]').getAttribute('data-selected'), null, 'the light is no longer selected there')
  passed('the outliner and the viewport agree')

  // Edit mode: the box takes the seven vertices that show, X-ray all eight.
  await run([{ command: 'edit.delete', params: { objects: ['Behind'] } }, { command: 'select.objects', params: { objects: ['Cube'] } }, { command: 'view.frameSelected', params: {} }])
  await run([{ command: 'view.set', params: { distance: (await state()).orbit.distance * 1.8 } }])
  await page.waitForTimeout(400)
  // A box round the cube's corners as they show.
  const corners = await Promise.all([-1, 1].flatMap((x) => [-1, 1].flatMap((y) => [-1, 1].map((z) => project([x, y, z])))))
  const around = [Math.min(...corners.map((p) => p[0])) - 25, Math.min(...corners.map((p) => p[1])) - 25, Math.max(...corners.map((p) => p[0])) + 25, Math.max(...corners.map((p) => p[1])) + 25]
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
  await page.keyboard.press('Tab')
  await until(async () => (await state()).mode === 'edit', 'Tab enters edit mode')
  await run([{ command: 'mesh.select', params: { vertices: [] } }])
  await until(async () => (await state()).vertices.length === 0, 'nothing selected')
  const editFrames = await softwareFrames()
  await dragBox([around[0], around[1]], [around[2], around[3]])
  await until(async () => (await state()).vertices.length === 7, async () => `the box takes the seven vertices that show (${(await state()).vertices})`)
  await page.keyboard.press('Alt+KeyZ')
  await until(async () => (await state()).xray === true, 'X-ray on')
  await dragBox([around[0], around[1]], [around[2], around[3]])
  await until(async () => (await state()).vertices.length === 8, 'with X-ray all eight')
  await page.keyboard.press('Alt+KeyZ')
  await until(async () => (await state()).xray === false, 'X-ray off')
  await page.waitForTimeout(300)
  if (gpu) assert.equal(await softwareFrames(), editFrames, 'edit mode boxes drew no software picture')
  passed('edit mode takes what shows; X-ray takes all')

  // A UV sphere: Alt+click an equator edge (loop), Ctrl+Alt+click (ring), L, Ctrl/Cmd+L, more and
  // less, Ctrl/Cmd+click's shortest path.
  await page.keyboard.press('Tab')
  await until(async () => (await state()).mode === 'object', 'Tab leaves edit mode')
  await run([
    { command: 'object.addUvSphere', params: { name: 'Ball', radius: 2, location: [0, 0, 6] } },
    { command: 'select.objects', params: { objects: ['Ball'] } },
    { command: 'view.frameSelected', params: {} },
    { command: 'mode.set', params: { mode: 'edit' } },
    { command: 'mesh.select', params: { vertices: [] } },
  ])
  await until(async () => (await state()).mode === 'edit' && (await state()).vertices.length === 0, 'the sphere in edit mode')
  const sphereUndo = await undo()
  await page.waitForTimeout(400)
  const ball = await named('Ball')
  // The sphere's equator, from its 32 segments: a point on it facing the eye, and the point above.
  const orbit = (await state()).orbit
  const facing = Math.atan2(-Math.cos(orbit.turn), Math.sin(orbit.turn))
  const step = (Math.PI * 2) / 32
  const toward = Math.round(facing / step) * step
  const at = (angle, z) => [2 * Math.cos(angle), 2 * Math.sin(angle), 6 + z]
  const edgeMiddle = await project(at(toward + step / 2, 0).map((value, index) => (index < 2 ? value * Math.cos(step / 2) : value)))
  await page.mouse.move(edgeMiddle[0], edgeMiddle[1])
  await page.keyboard.down('Alt')
  await page.mouse.click(edgeMiddle[0], edgeMiddle[1])
  await page.keyboard.up('Alt')
  await until(async () => (await state()).vertices.length === 32, async () => `Alt+click selects the equator's 32 vertices (${(await state()).vertices.length}; ball ${JSON.stringify(ball?.mesh)})`)
  // Ctrl/Cmd (a Mac's Control with the left button is its right button).
  await page.keyboard.down(mod)
  await page.keyboard.down('Alt')
  const ringAngle = Math.PI / 16
  const above = 2 * Math.sin(ringAngle) / 2
  const meridian = await project([2 * Math.cos(toward) * Math.cos(ringAngle / 2), 2 * Math.sin(toward) * Math.cos(ringAngle / 2), 6 + above])
  await page.mouse.click(meridian[0], meridian[1])
  await page.keyboard.up('Alt')
  await page.keyboard.up(mod)
  await until(async () => (await state()).vertices.length === 64, async () => `Ctrl/Cmd+Alt+click selects the ring's 64 vertices (${(await state()).vertices.length})`)
  await shot('select-ring')
  await page.keyboard.press(`${mod}+NumpadAdd`)
  await until(async () => (await state()).vertices.length === 128, async () => `Ctrl/Cmd+keypad + grows the band by a row each side (${(await state()).vertices.length})`)
  await page.keyboard.press(`${mod}+NumpadSubtract`)
  await until(async () => (await state()).vertices.length === 64, async () => `Ctrl/Cmd+keypad − takes the rows back (${(await state()).vertices.length})`)
  await run([{ command: 'mesh.select', params: { vertices: [] } }])
  await page.mouse.move(edgeMiddle[0], edgeMiddle[1])
  await page.keyboard.press('KeyL')
  await until(async () => (await state()).vertices.length === ball.mesh.vertices, 'L selects the part under the pointer (the whole sphere)')
  await page.keyboard.press('Shift+KeyL')
  await until(async () => (await state()).vertices.length === 0, 'Shift+L takes it out')
  const first = await project(at(toward, 0))
  const third = await project(at(toward + 3 * step, 0))
  await page.mouse.click(first[0], first[1])
  await until(async () => (await state()).vertices.length === 1, 'a click selects a vertex')
  await page.keyboard.down(mod)
  await page.mouse.click(third[0], third[1])
  await page.keyboard.up(mod)
  await until(async () => (await state()).vertices.length === 4, async () => `Ctrl/Cmd+click adds the shortest path (${(await state()).vertices.length})`)
  await page.keyboard.press(`${mod}+KeyL`)
  await until(async () => (await state()).vertices.length === ball.mesh.vertices, 'Ctrl/Cmd+L selects every part the selection touches')
  assert.equal(await undo(), sphereUndo, 'no selection was an undo step')
  passed('loops, rings, linked parts, more and less, the shortest path')

  // An undo keeps the selected elements while the mesh has as many.
  await run([{ command: 'mesh.select', params: { vertices: [0, 1] } }])
  await until(async () => (await state()).vertices.length === 2, 'two vertices')
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
  await page.keyboard.press('KeyG')
  await page.keyboard.press('KeyZ')
  await page.keyboard.press('Digit1')
  await page.keyboard.press('Enter')
  await until(async () => (await undo()) !== sphereUndo, 'G Z 1 moves the vertices')
  await page.keyboard.press(`${mod}+KeyZ`)
  await until(async () => (await undo()) === sphereUndo, 'undo takes the move back')
  await until(async () => (await state()).vertices.join() === '0,1', async () => `the vertices stay selected (${(await state()).vertices})`)
  passed('an undo keeps the selection')

  assert.deepEqual(errors, [], `no page errors: ${errors.join('; ')}`)
  console.log(`all passed (${which})`)
} catch (error) {
  console.log(said.slice(-40).join('\n'))
  throw error
} finally {
  await browser.close()
  await server.close()
}
