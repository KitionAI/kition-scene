// The thirty things a Blender user does in the first ten minutes (plan scene-usable-by-hand, phase
// 9, the acceptance checklist): node tests/first-ten-minutes.mjs [chromium|webkit]
// One section per item, with real mouse, wheel and keyboard events on the development host, read
// back through the editor's state, the engine's inspect, the header's text and the overlay's
// pixels. What the editor does not do as Blender does is not asserted here; the results table
// (kition-plan kit-docs/scene/walkthrough/acceptance.md) records it, and the sections print a NOTE.
//
// The GPU viewport is asked for (`kition.scene.gl` = `on`); KITION_SCENE_GL=off runs it on the
// software viewport. KITION_SCENE_SNAPSHOT=<dir> saves a screenshot per item there.
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
const until = async (check, message, timeout = 15000) => {
  const end = Date.now() + timeout
  while (Date.now() < end) {
    if (await check().catch(() => false)) return
    await page.waitForTimeout(50)
  }
  assert.fail(typeof message === 'function' ? await message() : message)
}
const near = (a, b, tolerance = 1e-3) => Math.abs(a - b) <= tolerance
const nearAll = (a, b, tolerance = 1e-3) => a.length === b.length && a.every((value, index) => near(value, b[index], tolerance))
const item = (number, name) => console.log(`PASS ${String(number).padStart(2)} ${name} (${which})`)
const note = (number, text) => console.log(`NOTE ${String(number).padStart(2)} ${text}`)

try {
  await page.goto(`http://127.0.0.1:${server.httpServer.address().port}/dev.html`)
  await page.waitForFunction(() => document.getElementById('status')?.textContent?.includes(' open in '), null, { timeout: 60000 }).catch(async (error) => {
    console.log(`status: ${await page.evaluate(() => document.getElementById('status')?.textContent).catch(() => '?')}`)
    console.log(said.slice(-60).join('\n'))
    throw error
  })
  const kit = page.frameLocator('#kit')
  const frame = page.frames().find((entry) => entry.url().includes('index.html'))
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
  const state = () => frame.evaluate(() => window.kitScene.state())
  const named = async (name) => (await state()).objects.find((entry) => entry.name === name)
  const details = async () => (await page.evaluate(async () => window.devHost.connection().ai.inspect())).details
  const view = async () => (await details()).view
  const undo = async () => (await details()).undo
  const selectedNames = async () => {
    const shown = await state()
    return shown.objects.filter((entry) => shown.selected.includes(entry.id)).map((entry) => entry.name).sort()
  }
  const modal = () => frame.evaluate(() => window.sceneModal?.answer ?? { active: false })
  const active = async () => (await modal()).active === true
  const header = () => kit.getByTestId('scene-modal-header').textContent().catch(() => '')
  const ended = async () => !(await active()) && (await kit.getByTestId('scene-modal-header').count()) === 0
  const operation = async () => (await state()).operation
  const vertexCount = async (name = 'Cube') => (await named(name))?.mesh?.vertices
  const shot = async (name) => {
    if (shots) await page.screenshot({ path: `${shots}/${name}.png` })
  }
  /** The undo label once it has stayed the same for 400 ms (a late step may still be on its way). */
  const steadyUndo = async () => {
    let last = await undo()
    for (let same = 0, tries = 0; same < 4 && tries < 60; tries += 1) {
      await page.waitForTimeout(100)
      const now = await undo()
      same = now === last ? same + 1 : 0
      last = now
    }
    return last
  }
  /** Where a world point shows on the page, by the editor's own projection. */
  const project = async (point) => {
    const shown = await state()
    const m = shown.viewMatrix
    const [x, y, z] = point
    const clip = [0, 1, 2, 3].map((row) => m[row] * x + m[4 + row] * y + m[8 + row] * z + m[12 + row])
    return [box.x + (((clip[0] / clip[3]) * 0.5 + 0.5) * shown.viewWidth) / dpr, box.y + ((0.5 - (clip[1] / clip[3]) * 0.5) * shown.viewHeight) / dpr]
  }
  /** Waits for the view to settle where `check` wants it (a change of view takes 200 ms). */
  const viewIs = (check, message) => until(async () => check(await view()), async () => `${message} (${JSON.stringify(await view())})`)
  /** The view once it has stayed the same for 300 ms. */
  const steadyView = async () => {
    let last = JSON.stringify(await view())
    for (let same = 0, tries = 0; same < 3 && tries < 60; tries += 1) {
      await page.waitForTimeout(100)
      const now = JSON.stringify(await view())
      same = now === last ? same + 1 : 0
      last = now
    }
    return JSON.parse(last)
  }
  /** Pixels of the overlay layer (the software picture without the GPU viewport) matching `test`. */
  const overlayInk = (kind) =>
    frame.evaluate(([gpuShown, kind]) => {
      const canvas = document.querySelector(gpuShown ? '[data-testid="scene-tool-overlay"]' : '[data-testid="scene-viewport"] canvas.kui-canvas-surface')
      const context = canvas?.getContext('2d')
      if (!canvas || !context) return -1
      const data = context.getImageData(0, 0, canvas.width, canvas.height).data
      let count = 0
      for (let index = 0; index < data.length; index += 4) {
        const [r, g, b, a] = [data[index], data[index + 1], data[index + 2], data[index + 3]]
        if (a === 0) continue
        if (kind === 'any' || (kind === 'red' && r > 200 && g < 120) || (kind === 'yellow' && r > 200 && g > 170 && b < 80)) count += 1
      }
      return count
    }, [gpu, kind])
  const centre = [box.x + box.width / 2, box.y + box.height / 2]
  const empty = [box.x + 40, box.y + box.height - 40]
  const glide = async (to, steps = 8) => {
    await page.mouse.move(to[0], to[1], { steps })
    await page.waitForTimeout(150)
  }
  const focusEditor = () => kit.locator('.kui-app').focus()
  /** Starts a modal with `key` at `at` and waits until it runs. */
  const begin = async (key, at) => {
    await page.mouse.move(at[0], at[1])
    await page.keyboard.press(key)
    await until(active, `${key} starts a transform`)
  }
  /** Object mode, the cube alone selected at the origin as it opened, nothing else changed. */
  const reset = async () => {
    if ((await state()).mode !== 'object') await run([{ command: 'mode.set', params: { mode: 'object' } }])
    await run([
      { command: 'select.objects', params: { objects: ['Cube'] } },
      { command: 'object.transform', params: { objects: ['Cube'], location: [0, 0, 0], rotation: [0, 0, 0], scale: [1, 1, 1] } },
    ])
    await until(async () => {
      const shown = await state()
      const cube = shown.objects.find((entry) => entry.name === 'Cube')
      return shown.mode === 'object' && shown.active === cube.id && shown.selected.length === 1 && cube.location.every((value) => value === 0) && cube.scale.every((value) => value === 1)
    }, 'the cube alone, at the origin, in object mode')
  }
  const back = async (label) => {
    await page.keyboard.press(`${mod}+KeyZ`)
    await until(async () => (await undo()) !== label, `undo takes the ${label} back`)
  }

  await run([{ command: 'view.frameAll', params: {} }])
  const opening = await steadyView()
  /** The view the test began with (a three-quarter view in perspective). */
  const openingView = async () => {
    // A key's change of view ends 200 ms on by handing the engine its view: a view set before then
    // would be replaced by it.
    await page.waitForTimeout(400)
    await run([{ command: 'view.set', params: { target: opening.target, distance: opening.distance, tilt: opening.tilt, turn: opening.turn, perspective: true } }])
    await viewIs((v) => near(v.tilt, opening.tilt) && near(v.turn, opening.turn) && v.perspective, 'the opening view again')
    await page.waitForTimeout(400)
    await viewIs((v) => near(v.tilt, opening.tilt) && near(v.turn, opening.turn) && v.perspective, 'the opening view stays')
  }
  await reset()
  const cube = await project([0, 0, 0])

  // 1. The middle button orbits (turntable: tilt and turn, no roll); the right button cancels it.
  let before = await steadyView()
  await page.mouse.move(cube[0] + 100, cube[1] + 80)
  await page.mouse.down({ button: 'middle' })
  await page.mouse.move(cube[0] + 180, cube[1] + 110, { steps: 6 })
  await page.mouse.up({ button: 'middle' })
  await viewIs((v) => !near(v.turn, before.turn, 1) && !near(v.tilt, before.tilt, 1) && near(v.distance, before.distance) && nearAll(v.target, before.target), 'a middle drag orbits about the view centre')
  const orbited = await steadyView()
  await page.mouse.move(cube[0] + 100, cube[1] + 80)
  await page.mouse.down({ button: 'middle' })
  await page.mouse.move(cube[0] + 200, cube[1] + 40, { steps: 6 })
  await page.waitForTimeout(150)
  await page.mouse.down({ button: 'right' })
  await page.mouse.up({ button: 'right' })
  await page.mouse.up({ button: 'middle' })
  await viewIs((v) => near(v.turn, orbited.turn, 1e-2) && near(v.tilt, orbited.tilt, 1e-2), 'the right button during the orbit puts the view back')
  await page.waitForTimeout(200)
  assert.equal(await kit.getByTestId('scene-context-menu').count(), 0, 'the right button opened no menu')
  await shot('01-orbit')
  item(1, 'the middle button orbits; the right button cancels it')

  // 2. The wheel zooms a step a notch toward the view's centre; Ctrl+middle drag zooms smoothly.
  before = await steadyView()
  await page.mouse.move(cube[0] + 200, cube[1] - 100)
  await page.mouse.wheel(0, -100)
  await viewIs((v) => v.distance < before.distance * 0.95 && nearAll(v.target, before.target) && near(v.turn, before.turn), 'a wheel notch zooms in toward the centre')
  before = await steadyView()
  await page.mouse.wheel(0, 100)
  await viewIs((v) => v.distance > before.distance * 1.05, 'a notch the other way zooms out')
  before = await steadyView()
  await page.keyboard.down('Control')
  await page.mouse.down({ button: 'middle' })
  await page.mouse.move(cube[0] + 200, cube[1] - 20, { steps: 6 })
  await page.mouse.up({ button: 'middle' })
  await page.keyboard.up('Control')
  await viewIs((v) => v.distance < before.distance * 0.9 && near(v.turn, before.turn), 'Ctrl+middle drag down zooms in')
  item(2, 'the wheel and Ctrl+middle drag zoom')

  // 3. Shift+middle drag pans.
  before = await steadyView()
  await page.keyboard.down('Shift')
  await page.mouse.move(centre[0], centre[1])
  await page.mouse.down({ button: 'middle' })
  await page.mouse.move(centre[0] + 90, centre[1] + 40, { steps: 6 })
  await page.mouse.up({ button: 'middle' })
  await page.keyboard.up('Shift')
  await viewIs((v) => !nearAll(v.target, before.target) && near(v.turn, before.turn) && near(v.distance, before.distance), 'Shift+middle drag pans')
  item(3, 'Shift+middle drag pans')
  await run([{ command: 'view.frameAll', params: {} }])
  await steadyView()

  // 4. A click selects only that object and makes it active; a click on nothing deselects all.
  await run([{ command: 'edit.selectAll', params: { action: 'select' } }])
  await until(async () => (await state()).selected.length === 3, 'everything selected')
  let at = await project([0, 0, 0])
  await page.mouse.click(at[0], at[1])
  await until(async () => (await selectedNames()).join() === 'Cube' && (await state()).active === (await named('Cube')).id, 'a click selects the cube alone, active')
  await shot('04-click-select')
  await page.mouse.click(empty[0], empty[1])
  await until(async () => (await state()).selected.length === 0, 'a click on nothing deselects all')
  // A double click is two clicks: it selects, and leaves the mode alone (it entered edit mode).
  await page.mouse.dblclick(at[0], at[1])
  await until(async () => (await selectedNames()).join() === 'Cube', 'a double click selects the cube')
  await page.waitForTimeout(400)
  assert.equal((await state()).mode, 'object', 'a double click stays in object mode')
  item(4, 'a click selects, a click on nothing deselects')

  // 5. G moves with the pointer, no button held; the header reads Dx Dy Dz; a click confirms;
  // the right button and Escape put it back.
  await reset()
  const base = await steadyUndo()
  const start = [cube[0] + 60, cube[1] + 40]
  await begin('KeyG', start)
  await glide([start[0] + 110, start[1] - 30])
  await until(async () => /^Dx: .* Dy: .* Dz: .* \(.* m\)$/.test(await header()), async () => `the header reads Dx: … Dy: … Dz: … (… m) (${await header()})`)
  await shot('05-grab')
  await page.mouse.click(start[0] + 110, start[1] - 30)
  await until(async () => (await undo()) === 'Move', 'a click confirms one Move step')
  await until(ended, 'the move ended')
  assert.ok(Math.hypot(...(await named('Cube')).location) > 0.3, 'the cube moved')
  await back('Move')
  await begin('KeyG', start)
  await glide([start[0] - 100, start[1] - 50])
  await page.mouse.click(start[0] - 100, start[1] - 50, { button: 'right' })
  await until(ended, 'the right button ends the move')
  await begin('KeyG', start)
  await glide([start[0] + 70, start[1] + 50])
  await page.keyboard.press('Escape')
  await until(ended, 'Escape ends the move')
  await page.waitForTimeout(200)
  assert.deepEqual((await named('Cube')).location, [0, 0, 0], 'cancelled, the cube is back')
  assert.equal(await undo(), base, 'no step')
  note(5, 'the moving object keeps its orange outline (Blender draws it white while it moves)')
  item(5, 'G moves, a click confirms, the right button and Escape cancel')

  // 6. G X keeps to X, with the axis drawn; X again is local X; a third time frees it.
  await run([{ command: 'object.transform', params: { objects: ['Cube'], rotation: [0, 0, 30] } }])
  await until(async () => near((await named('Cube')).rotation[2], 30), 'the cube turned 30°')
  await begin('KeyG', start)
  await page.keyboard.press('KeyX')
  await until(async () => /along global X$/.test(await header()), async () => `X: along global X (${await header()})`)
  await glide([start[0] + 120, start[1] + 40])
  if (gpu) await until(async () => (await overlayInk('red')) > 200, 'the X axis is drawn in red over the picture')
  await shot('06-axis-x')
  await page.keyboard.press('KeyX')
  await until(async () => /along local X$/.test(await header()), async () => `X again: along local X (${await header()})`)
  await page.keyboard.press('KeyX')
  await until(async () => /^Dx: /.test(await header()), async () => `a third X frees it (${await header()})`)
  await page.keyboard.press('KeyX')
  await page.keyboard.press('Enter')
  await until(async () => (await undo()) === 'Move', 'Enter confirms')
  const alongX = (await named('Cube')).location
  assert.ok(Math.abs(alongX[0]) > 0.1 && alongX[1] === 0 && alongX[2] === 0, `only X moved (${alongX})`)
  await reset()
  item(6, 'G X, X again local, a third time free')

  // 7. Typed values: G Z 2 shows [2|] = 2 m; minus negates; Backspace takes a digit back; Tab
  // goes to the next component.
  await begin('KeyG', start)
  await page.keyboard.press('KeyZ')
  await page.keyboard.press('Digit2')
  await until(async () => (await header()).includes('[2|] = 2.0000 m'), async () => `D: [2|] = 2 m (${await header()})`)
  await page.keyboard.press('Digit5')
  await page.keyboard.press('Backspace')
  await page.keyboard.press('Minus')
  await until(async () => (await header()).includes('[-(2|)] = -2.0000 m'), async () => `minus negates, Backspace took the 5 back (${await header()})`)
  await shot('07-typed')
  await page.keyboard.press('Enter')
  await until(async () => nearAll((await named('Cube')).location, [0, 0, -2]), 'G Z - 2 moves 2 down')
  await reset()
  await begin('KeyG', start)
  for (const key of ['Digit1', 'Tab', 'Digit3']) await page.keyboard.press(key)
  await until(async () => /Dx: 1\.0000 m .*Dy: \[3\|\]/.test(await header()), async () => `Tab goes on to Dy (${await header()})`)
  await page.keyboard.press('Enter')
  await until(async () => nearAll((await named('Cube')).location, [1, 3, 0]), async () => `G 1 Tab 3 moves by (1, 3, 0) (${(await named('Cube')).location})`)
  await reset()
  item(7, 'typed values: G Z 2, minus, Backspace, Tab')

  // 8. Ctrl/Cmd+Z undoes one transform, Shift+Ctrl/Cmd+Z redoes it.
  const beforeMove = await steadyUndo()
  await begin('KeyG', start)
  for (const key of ['KeyY', 'Digit3', 'Enter']) await page.keyboard.press(key)
  await until(async () => (await undo()) === 'Move', 'a Move step')
  await page.keyboard.press(`${mod}+KeyZ`)
  await until(async () => (await named('Cube')).location[1] === 0 && (await undo()) === beforeMove, 'one undo takes the whole move back')
  await page.keyboard.press(`${mod}+Shift+KeyZ`)
  await until(async () => near((await named('Cube')).location[1], 3), 'redo moves it again')
  await reset()
  item(8, 'Ctrl/Cmd+Z and Shift+Ctrl/Cmd+Z, a transform one step')

  // 9. Tab enters edit mode: the header's menus, the select-mode buttons, the modelling tools.
  const headerMenus = () => kit.locator('[data-testid^="scene-header-menu-"]').evaluateAll((items) => items.map((entry) => entry.dataset.testid.replace('scene-header-menu-', '')))
  await page.mouse.move(centre[0], centre[1])
  await page.keyboard.press('Tab')
  await until(async () => (await state()).mode === 'edit', 'Tab enters edit mode')
  await until(async () => (await headerMenus()).join() === 'View,Select,Mesh,Vertex,Edge,Face,UV', async () => `edit mode's menus (${await headerMenus()})`)
  for (const kind of ['vertex', 'edge', 'face']) assert.equal(await kit.getByTestId(`scene-select-${kind}`).count(), 1, `the ${kind} button`)
  await until(async () => (await kit.getByTestId('scene-rail-operations').count()) === 1, 'the toolbar has the modelling tools')
  await shot('09-edit-mode')
  await page.keyboard.press('Tab')
  await until(async () => (await state()).mode === 'object' && (await headerMenus()).join() === 'View,Select,Add,Object', 'Tab leaves it')
  assert.equal(await kit.getByTestId('scene-rail-operations').count(), 0, 'without the modelling tools')
  item(9, 'Tab: the header, the select-mode buttons and the toolbar follow')

  // 10. 1 / 2 / 3 pick vertex, edge and face select; the selection converts.
  await page.keyboard.press('Tab')
  await until(async () => (await state()).mode === 'edit', 'edit mode')
  await page.keyboard.press('Digit3')
  await until(async () => (await state()).select === 'face', '3 is face select')
  await run([{ command: 'mesh.select', params: { faces: [5] } }])
  await until(async () => (await state()).faces.length === 1, 'the top face')
  await page.keyboard.press('Digit1')
  await until(async () => (await state()).select === 'vertex' && (await state()).vertices.length === 4, 'vertex select keeps the face\'s four corners')
  await page.keyboard.press('Digit2')
  await until(async () => (await state()).select === 'edge', '2 is edge select')
  await page.keyboard.press('Digit3')
  await until(async () => (await state()).select === 'face' && (await state()).faces.length === 1, 'back to the one face')
  await shot('10-face-select')
  note(10, 'selected vertices, edges and faces are all drawn in one orange; Blender has #FF7A00, #FFD800 and a see-through #FFB700')
  item(10, '1 / 2 / 3 select modes, the selection converts')

  // 11. S scales (Scale X: … Y: … Z: …), R rotates (Rotation: …); both take axis keys and numbers.
  await run([{ command: 'mode.set', params: { mode: 'object' } }])
  await reset()
  await begin('KeyS', [cube[0] + 120, cube[1]])
  await glide([cube[0] + 180, cube[1]])
  await until(async () => /^Scale X: .* Y: .* Z: /.test(await header()), async () => `the header reads Scale X: … (${await header()})`)
  for (const key of ['KeyX', 'Digit2', 'Enter']) await page.keyboard.press(key)
  await until(async () => nearAll((await named('Cube')).scale, [2, 1, 1]), 'S X 2 doubles X')
  await reset()
  await begin('KeyR', [cube[0] + 120, cube[1]])
  await glide([cube[0] + 100, cube[1] - 70])
  await until(async () => /^Rotation: -?\d/.test(await header()), async () => `the header reads Rotation: … (${await header()})`)
  await shot('11-rotate')
  for (const key of ['KeyZ', 'Digit9', 'Digit0', 'Enter']) await page.keyboard.press(key)
  await until(async () => nearAll((await named('Cube')).rotation, [0, 0, 90]), 'R Z 90')
  await reset()
  note(11, 'R R (trackball) is not there: R twice stays a rotation about the view (the plan left it out)')
  item(11, 'S and R with axis keys and numbers')

  // 12. A selects all, Alt+A and A twice deselect all.
  await page.mouse.move(centre[0], centre[1])
  await page.keyboard.press('KeyA')
  await until(async () => (await state()).selected.length === 3, 'A selects all')
  await page.keyboard.press('Alt+KeyA')
  await until(async () => (await state()).selected.length === 0, 'Alt+A deselects all')
  // Past the double press's time: this A is a press of its own.
  await page.waitForTimeout(600)
  await page.keyboard.press('KeyA')
  await until(async () => (await state()).selected.length === 3, 'A again')
  await page.waitForTimeout(600)
  await page.keyboard.press('KeyA')
  await page.keyboard.press('KeyA')
  await until(async () => (await state()).selected.length === 0, 'A twice deselects all')
  item(12, 'A, Alt+A, A twice')

  // 13. Shift+click adds and makes active; on the active object it takes it out.
  await reset()
  const light = await project((await named('Light')).location)
  const lightClick = async () => {
    await page.keyboard.down('Shift')
    await page.mouse.click(light[0], light[1])
    await page.keyboard.up('Shift')
  }
  await lightClick()
  await until(async () => (await selectedNames()).join() === 'Cube,Light' && (await state()).active === (await named('Light')).id, 'Shift+click adds the light, active')
  await shot('13-shift-click')
  await lightClick()
  await until(async () => (await selectedNames()).join() === 'Cube', 'Shift+click on the active light takes it out')
  item(13, 'Shift+click adds and takes out')

  // 14. X and Delete delete in object mode; X in edit mode opens the Delete menu.
  await reset()
  const history = await steadyUndo()
  await page.mouse.move(centre[0], centre[1])
  await page.keyboard.press('Delete')
  await until(async () => (await named('Cube')) == null, 'Delete deletes the cube')
  await back('Delete')
  await until(async () => (await named('Cube')) != null && (await undo()) === history, 'undo brings it back')
  await run([{ command: 'select.objects', params: { objects: ['Cube'] } }, { command: 'mode.set', params: { mode: 'edit' } }, { command: 'mesh.selectMode', params: { mode: 'face' } }, { command: 'mesh.select', params: { faces: [5] } }])
  await until(async () => (await state()).mode === 'edit' && (await state()).faces.length === 1, 'a face in edit mode')
  await page.mouse.move(centre[0], centre[1])
  await page.keyboard.press('KeyX')
  await kit.getByTestId('scene-delete-menu').waitFor()
  for (const row of ['mesh.deleteVertices', 'mesh.deleteFaces', 'mesh.dissolveFaces']) assert.equal(await kit.getByTestId(`scene-popup-${row}`).count(), 1, `the Delete menu has ${row}`)
  await shot('14-delete-menu')
  await page.keyboard.press('Escape')
  await until(async () => (await kit.getByTestId('scene-delete-menu').count()) === 0, 'Escape closes it')
  await run([{ command: 'mode.set', params: { mode: 'object' } }])
  note(14, 'X in object mode deletes at once, with no "Delete selected objects?" to confirm (web/tests/delete-focus.mjs asserts that it deletes)')
  item(14, 'Delete deletes; edit mode X opens the Delete menu')

  // 15. Shift+A at the pointer: the new object at the 3D cursor, selected; Adjust Last Operation
  // remakes it with another size in the same step.
  await reset()
  await run([{ command: 'cursor.set', params: { location: [3, 1, 0] } }])
  await until(async () => nearAll((await state()).transform.cursor, [3, 1, 0]), 'the cursor moved')
  const addAt = [centre[0] - 150, centre[1] - 40]
  await page.mouse.move(addAt[0], addAt[1])
  await page.keyboard.press('Shift+KeyA')
  await kit.getByTestId('scene-add-menu').waitFor()
  const menuBox = await kit.getByTestId('scene-add-menu').boundingBox()
  assert.ok(Math.hypot(menuBox.x - addAt[0], menuBox.y - addAt[1]) < 300, `the menu opens at the pointer (${menuBox.x}, ${menuBox.y})`)
  await kit.getByTestId('scene-popup-mesh').hover()
  await kit.getByTestId('scene-popup-object.addCube').click()
  await until(async () => (await named('Cube.001')) != null, 'Shift+A ▸ Mesh ▸ Cube')
  const added = await named('Cube.001')
  assert.ok(nearAll(added.location, [3, 1, 0]) && nearAll(added.rotation, [0, 0, 0]), `at the cursor, aligned to the world (${added.location})`)
  assert.deepEqual([(await state()).selected, (await state()).active], [[added.id], added.id], 'selected and active')
  await until(async () => (await operation())?.command === 'object.addCube', 'Adjust Last Operation shows Add Cube')
  if ((await kit.getByTestId('scene-adjust-last').getAttribute('data-open')) !== 'true') await kit.getByTestId('scene-adjust-last-title').click()
  const addedUndo = await steadyUndo()
  const size = kit.getByTestId('scene-adjust-size').locator('input')
  await size.fill('0.5')
  await size.press('Enter')
  await until(async () => near((await named('Cube.001'))?.dimensions?.[0] ?? 0, 0.5), async () => `the Size field remakes the cube 0.5 m across (${(await named('Cube.001'))?.dimensions})`)
  assert.equal((await state()).objects.filter((entry) => entry.name.startsWith('Cube')).length, 2, 'one new cube, not two')
  assert.equal(await steadyUndo(), addedUndo, 'in the same step')
  await shot('15-shift-a-adjust')
  await run([{ command: 'edit.undo', params: {} }, { command: 'cursor.reset', params: {} }])
  await until(async () => (await named('Cube.001')) == null, 'undone')
  item(15, 'Shift+A at the cursor, with Adjust Last Operation')

  // 16. E on a face extrudes and moves along its normal (D: … normal); a click confirms.
  await reset()
  await run([{ command: 'mode.set', params: { mode: 'edit' } }, { command: 'mesh.selectMode', params: { mode: 'face' } }, { command: 'mesh.select', params: { faces: [5] } }])
  await until(async () => (await state()).mode === 'edit' && (await state()).faces.length === 1, 'the top face')
  const top = await project([0, 0, 1])
  await begin('KeyE', top)
  await until(async () => (await vertexCount()) === 12, 'the extrusion shows at once')
  await glide([top[0], top[1] - 60])
  await until(async () => /^D: .* normal$/.test(await header()), async () => `D: … normal (${await header()})`)
  await shot('16-extrude')
  await page.mouse.click(top[0], top[1] - 60)
  await until(async () => (await undo()) === 'Extrude Faces', 'a click confirms one Extrude Faces step')
  await back('Extrude Faces')
  await until(async () => (await vertexCount()) === 8, 'undone')
  await run([{ command: 'mode.set', params: { mode: 'object' } }])
  item(16, 'E extrudes along the normal')

  // 17. The keypad's views, smoothly, orthographic; an orbit goes back to perspective; keypad 0 is
  // the camera; the navigation gizmo's balls.
  await reset()
  await page.mouse.move(centre[0], centre[1])
  const framesBefore = gpu ? await frame.evaluate(() => window.sceneGl.frames) : 0
  await page.keyboard.press('Numpad1')
  await viewIs((v) => near(v.tilt, 90) && near(v.turn, 0) && !v.perspective, 'keypad 1: the front, orthographic')
  if (gpu) assert.ok((await frame.evaluate(() => window.sceneGl.frames)) - framesBefore >= 3, 'drawn on its way (a smooth change)')
  await page.keyboard.press('Numpad3')
  await viewIs((v) => near(v.tilt, 90) && near(v.turn, 90), 'keypad 3: the right')
  await page.keyboard.press('Numpad7')
  await viewIs((v) => near(v.tilt, 0), 'keypad 7: the top')
  await page.mouse.down({ button: 'middle' })
  await page.mouse.move(centre[0] + 60, centre[1] + 30, { steps: 6 })
  await page.mouse.up({ button: 'middle' })
  await viewIs((v) => v.perspective, 'an orbit from the top goes back to perspective')
  await page.keyboard.press('Numpad5')
  await viewIs((v) => !v.perspective, 'keypad 5: orthographic')
  await page.keyboard.press('Numpad5')
  await viewIs((v) => v.perspective, 'and back')
  await kit.getByTestId('scene-nav-view.right').click()
  await viewIs((v) => near(v.tilt, 90) && near(v.turn, 90), "the gizmo's X ball: the right")
  await page.mouse.move(centre[0], centre[1])
  await page.keyboard.press('Numpad0')
  await viewIs((v) => v.camera != null, 'keypad 0: through the camera')
  await shot('17-camera-view')
  await page.keyboard.press('Numpad0')
  await viewIs((v) => v.camera == null, 'keypad 0 again leaves it')
  item(17, 'keypad 1 / 3 / 7 / 5 / 0 and the navigation gizmo')

  // 18. Keypad . frames the selection; Home frames everything.
  await run([{ command: 'select.objects', params: { objects: ['Light'] } }])
  await until(async () => (await selectedNames()).join() === 'Light', 'the light')
  await page.mouse.move(centre[0], centre[1])
  await page.keyboard.press('NumpadDecimal')
  const lightAt = (await named('Light')).location
  await viewIs((v) => nearAll(v.target, lightAt, 1e-2), 'keypad . centres the light')
  const framed = await steadyView()
  await page.keyboard.press('Home')
  await viewIs((v) => !nearAll(v.target, lightAt, 1e-2) && v.distance > framed.distance, 'Home frames everything')
  item(18, 'keypad . and Home')
  await openingView()

  // 19. Shift+D: the copy follows the pointer; the right button leaves it where it was made.
  await reset()
  at = await project([0, 0, 0])
  await begin('Shift+KeyD', [at[0] + 40, at[1] + 20])
  await glide([at[0] + 180, at[1] + 20])
  await until(async () => (await named('Cube.001')) != null && (await modal()).changed === true, 'the copy is there and follows the pointer')
  await shot('19-duplicate')
  await page.mouse.click(at[0] + 180, at[1] + 20, { button: 'right' })
  await until(async () => (await undo()) === 'Duplicate', 'one Duplicate step')
  assert.deepEqual((await named('Cube.001')).location, [0, 0, 0], 'the right button leaves the copy where it was made')
  await back('Duplicate')
  await until(async () => (await named('Cube.001')) == null, 'undone')
  item(19, 'Shift+D moves the copy; the right button keeps it')

  // 20. A left drag boxes (drawn over the picture); Shift adds, Ctrl/Cmd takes out; B boxes once.
  await reset()
  // G, R and S above ran from the keyboard: when each ended, the tool from before came back.
  assert.equal((await details()).tool.id, 'select', 'Select Box is still the tool after G, R and S')
  // G, the pointer, a click: the Move tool the key picked gives way to Select Box again, so a
  // drag on empty space boxes instead of moving the cube.
  await begin('KeyG', start)
  await until(async () => (await details()).tool.id === 'move', 'G picks the move tool while it runs')
  await glide([start[0] + 80, start[1] + 20])
  await page.mouse.click(start[0] + 80, start[1] + 20)
  await until(async () => (await undo()) === 'Move', 'a click confirms the move')
  await until(async () => (await details()).tool.id === 'select', async () => `the tool from before comes back (${(await details()).tool.id})`)
  const movedTo = (await named('Cube')).location
  // Adjust Last Operation covers the bottom-left corner now: empty space higher up.
  const blank = [box.x + box.width * 0.2, box.y + box.height * 0.3]
  await page.mouse.click(blank[0], blank[1])
  await until(async () => (await state()).selected.length === 0, async () => `a click on nothing deselects (${JSON.stringify(await selectedNames())}, tool ${(await details()).tool.id})`)
  const there = await project(movedTo)
  await page.mouse.move(there[0] - 120, there[1] - 120)
  await page.mouse.down()
  await page.mouse.move(there[0], there[1], { steps: 3 })
  await page.mouse.move(there[0] + 120, there[1] + 120, { steps: 3 })
  await page.mouse.up()
  await until(async () => (await selectedNames()).join() === 'Cube', 'the drag on empty space boxes the cube')
  assert.deepEqual((await named('Cube')).location, movedTo, 'and moves nothing')
  await reset()
  await page.mouse.click(empty[0], empty[1])
  await until(async () => (await state()).selected.length === 0, 'nothing selected')
  at = await project([0, 0, 0])
  const lightNow = await project((await named('Light')).location)
  const boxDrag = async (from, to, modifier, hold) => {
    if (modifier) await page.keyboard.down(modifier)
    await page.mouse.move(from[0], from[1])
    await page.mouse.down()
    await page.mouse.move((from[0] + to[0]) / 2, (from[1] + to[1]) / 2, { steps: 3 })
    await page.mouse.move(to[0], to[1], { steps: 3 })
    if (hold) await hold()
    await page.mouse.up()
    if (modifier) await page.keyboard.up(modifier)
  }
  await boxDrag([at[0] - 70, at[1] - 70], [at[0] + 70, at[1] + 70], null, async () => {
    await until(async () => (await overlayInk('any')) > 200, 'the box is drawn over the picture')
    await shot('20-box')
  })
  await until(async () => (await selectedNames()).join() === 'Cube', 'a box selects the cube')
  await boxDrag([lightNow[0] - 30, lightNow[1] - 30], [lightNow[0] + 30, lightNow[1] + 30], 'Shift')
  await until(async () => (await selectedNames()).join() === 'Cube,Light', 'Shift+box adds the light')
  await boxDrag([at[0] - 50, at[1] - 50], [at[0] + 50, at[1] + 50], mod)
  await until(async () => (await selectedNames()).join() === 'Light', 'Ctrl/Cmd+box takes the cube out')
  await page.mouse.move(centre[0], centre[1])
  await page.keyboard.press('KeyB')
  await boxDrag([at[0] - 50, at[1] - 50], [at[0] + 50, at[1] + 50])
  await until(async () => (await selectedNames()).join() === 'Cube', 'B then a drag boxes')
  item(20, 'box select, Shift and Ctrl/Cmd, B')

  // 21. Z's pie (a click, or Z held toward an item and let go); Alt+Z X-ray; Shift+Z wireframe.
  await page.mouse.move(centre[0], centre[1])
  await page.keyboard.press('KeyZ')
  await kit.getByTestId('scene-pie').waitFor()
  await shot('21-pie')
  await kit.getByTestId('scene-pie-wireframe').click()
  await until(async () => (await state()).shading === 'wireframe', 'the pie: Wireframe')
  await page.mouse.move(centre[0] - 100, centre[1])
  await page.keyboard.down('KeyZ')
  await kit.getByTestId('scene-pie').waitFor()
  await page.mouse.move(centre[0] - 100, centre[1] + 60, { steps: 4 })
  await until(async () => (await kit.getByTestId('scene-pie').getAttribute('data-toward')) === 'bottom', 'down lights Material Preview')
  await page.keyboard.up('KeyZ')
  await until(async () => (await state()).shading === 'material', 'letting Z go chooses Material Preview')
  await page.keyboard.press('Shift+KeyZ')
  await until(async () => (await state()).shading === 'wireframe', 'Shift+Z: wireframe')
  await page.keyboard.press('Shift+KeyZ')
  await until(async () => (await state()).shading !== 'wireframe', 'Shift+Z again: back')
  await page.keyboard.press('Alt+KeyZ')
  await until(async () => (await state()).xray === true, 'Alt+Z: X-ray')
  await page.keyboard.press('Alt+KeyZ')
  await until(async () => (await state()).xray === false, 'Alt+Z again: off')
  await run([{ command: 'view.set', params: { shading: 'solid' } }])
  await until(async () => (await state()).shading === 'solid', 'solid')
  item(21, "Z's pie, Shift+Z and Alt+Z")

  // 22. Ctrl/Cmd+R: the yellow loop under the pointer, the wheel's cuts, a click cuts and slides
  // (Edge Slide: …), the right button leaves it in the middle.
  await reset()
  await run([{ command: 'mode.set', params: { mode: 'edit' } }, { command: 'edit.deselectAll', params: {} }])
  await until(async () => (await state()).mode === 'edit', 'edit mode')
  const side = await project([1, -1, 0])
  await page.mouse.move(side[0], side[1])
  await page.keyboard.press(`${mod}+KeyR`)
  await until(async () => (await modal()).kind === 'loopCut', 'Ctrl/Cmd+R starts the loop cut')
  await glide([side[0] + 1, side[1] + 1], 2)
  await until(async () => Array.isArray((await modal()).edge), 'the edge under the pointer is found')
  if (gpu) await until(async () => (await overlayInk('yellow')) > 50, 'the loop is drawn in yellow')
  await page.mouse.wheel(0, -100)
  await until(async () => /^Cuts: 2/.test(await header()), async () => `the wheel: two cuts (${await header()})`)
  await shot('22-loop-cut')
  await page.mouse.click(side[0] + 1, side[1] + 1)
  await until(async () => (await modal()).kind === 'slide' && /^Edge Slide: /.test(await header()), async () => `a click cuts and slides (${await header()})`)
  await glide([side[0] + 1, side[1] - 40])
  await page.mouse.click(side[0] + 1, side[1] - 40, { button: 'right' })
  await until(async () => (await undo()) === 'Loop Cut' && (await vertexCount()) === 16, 'one Loop Cut step, two loops')
  await back('Loop Cut')
  note(22, 'the header reads "Cuts: 2", without Blender\'s ", Smoothness: 0.00" (no smoothness)')
  item(22, 'Ctrl/Cmd+R loop cut and slide')

  // 23. Ctrl/Cmd+B: the pointer's distance is the width, the wheel the segments.
  await run([{ command: 'mesh.select', params: { vertices: [1, 5] } }])
  const edge = await project([0, -1, 1])
  await page.mouse.move(edge[0], edge[1] - 20)
  await page.keyboard.press(`${mod}+KeyB`)
  await until(async () => (await modal()).kind === 'bevel', 'Ctrl/Cmd+B starts the bevel')
  await glide([edge[0], edge[1] - 70])
  await until(async () => /^Offset: .* m, Segments: 1$/.test(await header()), async () => `Offset: … m, Segments: 1 (${await header()})`)
  await page.mouse.wheel(0, -100)
  await until(async () => /Segments: 2$/.test(await header()), 'the wheel adds a segment')
  await shot('23-bevel')
  await page.mouse.click(edge[0], edge[1] - 70)
  await until(async () => (await undo()) === 'Bevel Edges', 'one Bevel Edges step')
  await back('Bevel Edges')
  note(23, 'no Profile Shape (the header has no ", Profile Shape: 0.5")')
  item(23, 'Ctrl/Cmd+B bevel with the wheel')

  // 24. I: Thickness and Depth in the header; Ctrl changes the depth.
  await run([{ command: 'mesh.selectMode', params: { mode: 'face' } }, { command: 'mesh.select', params: { faces: [5] } }])
  await until(async () => (await state()).faces.length === 1, 'the top face')
  await page.mouse.move(top[0] + 160, top[1])
  await page.keyboard.press('KeyI')
  await until(async () => (await modal()).kind === 'inset', 'I starts the inset')
  await glide([top[0] + 100, top[1]])
  await until(async () => /^Thickness: .* m, Depth: 0\.0000 m$/.test(await header()), async () => `Thickness: … Depth: … (${await header()})`)
  await page.keyboard.down(mod)
  await glide([top[0] + 60, top[1]])
  await until(async () => Math.abs((await modal()).values?.depth ?? 0) > 0.05, 'Ctrl/Cmd changes the depth')
  await page.keyboard.up(mod)
  await shot('24-inset')
  await page.keyboard.press('Enter')
  await until(async () => (await undo()) === 'Inset Faces', 'one Inset Faces step')
  await back('Inset Faces')
  await run([{ command: 'mode.set', params: { mode: 'object' } }])
  note(24, 'no O (outset), B (boundary) or I (individual) switches; the inset is always per face')
  item(24, 'I insets with Thickness and Depth')

  // 25. Alt+click selects an edge loop, Ctrl/Cmd+click the shortest path, L the linked part.
  await run([
    { command: 'object.addUvSphere', params: { name: 'Ball', radius: 2, location: [0, 0, 6] } },
    { command: 'select.objects', params: { objects: ['Ball'] } },
    { command: 'view.frameSelected', params: {} },
    { command: 'mode.set', params: { mode: 'edit' } },
    { command: 'mesh.selectMode', params: { mode: 'vertex' } },
    { command: 'mesh.select', params: { vertices: [] } },
  ])
  await until(async () => (await state()).mode === 'edit' && (await state()).vertices.length === 0, 'the sphere in edit mode')
  await steadyView()
  const orbit = (await state()).orbit
  const facing = Math.atan2(-Math.cos(orbit.turn), Math.sin(orbit.turn))
  const segment = (Math.PI * 2) / 32
  const toward = Math.round(facing / segment) * segment
  const edgeMiddle = await project([2 * Math.cos(toward + segment / 2) * Math.cos(segment / 2), 2 * Math.sin(toward + segment / 2) * Math.cos(segment / 2), 6])
  await page.mouse.move(edgeMiddle[0], edgeMiddle[1])
  await page.keyboard.down('Alt')
  await page.mouse.click(edgeMiddle[0], edgeMiddle[1])
  await page.keyboard.up('Alt')
  await until(async () => (await state()).vertices.length === 32, async () => `Alt+click selects the equator's 32 vertices (${(await state()).vertices.length})`)
  await shot('25-loop')
  await run([{ command: 'mesh.select', params: { vertices: [] } }])
  await until(async () => (await state()).vertices.length === 0, 'nothing selected')
  await page.mouse.move(edgeMiddle[0], edgeMiddle[1])
  await page.keyboard.press('KeyL')
  await until(async () => (await state()).vertices.length === (await vertexCount('Ball')), 'L selects the linked part under the pointer: the whole sphere')
  await run([{ command: 'mesh.select', params: { vertices: [] } }])
  await until(async () => (await state()).vertices.length === 0, 'nothing selected')
  // Two clicks a few vertices apart along the equator: Ctrl/Cmd+click adds the path between.
  const onEquator = async (angle) => project([2 * Math.cos(angle), 2 * Math.sin(angle), 6])
  const first = await onEquator(toward - segment * 2)
  const last = await onEquator(toward + segment * 2)
  await page.mouse.click(first[0], first[1])
  await until(async () => (await state()).vertices.length === 1, 'a click selects a vertex')
  await page.keyboard.down(mod)
  await page.mouse.click(last[0], last[1])
  await page.keyboard.up(mod)
  await until(async () => (await state()).vertices.length === 5, async () => `Ctrl/Cmd+click adds the shortest path, five vertices (${(await state()).vertices.length})`)
  await run([{ command: 'mode.set', params: { mode: 'object' } }, { command: 'select.objects', params: { objects: ['Ball'] } }, { command: 'edit.delete', params: {} }, { command: 'view.frameAll', params: {} }])
  await until(async () => (await named('Ball')) == null, 'the sphere is gone')
  await steadyView()
  item(25, 'Alt+click loop, Ctrl/Cmd+click path, L linked')

  // 26. N's side panel: the Item location typed with units and arithmetic, Tab to the next field,
  // a label drag, Backspace over a field resets it.
  await reset()
  await page.mouse.move(centre[0], centre[1])
  await page.keyboard.press('KeyN')
  await until(async () => (await kit.getByTestId('scene-side').count()) === 0, 'N hides the side panel')
  await page.keyboard.press('KeyN')
  await until(async () => (await kit.getByTestId('scene-side').count()) === 1, 'N shows it')
  const locationX = kit.getByTestId('scene-location-x').locator('input')
  await locationX.click()
  await page.keyboard.press(`${mod}+KeyA`)
  await page.keyboard.type('50cm + 1m')
  await page.keyboard.press('Tab')
  await until(async () => near((await named('Cube')).location[0], 1.5), async () => `50cm + 1m is 1.5 m (${(await named('Cube')).location})`)
  await until(async () => frame.evaluate(() => document.activeElement?.closest?.('[data-testid="scene-location-y"]') != null), 'Tab goes to the Y field')
  await page.keyboard.type('2*2')
  await page.keyboard.press('Enter')
  await until(async () => near((await named('Cube')).location[1], 4), 'Y 2*2')
  await kit.getByTestId('scene-location-z-label').scrollIntoViewIfNeeded()
  const label = await kit.getByTestId('scene-location-z-label').boundingBox()
  await page.mouse.move(label.x + label.width / 2, label.y + label.height / 2)
  await page.mouse.down()
  for (let step = 1; step <= 4; step += 1) {
    await page.mouse.move(label.x + label.width / 2 + step * 10, label.y + label.height / 2)
    await page.waitForTimeout(30)
  }
  await page.mouse.up()
  await until(async () => (await named('Cube')).location[2] > 0.5, 'a label drag changes Z')
  await shot('26-item')
  const fieldBox = await kit.getByTestId('scene-location-y').boundingBox()
  await page.mouse.move(fieldBox.x + fieldBox.width * 0.7, fieldBox.y + fieldBox.height / 2)
  await page.keyboard.press('Backspace')
  await until(async () => (await named('Cube')).location.every((value) => value === 0), 'Backspace over the field resets the location')
  item(26, 'N, typed units and arithmetic, Tab, a label drag, Backspace')

  // 27. The Move gizmo: a handle lights up under the pointer, an arrow keeps to its axis, a square
  // to its plane, the middle moves freely; Ctrl/Cmd steps a handle's drag.
  await reset()
  await kit.getByTestId('scene-tool-move').click()
  const gizmo = () => frame.evaluate(() => window.sceneGizmo)
  await until(async () => (await gizmo())?.kind === 'move' && (await gizmo()).handles['move-x'] != null, 'the move tool shows its gizmo')
  const onPage = (point) => [box.x + point[0] / dpr, box.y + point[1] / dpr]
  const shaft = async (name) => {
    const handles = (await gizmo()).handles
    const middle = await project((await state()).transform.gizmo.centre)
    const end = onPage(handles[name])
    return [middle[0] + (end[0] - middle[0]) * 0.75, middle[1] + (end[1] - middle[1]) * 0.75]
  }
  /** The gizmo once it is drawn at the cube put back at the origin (its handles follow a frame later). */
  const gizmoBack = async () => {
    await until(async () => nearAll((await state()).transform.gizmo?.centre ?? [9, 9, 9], [0, 0, 0]), 'the gizmo is at the cube again')
    let last = JSON.stringify((await gizmo())?.handles)
    for (let same = 0, tries = 0; same < 3 && tries < 40; tries += 1) {
      await page.waitForTimeout(100)
      const now = JSON.stringify((await gizmo())?.handles)
      same = now === last ? same + 1 : 0
      last = now
    }
  }
  const handleDrag = async (from, by, modifier) => {
    await page.mouse.move(from[0], from[1])
    if (modifier) await page.keyboard.down(modifier)
    await page.mouse.down()
    await until(active, 'the handle starts a transform')
    await page.mouse.move(from[0] + by[0], from[1] + by[1], { steps: 10 })
    await page.waitForTimeout(150)
    await page.mouse.up()
    if (modifier) await page.keyboard.up(modifier)
    await until(async () => !(await active()), 'the release ends it')
  }
  const xShaft = await shaft('move-x')
  await page.mouse.move(xShaft[0], xShaft[1])
  await until(async () => (await gizmo()).hovered === 'move-x', 'the X arrow lights up under the pointer')
  await shot('27-gizmo')
  await handleDrag(xShaft, [70, 0])
  await until(async () => (await undo()) === 'Move', 'the release confirms one Move step')
  let moved = (await named('Cube')).location
  assert.ok(Math.abs(moved[0]) > 0.2 && moved[1] === 0 && moved[2] === 0, `the X arrow keeps to X (${moved})`)
  await reset()
  await gizmoBack()
  await handleDrag(onPage((await gizmo()).handles['move-plane-z']), [40, 30])
  await until(async () => (await undo()) === 'Move', 'the square drag is a Move step')
  moved = (await named('Cube')).location
  assert.ok(moved[2] === 0 && Math.abs(moved[0]) + Math.abs(moved[1]) > 0.2, `the XY square keeps z (${moved})`)
  await reset()
  await gizmoBack()
  await handleDrag(await shaft('move-x'), [160, 0], mod)
  await until(async () => (await undo()) === 'Move', 'the stepped drag is a Move step')
  moved = (await named('Cube')).location
  assert.ok(Math.abs(moved[0]) >= 1 - 1e-4 && near(moved[0], Math.round(moved[0]), 1e-4), `Ctrl/Cmd steps the arrow by whole metres (${moved})`)
  await reset()
  at = await project([0, 0, 0])
  await page.mouse.move(at[0], at[1])
  await page.mouse.down()
  await page.mouse.move(at[0] + 50, at[1] + 20, { steps: 8 })
  await page.waitForTimeout(150)
  await page.mouse.up()
  await until(async () => (await undo()) === 'Move', 'the middle drags freely')
  assert.ok((await named('Cube')).location.filter((value) => value !== 0).length >= 2, 'in the view plane')
  await reset()
  await page.mouse.move(centre[0], centre[1])
  await page.keyboard.press('KeyW')
  await until(async () => (await details()).tool.id === 'select', 'W: back to the select tool')
  item(27, 'the Move gizmo: hover, an axis, a plane, the middle, Ctrl/Cmd steps')

  // 28. Ctrl/Cmd held: a move in 1 m steps, a rotation in 5°, a scale in 0.1; with Shift as well
  // 0.1 m, 1° and 0.01.
  const stepped = async (key, axisKey, to, read, check, message, shift = false) => {
    await reset()
    const from = [cube[0] + 100, cube[1] + 10]
    await begin(key, from)
    if (axisKey) await page.keyboard.press(axisKey)
    await page.keyboard.down(mod)
    if (shift) await page.keyboard.down('Shift')
    await glide(to(from))
    await page.mouse.move(to(from)[0] + 1, to(from)[1])
    await page.waitForTimeout(150)
    await page.keyboard.press('Enter')
    if (shift) await page.keyboard.up('Shift')
    await page.keyboard.up(mod)
    await until(async () => !(await active()) && (await undo()) !== base, 'confirmed')
    const value = read(await named('Cube'))
    assert.ok(check(value), `${message} (${value})`)
  }
  const whole = (value, step) => Math.abs(value / step - Math.round(value / step)) < 1e-3
  await stepped('KeyG', 'KeyX', (from) => [from[0] + 230, from[1]], (o) => o.location[0], (v) => Math.abs(v) >= 1 && whole(v, 1), 'a move in whole metres')
  await stepped('KeyR', 'KeyZ', (from) => [from[0] - 60, from[1] - 90], (o) => o.rotation[2], (v) => Math.abs(v) >= 5 && whole(v, 5), 'a rotation in 5° steps')
  await stepped('KeyS', null, (from) => [from[0] + 90, from[1]], (o) => o.scale[0], (v) => !near(v, 1) && whole(v, 0.1), 'a scale in steps of 0.1')
  await stepped('KeyG', 'KeyX', (from) => [from[0] + 230, from[1]], (o) => o.location[0], (v) => Math.abs(v) > 0.05 && whole(v, 0.1), 'Shift too: 0.1 m steps', true)
  await stepped('KeyR', 'KeyZ', (from) => [from[0] - 60, from[1] - 90], (o) => o.rotation[2], (v) => Math.abs(v) >= 1 && whole(v, 1), 'Shift too: 1° steps', true)
  await stepped('KeyS', null, (from) => [from[0] + 90, from[1]], (o) => o.scale[0], (v) => !near(v, 1) && whole(v, 0.01), 'Shift too: 0.01 steps', true)
  await reset()
  item(28, 'Ctrl/Cmd steps, and finer with Shift')

  // 29. The right button's menu follows the mode; F3 finds the menus' rows.
  await page.mouse.click(centre[0], centre[1], { button: 'right' })
  await kit.getByTestId('scene-context-menu').waitFor()
  await until(async () => (await kit.getByTestId('scene-context-title').getAttribute('data-kind')) === 'object', "object mode's menu")
  await page.keyboard.press('Escape')
  await until(async () => (await kit.getByTestId('scene-context-menu').count()) === 0, 'closed')
  await run([{ command: 'mode.set', params: { mode: 'edit' } }, { command: 'mesh.selectMode', params: { mode: 'face' } }, { command: 'mesh.select', params: { faces: [5] } }])
  await until(async () => (await state()).mode === 'edit', 'edit mode')
  await page.mouse.click(centre[0], centre[1], { button: 'right' })
  await kit.getByTestId('scene-context-menu').waitFor()
  await until(async () => (await kit.getByTestId('scene-context-title').getAttribute('data-kind')) === 'edit-face', "edit mode's face menu")
  await shot('29-context-edit')
  await page.keyboard.press('Escape')
  await until(async () => (await kit.getByTestId('scene-context-menu').count()) === 0, 'closed')
  await page.mouse.move(centre[0], centre[1])
  await page.keyboard.press('F3')
  await kit.getByTestId('scene-search').waitFor()
  await kit.getByTestId('scene-search-field').fill('loop cut')
  await until(async () => (await kit.getByTestId('scene-search-item').first().getAttribute('data-id')) === 'mesh.loopCut', 'F3 finds Loop Cut')
  await kit.getByTestId('scene-search-field').fill('subdivide')
  await until(async () => (await kit.getByTestId('scene-search-item').first().getAttribute('data-id'))?.startsWith('mesh.subdivide'), 'and Subdivide')
  await shot('29-search')
  await page.keyboard.press('Escape')
  await until(async () => (await kit.getByTestId('scene-search').count()) === 0, 'closed')
  await run([{ command: 'mode.set', params: { mode: 'object' } }])
  item(29, 'right-click menus by mode, F3')

  // 30. The outliner: a click selects (the viewport agrees), a double click renames, the eye hides;
  // H and Alt+H; Ctrl+P parents (Control+P on a Mac); Ctrl/Cmd+J joins; Space plays; I keys.
  await reset()
  const row = (name) => kit.locator(`[data-testid="scene-outliner-item"][data-name="${name}"] .scene-outliner-name`)
  await row('Light').click()
  await until(async () => (await selectedNames()).join() === 'Light' && (await state()).active === (await named('Light')).id, 'a row click selects the light')
  await row('Cube').dblclick()
  await kit.getByTestId('scene-rename').waitFor()
  await kit.getByTestId('scene-rename').fill('Box')
  await kit.getByTestId('scene-rename').press('Enter')
  await until(async () => (await named('Box')) != null, 'a double click renames')
  await run([{ command: 'edit.undo', params: {} }])
  await until(async () => (await named('Cube')) != null, 'undone')
  await kit.getByTestId('scene-outliner-eye-Light').click()
  await until(async () => (await named('Light')).hidden === true, 'the eye hides the light')
  await page.mouse.move(centre[0], centre[1])
  await focusEditor()
  await page.keyboard.press('Alt+KeyH')
  await until(async () => (await named('Light')).hidden !== true, 'Alt+H shows it')
  await reset()
  await page.mouse.move(centre[0], centre[1])
  await page.keyboard.press('KeyH')
  await until(async () => (await named('Cube')).hidden === true, 'H hides the cube')
  await page.keyboard.press('Alt+KeyH')
  await until(async () => (await named('Cube')).hidden !== true, 'Alt+H shows it')
  await shot('30-outliner')
  // Ctrl+P: the light parented to the cube (Control+P on a Mac: Cmd+P is the host's).
  await run([{ command: 'select.objects', params: { objects: ['Light'] } }, { command: 'select.objects', params: { objects: ['Cube'], extend: true } }])
  await until(async () => (await state()).active === (await named('Cube')).id && (await state()).selected.length === 2, 'the light and the cube, the cube active')
  await page.mouse.move(centre[0], centre[1])
  await page.keyboard.press('Control+KeyP')
  await kit.getByTestId('scene-parent-menu').waitFor()
  await kit.getByTestId('scene-popup-parent-object').click()
  await until(async () => (await named('Light')).parent === (await named('Cube')).id, 'Ctrl+P ▸ Object parents the light')
  await run([{ command: 'edit.undo', params: {} }])
  await until(async () => (await named('Light')).parent == null, 'undone')
  // Ctrl/Cmd+J joins two meshes.
  await run([{ command: 'object.addCube', params: { name: 'Other', size: 1, location: [3, 0, 0] } }, { command: 'select.objects', params: { objects: ['Other'] } }, { command: 'select.objects', params: { objects: ['Cube'], extend: true } }])
  await until(async () => (await named('Other')) != null && (await state()).selected.length === 2, 'two cubes selected')
  await page.mouse.move(centre[0], centre[1])
  await page.keyboard.press(`${mod}+KeyJ`)
  await until(async () => (await named('Other')) == null && (await vertexCount()) === 16, 'Ctrl/Cmd+J joins them into the cube')
  await run([{ command: 'edit.undo', params: {} }, { command: 'edit.undo', params: {} }])
  await until(async () => (await named('Other')) == null && (await vertexCount()) === 8, 'undone')
  // I keys location, rotation and scale with no menu; a tap of Space plays and pauses.
  await reset()
  await page.mouse.move(centre[0], centre[1])
  await focusEditor()
  await page.keyboard.press('KeyI')
  await until(async () => (await named('Cube')).animated?.join() === 'location,rotation,scale', 'I keys location, rotation and scale')
  const play = kit.getByTestId('scene-timeline-play')
  await page.keyboard.press('Space')
  await until(async () => (await play.getAttribute('data-playing')) === 'true', 'a tap of Space plays')
  await page.keyboard.press('Space')
  await until(async () => (await play.getAttribute('data-playing')) == null, 'and pauses')
  await run([{ command: 'animation.clear', params: {} }, { command: 'frame.set', params: { frame: 1 } }])
  await until(async () => !(await named('Cube')).animated?.length, 'keys cleared')
  note(30, 'no collections, so no M (Move to Collection); Ctrl+P is Control+P on a Mac only (Cmd+P is the host\'s)')
  item(30, 'the outliner, H / Alt+H, Ctrl+P, Ctrl/Cmd+J, Space, I')

  assert.deepEqual(errors, [], 'no page errors')
  console.log(`first-ten-minutes: all passed (${which})`)
} catch (error) {
  console.log(said.slice(-40).join('\n'))
  if (shots) await page.screenshot({ path: `${shots}/failure.png` }).catch(() => undefined)
  throw error
} finally {
  await browser.close()
  await server.close()
}
