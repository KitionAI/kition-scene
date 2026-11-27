// The modal transform by hand (plan scene-usable-by-hand, phase 2): node tests/modal.mjs [chromium|webkit]
// Real mouse and keyboard events on the development host, read back through the engine's inspect:
// G, R and S without a button held, X / Y / Z and Shift+axis, typed values (G Z 2, G Z - 2, R Z 90,
// S X 2, S Z 0.05), Ctrl's steps, the right button cancelling, one undo step per transform, X during
// a tool's drag, Shift+D moving the copies, a face moved in edit mode, Alt+G / Alt+R / Alt+S, the
// header and the axis line over the picture.
//
// The GPU viewport is asked for even on a GPU emulated on the CPU (`kition.scene.gl` = `on`), as
// basics.mjs does; KITION_SCENE_GL=off runs it on the software viewport.
import assert from 'node:assert/strict'
import { chromium, webkit } from 'playwright'
import { createServer } from 'vite'

const which = process.argv[2] ?? 'chromium'
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
const details = async () => (await page.evaluate(async () => window.devHost.connection().ai.inspect())).details
const named = async (name) => (await details()).objects.find((item) => item.name === name)
const round = (values) => values.map((value) => Math.round(value * 1000) / 1000)
const close = (a, b, tolerance = 1e-3) => a.length === b.length && a.every((value, index) => Math.abs(value - b[index]) <= tolerance)

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
  const run = (steps) => frame.evaluate((steps) => window.kitScene.run(steps), steps)
  const modal = () => frame.evaluate(() => window.sceneModal?.answer ?? { active: false })
  const header = () => kit.getByTestId('scene-modal-header').textContent().catch(() => '')
  const active = async () => (await modal()).active === true
  const ended = async () => !(await active()) && (await kit.getByTestId('scene-modal-header').count()) === 0
  const undo = async () => (await details()).undo

  // The cube selected, at the origin, seen from the front-right as the scene opens.
  await run([{ command: 'select.objects', params: { objects: ['Cube'] } }])
  await until(async () => (await details()).selection.objects.length === 1, 'the cube is selected')
  const centre = [box.x + box.width / 2, box.y + box.height / 2]
  const start = [centre[0] + 60, centre[1] + 40]
  const base = await undo()

  /** Moves the pointer in steps, waiting for the transform to take each move. */
  const glide = async (to, steps = 8) => {
    await page.mouse.move(to[0], to[1], { steps })
    await page.waitForTimeout(120)
  }
  /** Starts a transform with `key` where the pointer is, and waits until it runs. */
  const begin = async (key) => {
    await page.mouse.move(start[0], start[1])
    await page.keyboard.press(key)
    await until(active, `${key} starts a transform`)
  }
  const back = async (label) => {
    await page.keyboard.press(`${(await page.evaluate(() => /Mac|iPhone|iPad/i.test(navigator.platform))) ? 'Meta' : 'Control'}+KeyZ`)
    await until(async () => (await undo()) !== label, `undo takes the ${label} back`)
  }

  // G, the pointer, a left click: a Move step; one undo puts it back.
  await begin('KeyG')
  await glide([start[0] + 120, start[1] - 30])
  await until(async () => /^Dx: /.test(await header()), async () => `the header reads Dx: … (${await header()})`)
  assert.equal(await undo(), base, 'the transform under way is no step yet')
  await page.mouse.click(start[0] + 120, start[1] - 30)
  await until(async () => (await undo()) === 'Move', async () => `the click confirms one Move step (${await undo()})`)
  await until(ended, 'the transform ended')
  const moved = (await named('Cube')).location
  assert.ok(Math.hypot(...moved) > 0.3, `the cube moved (${moved})`)
  await back('Move')
  await until(async () => (await named('Cube')).location.join() === '0,0,0', 'one undo puts the cube back')
  assert.equal(await undo(), base, 'the undo stack is as before')
  passed('G follows the pointer and a click confirms one step')

  // G X: only X changes; the axis line shows over the picture.
  await begin('KeyG')
  await page.keyboard.press('KeyX')
  await until(async () => /along global X/.test(await header()), async () => `the header says along global X (${await header()})`)
  await glide([start[0] + 140, start[1] + 60])
  const ink = await frame.evaluate(() => {
    const canvas = document.querySelector('[data-testid="scene-tool-overlay"]')
    const data = canvas.getContext('2d').getImageData(0, 0, canvas.width, canvas.height).data
    let red = 0
    for (let index = 0; index < data.length; index += 4) if (data[index + 3] > 0 && data[index] > 200 && data[index + 1] < 120) red += 1
    return { red, modal: window.sceneToolOverlay?.modal }
  })
  assert.ok(ink.modal && ink.red > 200, `the X axis line is drawn in red over the picture (${JSON.stringify(ink)})`)
  await page.keyboard.press('Enter')
  await until(async () => (await undo()) === 'Move', 'Enter confirms')
  const alongX = (await named('Cube')).location
  assert.ok(Math.abs(alongX[0]) > 0.1 && alongX[1] === 0 && alongX[2] === 0, `only X moved (${alongX})`)
  await back('Move')
  passed('G X keeps the move to the X axis')

  // G Shift+Z: the plane across Z; Z stays.
  await begin('KeyG')
  await page.keyboard.press('Shift+KeyZ')
  await until(async () => /locking global Z/.test(await header()), async () => `the header says locking global Z (${await header()})`)
  await glide([start[0] + 100, start[1] - 80])
  await page.keyboard.press('Enter')
  await until(async () => (await undo()) === 'Move', 'Enter confirms')
  const plane = (await named('Cube')).location
  assert.ok(plane[2] === 0 && Math.hypot(plane[0], plane[1]) > 0.1, `X and Y moved, Z did not (${plane})`)
  await back('Move')
  passed('G Shift+Z keeps the move off the Z axis')

  // G Z 2 Enter: exactly 2 up; the header shows what is typed.
  await begin('KeyG')
  await glide([start[0] + 50, start[1] + 50])
  await page.keyboard.press('KeyZ')
  await page.keyboard.press('Digit2')
  await until(async () => (await header()).includes('[2|] = 2.0000 m'), async () => `the header shows the typed 2 (${await header()})`)
  await page.keyboard.press('Enter')
  await until(async () => (await undo()) === 'Move', 'Enter confirms the typed value')
  assert.deepEqual(round((await named('Cube')).location), [0, 0, 2], 'G Z 2 moves 2 up')
  await back('Move')
  // G Z - 2: minus negates.
  await begin('KeyG')
  for (const key of ['KeyZ', 'Minus', 'Digit2']) await page.keyboard.press(key)
  await until(async () => (await header()).includes('[-(2|)] = -2.0000 m'), async () => `the header shows -(2) (${await header()})`)
  await page.keyboard.press('Enter')
  await until(async () => (await undo()) === 'Move', 'Enter confirms')
  assert.deepEqual(round((await named('Cube')).location), [0, 0, -2], 'G Z - 2 moves 2 down')
  await back('Move')
  passed('typed values: G Z 2 and G Z - 2')

  // R Z 90, S X 2, S Z 0.05.
  await begin('KeyR')
  for (const key of ['KeyZ', 'Digit9', 'Digit0']) await page.keyboard.press(key)
  await until(async () => /^Rotation: \[90\|\]/.test(await header()), async () => `the header reads Rotation: [90|] (${await header()})`)
  await page.keyboard.press('Enter')
  await until(async () => (await undo()) === 'Rotate', 'a Rotate step')
  assert.deepEqual(round((await named('Cube')).rotation), [0, 0, 90], 'R Z 90 turns 90° about Z')
  await back('Rotate')
  await begin('KeyS')
  for (const key of ['KeyX', 'Digit2', 'Enter']) await page.keyboard.press(key)
  await until(async () => (await undo()) === 'Scale', 'a Scale step')
  assert.deepEqual(round((await named('Cube')).scale), [2, 1, 1], 'S X 2 doubles X')
  await back('Scale')
  await begin('KeyS')
  for (const key of ['KeyZ', 'Digit0', 'Period', 'Digit0', 'Digit5', 'Enter']) await page.keyboard.press(key)
  await until(async () => (await undo()) === 'Scale', 'a Scale step')
  assert.deepEqual(round((await named('Cube')).scale), [1, 1, 0.05], 'S Z 0.05 flattens Z')
  await back('Scale')
  passed('R Z 90, S X 2 and S Z 0.05')

  // Ctrl steps a move by whole metres.
  const step = (await page.evaluate(() => /Mac|iPhone|iPad/i.test(navigator.platform))) ? 'Meta' : 'Control'
  await begin('KeyG')
  await page.keyboard.press('KeyX')
  await page.keyboard.down(step)
  await glide([start[0] + 230, start[1]])
  await until(async () => (await modal()).step === true, 'the transform reads Ctrl')
  await page.keyboard.up(step)
  await page.mouse.move(start[0] + 230, start[1])
  await page.keyboard.down(step)
  await page.mouse.move(start[0] + 231, start[1])
  await page.waitForTimeout(150)
  await page.keyboard.press('Enter')
  await page.keyboard.up(step)
  await until(async () => (await undo()) === 'Move', 'a Move step')
  const stepped = (await named('Cube')).location
  assert.ok(Math.abs(stepped[0]) >= 1 && Math.abs(stepped[0] - Math.round(stepped[0])) < 1e-4, `Ctrl moved by whole metres (${stepped})`)
  await back('Move')
  passed('Ctrl steps the value')

  // The right button cancels: nothing moved, no step, nothing unsaved.
  const dirty = await page.evaluate(() => window.devHost.connection().ai.inspect().then((answer) => answer.dirty ?? null))
  await begin('KeyG')
  await glide([start[0] - 150, start[1] - 60])
  assert.ok((await modal()).changed, 'the transform moved the cube')
  await page.mouse.click(start[0] - 150, start[1] - 60, { button: 'right' })
  await until(ended, 'the right button ends the transform')
  await page.waitForTimeout(300)
  assert.equal((await named('Cube')).location.join(), '0,0,0', 'the cube is back')
  assert.equal(await undo(), base, 'no undo step')
  assert.equal(await kit.getByTestId('scene-context-menu').count(), 0, 'the right button opened no menu')
  if (dirty != null) assert.equal(await page.evaluate(() => window.devHost.connection().ai.inspect().then((answer) => answer.dirty ?? null)), dirty, 'cancelling leaves the document as it was')
  // Escape too.
  await begin('KeyR')
  await glide([start[0] + 40, start[1] - 120])
  await page.keyboard.press('Escape')
  await until(ended, 'Escape ends the transform')
  assert.equal((await named('Cube')).rotation.join(), '0,0,0', 'Escape puts the rotation back')
  assert.equal(await undo(), base, 'no undo step')
  passed('the right button and Escape cancel')

  // X during a Move tool's drag keeps it to X and deletes nothing: one Move step. The transforms
  // above began from the keys, and each one's end gave back the tool from before: Select Box.
  await until(async () => (await details()).tool.id === 'select', 'the select tool is back after the keys\' transforms')
  await kit.getByTestId('scene-tool-move').click()
  await until(async () => (await details()).tool.id === 'move', 'the move tool')
  const cube = centre
  await page.mouse.move(cube[0], cube[1])
  await page.mouse.down()
  await page.mouse.move(cube[0] + 40, cube[1] + 30, { steps: 4 })
  await page.keyboard.press('KeyX')
  await page.mouse.move(cube[0] + 120, cube[1] + 60, { steps: 6 })
  await page.waitForTimeout(150)
  await page.mouse.up()
  await until(async () => (await undo()) === 'Move', async () => `the drag is one Move step (${await undo()})`)
  const dragged = await named('Cube')
  assert.ok(dragged, 'X deleted nothing')
  assert.ok(Math.abs(dragged.location[0]) > 0.1 && dragged.location[1] === 0 && dragged.location[2] === 0, `the drag kept to X (${dragged.location})`)
  await back('Move')
  await page.keyboard.press('KeyW')
  await until(async () => (await details()).tool.id === 'select', 'W picks the select tool')
  passed('X during a drag keeps it to X')

  // Shift+D: the copy follows the pointer; one Duplicate step. The right button leaves the copy where it was made.
  await page.mouse.move(start[0], start[1])
  await page.keyboard.press('Shift+KeyD')
  await until(active, 'Shift+D starts moving the copy')
  await until(async () => (await details()).objects.some((item) => item.name === 'Cube.001') || (await frame.evaluate(() => window.kitScene.state()?.objects.some((item) => item.name === 'Cube.001'))), 'the copy is there')
  await glide([start[0] + 150, start[1]])
  await page.mouse.click(start[0] + 150, start[1])
  await until(async () => (await undo()) === 'Duplicate', async () => `one Duplicate step (${await undo()})`)
  const copy = await named('Cube.001')
  assert.ok(copy && Math.hypot(...copy.location) > 0.3, `the copy moved (${copy?.location})`)
  assert.equal((await named('Cube')).location.join(), '0,0,0', 'the original stayed')
  await back('Duplicate')
  await until(async () => !(await named('Cube.001')), 'one undo takes the copy away')
  await run([{ command: 'select.objects', params: { objects: ['Cube'] } }])
  await page.mouse.move(start[0], start[1])
  await page.keyboard.press('Shift+KeyD')
  await until(active, 'Shift+D starts moving the copy')
  await glide([start[0] + 150, start[1]])
  await page.mouse.click(start[0] + 150, start[1], { button: 'right' })
  await until(async () => (await undo()) === 'Duplicate', 'the right button keeps the copy, one Duplicate step')
  assert.equal((await named('Cube.001'))?.location.join(), '0,0,0', 'the copy stays where it was made')
  await back('Duplicate')
  await run([{ command: 'select.objects', params: { objects: ['Cube'] } }])
  passed('Shift+D moves the copy')

  // Alt+G, Alt+R, Alt+S clear the location, the rotation and the scale.
  await run([{ command: 'object.transform', params: { objects: ['Cube'], location: [1, 2, 3], rotation: [10, 20, 30], scale: [2, 2, 2] } }])
  await until(async () => (await named('Cube')).location.join() === '1,2,3', 'the cube is placed')
  for (const [key, field, value] of [['Alt+KeyG', 'location', '0,0,0'], ['Alt+KeyR', 'rotation', '0,0,0'], ['Alt+KeyS', 'scale', '1,1,1']]) {
    await page.mouse.move(start[0], start[1])
    await page.keyboard.press(key)
    await until(async () => (await named('Cube'))[field].join() === value, `${key} clears the ${field}`)
  }
  await run([{ command: 'object.transform', params: { objects: ['Cube'], location: [0, 0, 0], rotation: [0, 0, 0], scale: [1, 1, 1] } }])
  passed('Alt+G, Alt+R and Alt+S clear')

  // Edit mode: a face moved up by G Z 1.
  await run([
    { command: 'mode.set', params: { mode: 'edit' } },
    { command: 'mesh.selectMode', params: { mode: 'face' } },
    { command: 'mesh.select', params: { faces: [0] } },
  ])
  await until(async () => (await details()).mode.mode === 'edit', 'edit mode')
  const vertices = await frame.evaluate(() => window.kitScene.state().vertices)
  await begin('KeyG')
  for (const key of ['KeyZ', 'Digit1', 'Enter']) await page.keyboard.press(key)
  await until(async () => (await undo()) === 'Move', 'a Move step in edit mode')
  const dimensions = (await named('Cube')).dimensions
  assert.ok(Math.abs(dimensions[2] - 3) < 1e-3 || Math.abs(dimensions[2] - 1) < 1e-3, `the face moved 1 along Z (dimensions ${dimensions}, ${vertices.length} vertices selected)`)
  await back('Move')
  await until(async () => Math.abs((await named('Cube')).dimensions[2] - 2) < 1e-3, 'one undo puts the face back')
  await run([{ command: 'mode.set', params: { mode: 'object' } }])
  passed('a face moves in edit mode')

  assert.deepEqual(errors, [], 'no page errors')
  console.log(`modal: all passed (${which})`)
} finally {
  await browser.close()
  await server.close()
}
