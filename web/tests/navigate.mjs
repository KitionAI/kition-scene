// The viewport's navigation by hand (plan scene-usable-by-hand, phase 4): node tests/navigate.mjs [chromium|webkit]
// Real mouse, wheel and keyboard events on the development host, read back through the engine's
// view state: the keypad's views, steps, zoom and pan, automatic
// perspective, the smooth change of view, the navigation gizmo (a click on a ball, again from the
// other side, a drag; its zoom and pan buttons), the wheel told from a trackpad's two fingers,
// Ctrl+middle drag, the zoom's limits, the camera view's frame, the active camera put where the eye
// is, the View menu on `, the keypad in edit mode, and Ctrl/Cmd+Z right after a menu's row.
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
// What the pages said, printed when the document does not open (on a runner nothing else shows why).
const said = []
page.on('console', (message) => said.push(`${message.type()}: ${message.text()}`))
page.on('pageerror', (error) => said.push(`pageerror: ${error.message}\n${error.stack ?? ''}`))
const until = async (check, message, timeout = 15000) => {
  const end = Date.now() + timeout
  while (Date.now() < end) {
    if (await check().catch(() => false)) return
    await page.waitForTimeout(50)
  }
  assert.fail(typeof message === 'function' ? await message() : message)
}
const passed = (name) => console.log(`PASS ${name} (${which})`)
const near = (a, b, tolerance = 0.01) => Math.abs(a - b) <= tolerance
/** a − b in degrees, the short way round. */
const turned = (a, b) => ((((a - b) % 360) + 540) % 360) - 180

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
  const run = (steps) => frame.evaluate((steps) => window.kitScene.run(steps), steps)
  const state = () => frame.evaluate(() => window.kitScene.state())
  const details = async () => (await page.evaluate(async () => window.devHost.connection().ai.inspect())).details
  const view = async () => (await details()).view
  const undo = async () => (await details()).undo
  const centre = [box.x + box.width / 2, box.y + box.height / 2]
  /** Waits for the view to settle where `check` wants it (a change of view takes 200 ms). */
  const viewIs = (check, message) => until(async () => check(await view()), async () => `${message} (${JSON.stringify(await view())})`)
  const shot = async (name) => {
    if (shots) await page.screenshot({ path: `${shots}/${name}.png` })
  }
  await shot('01-opening')

  // Ctrl/Cmd+Z right after a menu's row: the frame's list of what can run follows the change a
  // moment later, and a Ctrl/Cmd shortcut it thought disabled was dropped.
  await run([{ command: 'select.objects', params: { objects: ['Cube'] } }, { command: 'mode.set', params: { mode: 'edit' } }, { command: 'mesh.selectMode', params: { mode: 'face' } }, { command: 'mesh.select', params: { faces: [5] } }])
  await until(async () => (await details()).mode.mode === 'edit', 'edit mode')
  const facesBefore = (await state()).objects.find((item) => item.name === 'Cube').mesh.faces
  await page.mouse.move(centre[0], centre[1])
  await page.keyboard.press('KeyX')
  await kit.getByTestId('scene-delete-menu').waitFor()
  await kit.getByTestId('scene-popup-mesh.deleteFaces').click()
  await until(async () => (await undo()) === 'Delete Faces', 'Faces deletes the face')
  await page.keyboard.press(`${mod}+KeyZ`)
  await until(async () => (await state()).objects.find((item) => item.name === 'Cube').mesh.faces === facesBefore, async () => `Ctrl/Cmd+Z right after the menu undoes the deletion (${await undo()})`)
  passed('Ctrl/Cmd+Z right after a menu row undoes it')

  // B9 / N6 in edit mode: keypad 1 is the front view; the number row's 1 is vertex select.
  await page.keyboard.press('Numpad1')
  await viewIs((v) => near(v.tilt, 90) && near(v.turn, 0) && !v.perspective, 'keypad 1 looks from the front, orthographic')
  assert.equal((await details()).mode.select, 'face', 'keypad 1 leaves the select mode')
  await page.keyboard.press('Digit1')
  await until(async () => (await details()).mode.select === 'vertex', 'the number row 1 is vertex select')
  await run([{ command: 'mode.set', params: { mode: 'object' } }])
  await until(async () => (await details()).mode.mode === 'object', 'object mode')
  passed('in edit mode keypad 1 is the front view, the number row 1 vertex select')

  // N6: the keypad's views; Ctrl (Cmd on a Mac) looks from the other side.
  const axisKeys = [
    ['Numpad1', 90, 0], [`${mod}+Numpad1`, 90, 180],
    ['Numpad3', 90, 90], [`${mod}+Numpad3`, 90, 270],
    ['Numpad7', 0, 0], [`${mod}+Numpad7`, 180, 0],
  ]
  for (const [key, tilt, turn] of axisKeys) {
    await page.keyboard.press(key)
    await viewIs((v) => near(v.tilt, tilt) && near(Math.abs(turned(v.turn, turn)), 0) && !v.perspective, `${key} looks along its axis`)
  }
  // 9 turns to the opposite view: bottom to top.
  await page.keyboard.press('Numpad9')
  await viewIs((v) => near(v.tilt, 0), 'keypad 9 turns the bottom view to the top')
  // A5: an orbit step away from an orthographic axis view is in perspective.
  await page.keyboard.press('Numpad2')
  await viewIs((v) => near(v.tilt, 15) && v.perspective, 'keypad 2 lowers the eye 15° and turns perspective')
  await page.keyboard.press('Numpad8')
  await viewIs((v) => near(v.tilt, 0), 'keypad 8 raises it again')
  await page.keyboard.press('Numpad2')
  await page.keyboard.press('Numpad2')
  await viewIs((v) => near(v.tilt, 30), 'keypad 2 twice')
  let before = await view()
  await page.keyboard.press('Numpad4')
  await viewIs((v) => near(turned(v.turn, before.turn), -15), 'keypad 4 turns 15°')
  await page.keyboard.press('Numpad6')
  await page.keyboard.press('Numpad6')
  await viewIs((v) => near(turned(v.turn, before.turn), 15), 'keypad 6 turns the other way')
  // 5 toggles the projection.
  before = await view()
  await page.keyboard.press('Numpad5')
  await viewIs((v) => v.perspective !== before.perspective, 'keypad 5 toggles perspective')
  await page.keyboard.press('Numpad5')
  await viewIs((v) => v.perspective === before.perspective, 'keypad 5 again')
  // + and − zoom by steps; Ctrl+4 / 6 / 8 / 2 pan.
  before = await view()
  await page.keyboard.press('NumpadAdd')
  await viewIs((v) => near(v.distance, before.distance / 1.2, 0.01), 'keypad + zooms in a step')
  await page.keyboard.press('NumpadSubtract')
  await viewIs((v) => near(v.distance, before.distance, 0.01), 'keypad − zooms out a step')
  await page.keyboard.press(`${mod}+Numpad6`)
  await viewIs((v) => v.target.some((value, index) => !near(value, before.target[index], 1e-3)) && near(v.distance, before.distance, 0.01), 'Ctrl+keypad 6 pans')
  passed('the keypad: views, opposite, steps, projection, zoom and pan')

  // The change of view is smooth: on the GPU viewport it draws frames between the two views, and
  // the engine gets the view it ends at.
  if (gpu) {
    const frames = await frame.evaluate(() => window.sceneGl.frames)
    await page.keyboard.press('Numpad1')
    await viewIs((v) => near(v.tilt, 90) && near(v.turn, 0), 'keypad 1')
    const drawn = (await frame.evaluate(() => window.sceneGl.frames)) - frames
    assert.ok(drawn >= 3, `the change of view drew frames on its way (${drawn})`)
    passed('a change of view is smooth')
  }

  // Alt+1 / 3 / 7 for keyboards without a keypad.
  await page.keyboard.press('Alt+Digit7')
  await viewIs((v) => near(v.tilt, 0) && !v.perspective, 'Alt+7 is the top view')
  await page.keyboard.press('Alt+Digit3')
  await viewIs((v) => near(v.tilt, 90) && near(v.turn, 90), 'Alt+3 is the right view')

  // A5: a middle drag from the right view orbits in perspective.
  await page.mouse.move(centre[0], centre[1])
  await page.mouse.down({ button: 'middle' })
  await page.mouse.move(centre[0] + 60, centre[1] + 20, { steps: 6 })
  await page.mouse.up({ button: 'middle' })
  await viewIs((v) => v.perspective && !near(v.turn, 90), 'the orbit from the right view is in perspective')
  passed('Alt+1 / 3 / 7, and automatic perspective')

  // `: the View menu at the pointer.
  await page.keyboard.press('Backquote')
  await kit.getByTestId('scene-view-menu').waitFor()
  await shot('02-view-menu')
  await kit.getByTestId('scene-popup-view.left').click()
  await viewIs((v) => near(v.tilt, 90) && near(v.turn, 270) && !v.perspective, 'the menu row looks from the left')
  passed('` opens the View menu')

  // N7: the navigation gizmo. A click on Z looks from the top; again, from the bottom.
  await kit.getByTestId('scene-nav-view.top').click()
  await viewIs((v) => near(v.tilt, 0) && !v.perspective, 'a click on Z is the top view')
  await kit.getByTestId('scene-nav-view.top').click()
  await viewIs((v) => near(v.tilt, 180), 'a second click is the bottom view')
  await kit.getByTestId('scene-nav-view.front').click()
  await viewIs((v) => near(v.tilt, 90) && near(v.turn, 0), 'a click on −Y is the front view')
  // A drag on it orbits (in perspective, off the axis).
  const gizmo = await kit.getByTestId('scene-gizmo').boundingBox()
  before = await view()
  await page.mouse.move(gizmo.x + gizmo.width / 2 + 25, gizmo.y + gizmo.height / 2 + 25)
  await page.mouse.down()
  await page.mouse.move(gizmo.x + gizmo.width / 2 - 15, gizmo.y + gizmo.height / 2 + 5, { steps: 6 })
  await page.mouse.up()
  await viewIs((v) => !near(v.turn, before.turn) && v.perspective, 'a drag on the gizmo orbits')
  // Its zoom button: drag down zooms in; its pan button moves the view.
  for (const [id, check, message] of [
    ['scene-nav-zoom', (v, b) => v.distance < b.distance * 0.9, 'dragging the zoom button down zooms in'],
    ['scene-nav-pan', (v, b) => v.target.some((value, index) => !near(value, b.target[index], 1e-3)), 'dragging the pan button pans'],
  ]) {
    const button = await kit.getByTestId(id).boundingBox()
    before = await view()
    await page.mouse.move(button.x + button.width / 2, button.y + button.height / 2)
    await page.mouse.down()
    await page.mouse.move(button.x + button.width / 2 - 30, button.y + button.height / 2 + 40, { steps: 6 })
    await page.mouse.up()
    await viewIs((v) => check(v, before), message)
  }
  await shot('03-gizmo')
  passed('the navigation gizmo: clicks, again from the other side, drags, zoom and pan')

  // N8: the wheel and a trackpad. A plain vertical wheel in whole pixels zooms; a sideways or
  // fractional scroll is a trackpad's two fingers and orbits; with Shift it pans; Ctrl zooms.
  await page.mouse.move(centre[0], centre[1])
  before = await view()
  await page.mouse.wheel(0, 240)
  await viewIs((v) => v.distance > before.distance * 1.1 && near(v.turn, before.turn) && near(v.tilt, before.tilt), 'the wheel zooms')
  before = await view()
  await page.mouse.wheel(40, 0)
  await viewIs((v) => !near(v.turn, before.turn) && near(v.distance, before.distance, 1e-3), 'a sideways scroll orbits')
  // A fractional vertical scroll orbits. Playwright's WebKit hands the page whole pixels: there the
  // same scroll is a wheel's, and zooms (what cannot be told apart stays the wheel's).
  await page.waitForTimeout(300)
  await frame.evaluate(() => {
    window.addEventListener('wheel', (event) => (window.lastWheel = [event.deltaX, event.deltaY]), { capture: true, once: true })
  })
  before = await view()
  await page.mouse.wheel(0, 20.5)
  await until(async () => (await frame.evaluate(() => window.lastWheel)) != null, 'the scroll reached the page')
  const fractional = !Number.isInteger((await frame.evaluate(() => window.lastWheel))[1])
  if (fractional) await viewIs((v) => !near(v.tilt, before.tilt) && near(v.distance, before.distance, 1e-3), 'a fractional scroll orbits')
  else await viewIs((v) => near(v.tilt, before.tilt) && !near(v.distance, before.distance, 1e-3), 'a scroll in whole pixels zooms')
  before = await view()
  await page.keyboard.down('Shift')
  await page.mouse.wheel(12.5, 18.5)
  await page.keyboard.up('Shift')
  await viewIs((v) => v.target.some((value, index) => !near(value, before.target[index], 1e-3)) && near(v.turn, before.turn) && near(v.tilt, before.tilt), 'Shift with two fingers pans')
  before = await view()
  await page.keyboard.down('Control')
  await page.mouse.wheel(0, -30.5)
  await page.keyboard.up('Control')
  await viewIs((v) => v.distance < before.distance && near(v.turn, before.turn), 'Ctrl with two fingers (a pinch) zooms')
  // Ctrl+middle drag zooms: up out, down in.
  before = await view()
  await page.keyboard.down('Control')
  await page.mouse.down({ button: 'middle' })
  await page.mouse.move(centre[0], centre[1] + 80, { steps: 6 })
  await page.mouse.up({ button: 'middle' })
  await page.keyboard.up('Control')
  await viewIs((v) => v.distance < before.distance * 0.9 && near(v.turn, before.turn), 'Ctrl+middle drag down zooms in')
  passed('the wheel zooms, two fingers orbit, Shift pans, Ctrl zooms, Ctrl+middle drag zooms')

  // A4: the zoom keeps within limits, so the picture never goes empty.
  for (let index = 0; index < 60; index += 1) await page.keyboard.press('NumpadAdd')
  await viewIs((v) => near(v.distance, 0.05, 1e-3), 'zooming in stops at 5 cm')
  await page.mouse.wheel(0, -400)
  await page.waitForTimeout(300)
  assert.ok((await view()).distance >= 0.05 - 1e-4, 'the wheel stops there too')
  for (let index = 0; index < 80; index += 1) await page.keyboard.press('NumpadSubtract')
  // 80 steps of 1.2 would be 100 km: the zoom stops while the scene is still some pixels across.
  let far = 0
  await until(async () => {
    const now = (await view()).distance
    const still = now === far && now > 100
    far = now
    if (!still) await page.waitForTimeout(250)
    return still
  }, () => `zooming out stops (${far})`, 30000)
  assert.ok(far <= 40 * 50, `zooming out stops while the scene shows (${far})`)
  await page.keyboard.press('Home')
  await viewIs((v) => v.distance < 100, 'Home frames everything')
  passed('the zoom stops at its limits, and Home frames everything')

  // A6: in the camera view the wheel and a pan move the camera's frame, not out of the camera.
  await page.keyboard.press('Numpad0')
  await viewIs((v) => v.camera != null, 'keypad 0 looks through the camera')
  await page.mouse.move(centre[0], centre[1])
  await page.mouse.wheel(0, -200)
  await viewIs((v) => v.camera != null && v.cameraZoom > 1.1, 'the wheel zooms the camera frame and stays in the camera view')
  await page.keyboard.down('Shift')
  await page.mouse.down({ button: 'middle' })
  await page.mouse.move(centre[0] + 80, centre[1], { steps: 4 })
  await page.mouse.up({ button: 'middle' })
  await page.keyboard.up('Shift')
  await viewIs((v) => v.camera != null && v.cameraOffset[0] > 0.05, 'Shift+middle drag moves the frame')
  await shot('04-camera-frame')
  await page.keyboard.press('Home')
  await viewIs((v) => v.camera != null && near(v.cameraZoom, 1) && near(v.cameraOffset[0], 0), 'Home fits the frame again')
  await page.keyboard.press('Numpad0')
  await viewIs((v) => v.camera == null, 'keypad 0 again leaves the camera view')
  passed('the camera view: the wheel and a pan move the frame, Home fits it')

  // N19: Ctrl+Alt+keypad 0 puts the active camera where the eye is, as one undo step, and looks
  // through it.
  await page.keyboard.press('Numpad1')
  await viewIs((v) => near(v.tilt, 90) && near(v.turn, 0), 'the front view')
  const eye = await view()
  const history = await undo()
  await page.keyboard.press(`${mod}+Alt+Numpad0`)
  await viewIs((v) => v.camera != null, 'the camera view')
  const camera = (await details()).objects.find((item) => item.name === 'Camera')
  assert.ok(near(camera.location[0], eye.target[0], 1e-3) && near(camera.location[1], eye.target[1] - eye.distance, 1e-2) && near(camera.location[2], eye.target[2], 1e-3), `the camera is at the eye (${camera.location})`)
  assert.ok(near(camera.rotation[0], 90, 1e-2) && near(camera.rotation[2], 0, 1e-2), `the camera looks the view's way (${camera.rotation})`)
  assert.notEqual(await undo(), history, 'one undo step')
  await page.keyboard.press(`${mod}+KeyZ`)
  await until(async () => (await undo()) === history, 'one undo takes it back')
  passed('Ctrl+Alt+keypad 0 aligns the active camera to the view')

  assert.deepEqual(errors, [], 'no page errors')
  console.log(`navigate: all passed (${which})`)
} finally {
  await browser.close()
  await server.close()
}
