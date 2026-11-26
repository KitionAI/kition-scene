// The viewport's basics by hand (plan scene-usable-by-hand, phase 1): node tests/basics.mjs [chromium|webkit]
// Real mouse and keyboard events on the development host, read back through the engine's inspect:
// the keyboard on open, keys during a drag, the tools' overlays over the GPU picture, the number
// fields' precision, label drags and focus, clicks and boxes with the transform and select tools,
// Cmd/Ctrl+click, the right click, the number row against the keypad, the outliner in edit mode,
// F2, the save note, a command with nothing to do, and a command that changes nothing.
//
// The GPU viewport is asked for even on a GPU emulated on the CPU (`kition.scene.gl` = `on`), so
// the overlay layer is tested where CI has no GPU; where WebGL2 is missing altogether the software
// picture shows the overlays and the test checks them there.
import assert from 'node:assert/strict'
import { chromium, webkit } from 'playwright'
import { createServer } from 'vite'

const which = process.argv[2] ?? 'chromium'
const server = await createServer({ root: new URL('..', import.meta.url).pathname, configFile: new URL('../vite.config.ts', import.meta.url).pathname, server: { port: 0, host: '127.0.0.1' }, logLevel: 'error' })
await server.listen()
const gpuArgs = process.platform === 'darwin' ? ['--use-angle=metal', '--enable-gpu', '--ignore-gpu-blocklist'] : ['--enable-unsafe-swiftshader', '--ignore-gpu-blocklist']
const browser = await (which === 'webkit' ? webkit : chromium).launch(which === 'webkit' ? {} : { args: gpuArgs })
const context = await browser.newContext({ viewport: { width: 1440, height: 900 } })
// KITION_SCENE_GL=off runs it on the software viewport (as flow.mjs).
await context.addInitScript((setting) => window.localStorage.setItem('kition.scene.gl', setting), process.env.KITION_SCENE_GL === 'off' ? 'off' : 'on')
const page = await context.newPage()
const errors = []
page.on('pageerror', (error) => errors.push(error.message))
// The editor's shortcuts take Cmd where the page says it runs on a Mac, else Ctrl (flow.mjs).
const mod = (await page.evaluate(() => /Mac|iPhone|iPad/i.test(navigator.platform))) ? 'Meta' : 'Control'
const until = async (check, message, timeout = 15000) => {
  const end = Date.now() + timeout
  while (Date.now() < end) {
    if (await check().catch(() => false)) return
    await page.waitForTimeout(50)
  }
  assert.fail(message)
}
const passed = (name) => console.log(`PASS ${name} (${which})`)
const details = async () => (await page.evaluate(async () => window.devHost.connection().ai.inspect())).details
const named = async (name) => (await details()).objects.find((item) => item.name === name)
const selectedNames = async () => {
  const state = await details()
  return state.objects.filter((item) => state.selection.objects.includes(item.id)).map((item) => item.name).sort()
}
const settle = () => page.waitForTimeout(250)

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

  // Where a world point is drawn (the editor's camera maths, web/src/gl/camera.ts; lens 50 on a
  // 72 mm sensor), in page pixels.
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
  const empty = [box.x + 40, box.y + box.height - 40]

  // B17: the page opens with the keyboard for the editor.
  await page.keyboard.press('KeyA')
  await until(async () => (await selectedNames()).length === 3, 'A right after the page opens selects everything')
  await page.keyboard.press('Alt+KeyA')
  await until(async () => (await selectedNames()).length === 0, 'Alt+A deselects')
  // Home frames everything (the light and the camera start outside the view).
  const distance = (await details()).view.distance
  await page.keyboard.press('Home')
  await until(async () => (await details()).view.distance !== distance, 'Home frames everything')
  await settle()
  passed('the keyboard works before any click')

  const cube = await project([0, 0, 0])
  await page.mouse.click(cube[0], cube[1])
  await until(async () => (await selectedNames()).join() === 'Cube', 'a click selects the cube')

  // B13: Cmd+S says what it saved, no placeholder.
  await page.keyboard.press(`${mod}+KeyS`)
  await kit.locator('.kui-toast').first().waitFor({ timeout: 10000 })
  const toasts = await kit.locator('.kui-toast').allTextContents()
  assert.ok(toasts.length && toasts.every((text) => !text.includes('{')), `the save's note has no placeholder: ${JSON.stringify(toasts)}`)
  await until(async () => (await kit.getByTestId('scene-unsaved').count()) === 0, 'saved')
  passed('Cmd+S names no placeholder')

  // A15: Apply Transform on a cube with nothing to apply leaves the file saved.
  await page.keyboard.press(`${mod}+KeyA`)
  await settle()
  await page.waitForTimeout(400)
  assert.equal(await kit.getByTestId('scene-unsaved').count(), 0, 'a command that changes nothing leaves the file saved')
  passed('a command that changes nothing is no unsaved change')

  // B4: typed values keep their digits; the field shows no trailing zeros. A9: Enter gives the
  // keyboard back to the editor.
  const field = (id) => kit.getByTestId(id).locator('input')
  await field('scene-location-x').fill('0.05')
  await field('scene-location-x').press('Enter')
  await until(async () => (await named('Cube')).location[0] === 0.05, 'location X keeps 0.05')
  assert.equal(await field('scene-location-x').inputValue(), '0.05')
  const focused = await frame.evaluate(() => document.activeElement?.className ?? '')
  assert.ok(String(focused).includes('kui-app'), `Enter gives the keyboard back to the editor (focus on ${focused})`)
  await field('scene-scale-z').fill('1.35')
  await field('scene-scale-z').press('Enter')
  await until(async () => (await named('Cube')).scale[2] === 1.35, 'scale Z keeps 1.35')
  await field('scene-rotation-z').fill('33.33')
  await field('scene-rotation-z').press('Enter')
  await until(async () => Math.abs((await named('Cube')).rotation[2] - 33.33) < 1e-3, 'rotation Z keeps 33.33°')
  await field('scene-location-y').fill('0.0004')
  await field('scene-location-y').press('Enter')
  await settle()
  assert.equal((await named('Cube')).location[1], 0, 'location stops at a millimetre')
  passed('number fields keep typed precision and give the keyboard back')

  // B5: dragging a field's label moves the cube at once and commits one step on release.
  const undoBefore = (await details()).undo
  const label = await kit.getByTestId('scene-location-x-label').boundingBox()
  await page.mouse.move(label.x + label.width / 2, label.y + label.height / 2)
  await page.mouse.down()
  for (let step = 1; step <= 6; step += 1) {
    await page.mouse.move(label.x + label.width / 2 + step * 10, label.y + label.height / 2)
    await page.waitForTimeout(30)
  }
  // The editor's own state shows the preview (the host's inspect reads the saved replica).
  const shown = () => frame.evaluate(() => window.kitScene.state().objects.find((item) => item.name === 'Cube').location[0])
  await until(async () => (await shown()) > 2, 'the cube follows the label drag at once')
  assert.equal((await details()).undo, undoBefore, 'a drag in progress is no undo step')
  await page.mouse.up()
  await until(async () => (await details()).undo === 'Set Transform' && (await named('Cube')).location[0] > 2, 'the release commits one step')
  const dragged = (await named('Cube')).location[0]
  await page.keyboard.press(`${mod}+KeyZ`)
  await until(async () => (await named('Cube')).location[0] === 0.05, `one undo takes the drag back (was ${dragged})`)
  passed('a label drag previews and commits one step')

  // A14: a field its row already names shows a grip, not its name again.
  await kit.getByTestId('scene-add-modifier').click()
  await kit.getByTestId('scene-add-modifier-subdivision').click()
  const levels = kit.getByTestId('scene-modifier-0-levels-label')
  await levels.waitFor()
  assert.equal((await levels.textContent()).trim(), '', 'the row says Levels once')
  assert.ok((await levels.getAttribute('class')).includes('scene-number-grip'))
  await page.keyboard.press(`${mod}+KeyZ`)
  await until(async () => !(await named('Cube')).modifiers?.length, 'undo takes the modifier back')
  passed('a field is not named twice')

  // B2: X during a drag of the move tool deletes nothing; the drag is one Move step.
  const moveFrom = await project([0.05, 0, 0])
  await page.mouse.click(moveFrom[0], moveFrom[1])
  await until(async () => (await selectedNames()).join() === 'Cube', `a click selects the cube (${JSON.stringify(await selectedNames())})`)
  await page.keyboard.press('KeyG')
  await until(async () => (await details()).tool.id === 'move', 'G picks the move tool')
  const before = (await named('Cube')).location
  await page.mouse.move(moveFrom[0], moveFrom[1])
  await page.mouse.down()
  for (let step = 1; step <= 5; step += 1) await page.mouse.move(moveFrom[0] + step * 12, moveFrom[1], { steps: 2 })
  await page.keyboard.press('KeyX')
  for (let step = 6; step <= 10; step += 1) await page.mouse.move(moveFrom[0] + step * 12, moveFrom[1], { steps: 2 })
  await page.mouse.up()
  await until(async () => (await details()).undo === 'Move', `the drag is a Move step (${JSON.stringify([(await details()).undo, (await details()).tool, await selectedNames(), (await named('Cube'))?.location])})`)
  await settle()
  assert.ok(await named('Cube'), 'X during the drag deleted nothing')
  await page.keyboard.press(`${mod}+KeyZ`)
  await until(async () => (await named('Cube'))?.location[0] === before[0], 'one undo takes the move back')
  assert.ok(await named('Cube'), 'the cube is still there after undo')
  passed('keys during a drag run no shortcut')

  // B8: a click with the move tool selects; a click on nothing clears.
  await page.mouse.click(empty[0], empty[1])
  await until(async () => (await selectedNames()).length === 0, 'the move tool: a click on nothing clears the selection')
  const light = await project((await named('Light')).location)
  await page.mouse.click(cube[0], cube[1])
  await until(async () => (await selectedNames()).join() === 'Cube', 'the move tool: a click selects the cube')
  passed('transform tools select by clicking')

  // B3: the tools' overlays show over the picture.
  const ink = () =>
    frame.evaluate(() => {
      const gpu = window.sceneGl?.active === true
      const canvas = document.querySelector(gpu ? '[data-testid="scene-tool-overlay"]' : '[data-testid="scene-viewport"] canvas.kui-canvas-surface')
      const context = canvas?.getContext('2d')
      if (!canvas || !context) return { gpu, count: -1 }
      const data = context.getImageData(0, 0, canvas.width, canvas.height).data
      let count = 0
      for (let index = 3; index < data.length; index += 4) if (data[index] > 0) count += 1
      return { gpu, count, total: data.length / 4 }
    })
  await page.keyboard.press('KeyW')
  await until(async () => (await details()).tool.id === 'select', 'W picks the select tool')
  await settle()
  const quiet = await ink()
  await page.mouse.move(box.x + 60, box.y + 60)
  await page.mouse.down()
  await page.mouse.move(box.x + 200, box.y + 160, { steps: 6 })
  await page.waitForTimeout(300)
  const boxed = await ink()
  if (boxed.gpu) {
    assert.equal(quiet.count, 0, 'the overlay layer is empty with no gesture')
    assert.ok(boxed.count > 200, `the box shows over the GPU picture (${boxed.count} pixels)`)
  } else {
    assert.ok(boxed.count > 0, 'the software picture shows the box')
  }
  await page.keyboard.press('Escape')
  await page.mouse.up()
  await page.mouse.click(cube[0], cube[1])
  await until(async () => (await selectedNames()).join() === 'Cube', 'the cube is selected again')
  await page.keyboard.press('KeyR')
  await until(async () => (await details()).tool.id === 'rotate', 'R picks the rotate tool')
  await settle()
  await page.mouse.move(cube[0] + 80, cube[1])
  await page.mouse.down()
  await page.mouse.move(cube[0] + 60, cube[1] - 70, { steps: 6 })
  await page.waitForTimeout(400)
  const rotating = await ink()
  const readout = await frame.evaluate(() => window.sceneToolOverlay ?? null)
  if (rotating.gpu) assert.ok(rotating.count > 100 && readout?.drawn, `the rotate readout and its line show over the GPU picture (${rotating.count} pixels)`)
  await page.keyboard.press('Escape')
  // The button is let go once the editor shows the rotation taken back (a slow machine may still
  // be answering the drag's last moves when Escape comes).
  const liveZ = () => frame.evaluate(() => window.kitScene.state().objects.find((item) => item.name === 'Cube').rotation[2])
  await until(async () => Math.abs((await liveZ()) - 33.33) < 1e-3, 'Escape takes the rotation back')
  await page.mouse.up()
  await settle()
  assert.deepEqual((await named('Cube')).rotation.map((value) => Math.round(value * 100) / 100), [0, 0, 33.33], 'Escape cancelled the rotation')
  passed(`tool overlays show (${boxed.gpu ? 'GPU layer' : 'software picture'})`)

  // B10: Shift+box adds, Cmd/Ctrl+box takes away.
  await page.keyboard.press('KeyW')
  await until(async () => (await details()).tool.id === 'select', 'W picks the select tool')
  const lightBox = [light[0] - 30, light[1] - 30, light[0] + 30, light[1] + 30]
  const boxDrag = async (rect, modifier) => {
    if (modifier) await page.keyboard.down(modifier)
    await page.mouse.move(rect[0], rect[1])
    await page.mouse.down()
    await page.mouse.move(rect[2], rect[3], { steps: 6 })
    await page.mouse.up()
    if (modifier) await page.keyboard.up(modifier)
    await settle()
  }
  await boxDrag(lightBox, 'Shift')
  await until(async () => (await selectedNames()).join() === 'Cube,Light', `Shift+box adds the light (${JSON.stringify(await selectedNames())})`)
  await boxDrag(lightBox, 'Shift')
  await until(async () => (await selectedNames()).join() === 'Cube,Light', 'Shift+box never takes away')
  await boxDrag(lightBox, mod)
  await until(async () => (await selectedNames()).join() === 'Cube', 'Cmd/Ctrl+box takes the light away')
  passed('box select adds and takes away')

  // A13: Cmd/Ctrl+click picks the nearest origin.
  await page.mouse.click(light[0] + 25, light[1] + 25)
  await until(async () => (await selectedNames()).length === 0, 'a plain click beside the light picks nothing')
  // The modifier held on the keyboard: Playwright's click `modifiers` do not reach the pointer
  // events of a page in a frame.
  await page.keyboard.down(mod)
  await page.mouse.click(light[0] + 25, light[1] + 25)
  await page.keyboard.up(mod)
  await until(async () => (await selectedNames()).join() === 'Light', 'Cmd/Ctrl+click picks the light by its origin')
  passed('Cmd/Ctrl+click picks by the origin')

  // B14: a right click on an unselected object selects it before the menu opens.
  await page.mouse.click(cube[0], cube[1], { button: 'right' })
  await kit.getByTestId('scene-context-menu').waitFor()
  await until(async () => (await selectedNames()).join() === 'Cube', 'the right click selected the cube')
  await page.keyboard.press('Escape')
  await settle()
  passed('a right click selects what it is on')

  // A11: a command with nothing to do says so in the status bar, not as an error card.
  const errorCards = () => kit.locator('.kui-toast').filter({ hasText: 'not-allowed' }).count()
  await page.keyboard.press('Alt+KeyH')
  await kit.getByTestId('scene-status-note').waitFor({ timeout: 10000 })
  assert.match(await kit.getByTestId('scene-status-note').textContent(), /hidden/i)
  assert.equal(await errorCards(), 0, 'no error card for Show Hidden with nothing hidden')
  passed('nothing to do is a status note')

  // B9: in edit mode the number row picks the select mode; the keypad does not.
  await page.keyboard.press('Tab')
  await until(async () => (await details()).mode.mode === 'edit', 'Tab edits the cube')
  await page.keyboard.press('Digit3')
  await until(async () => (await details()).mode.select === 'face', '3 is face select')
  await page.keyboard.press('Numpad1')
  await settle()
  assert.equal((await details()).mode.select, 'face', 'keypad 1 leaves the select mode')
  await page.keyboard.press('Digit1')
  await until(async () => (await details()).mode.select === 'vertex', '1 is vertex select')
  passed('the keypad is not the select modes')

  // B11: the outliner in edit mode: another object ends edit mode first.
  await kit.locator('[data-testid="scene-outliner-item"][data-name="Light"] .scene-outliner-name').click()
  await until(async () => (await details()).mode.mode === 'object' && (await selectedNames()).join() === 'Light', 'a click on the light row leaves edit mode and selects the light')
  passed('the outliner leaves edit mode for another object')

  // B12: F2 renames the active object in its outliner row.
  await page.mouse.click(cube[0], cube[1])
  await until(async () => (await selectedNames()).join() === 'Cube', 'the cube is selected')
  await page.keyboard.press('F2')
  const rename = kit.getByTestId('scene-rename')
  await rename.waitFor({ timeout: 10000 })
  await rename.fill('Box')
  await rename.press('Enter')
  await until(async () => (await named('Box')) != null, 'F2 renamed the cube')
  assert.equal(await errorCards(), 0, 'F2 shows no error')
  await page.keyboard.press(`${mod}+KeyZ`)
  await until(async () => (await named('Cube')) != null, 'undo takes the name back')
  passed('F2 renames in place')

  assert.deepEqual(errors, [], 'no page errors')
  console.log(`PASS the viewport's basics (${which})`)
} finally {
  await browser.close()
  await server.close()
}
