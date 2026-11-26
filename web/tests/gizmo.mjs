// The 3D cursor, the transform tools' gizmos, the pivot point, the transform orientation and
// snapping, by hand (plan scene-usable-by-hand, phase 5): node tests/gizmo.mjs [chromium|webkit]
// Real mouse and keyboard events on the development host, read back through the engine's inspect
// and view state: Shift+right click puts the cursor on the surface under the pointer (or on the
// plane through it facing the view) and opens no menu; an object added from the menus lands at the
// cursor; the Shift+S menu and Shift+C; the Move, Rotate and Scale gizmos (a handle lights up
// under the pointer, an arrow's drag changes one axis, a square's two, a ring turns about its
// axis, a box-ended axis scales along it, the right button cancels); the pivot point at the 3D
// cursor (`.`); the view orientation (`,`); Shift+Tab and vertex, edge and face snapping.
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
const until = async (check, message, timeout = 15000) => {
  const end = Date.now() + timeout
  while (Date.now() < end) {
    if (await check().catch(() => false)) return
    await page.waitForTimeout(50)
  }
  assert.fail(typeof message === 'function' ? await message() : message)
}
const passed = (name) => console.log(`PASS ${name} (${which})`)
const near = (a, b, tolerance = 1e-3) => Math.abs(a - b) <= tolerance
const nearAll = (a, b, tolerance = 1e-3) => a.length === b.length && a.every((value, index) => near(value, b[index], tolerance))

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
  console.log(`viewport: ${gl?.active ? 'GPU' : `software (${gl?.reason ?? 'no GPU probe'})`}`)
  const mac = await page.evaluate(() => /Mac|iPhone|iPad/i.test(navigator.platform))
  const mod = mac ? 'Meta' : 'Control'
  const dpr = await frame.evaluate(() => window.devicePixelRatio || 1)
  const run = (steps) => frame.evaluate((steps) => window.kitScene.run(steps), steps)
  const details = async () => (await page.evaluate(async () => window.devHost.connection().ai.inspect())).details
  // The person's own editor state (the agent's inspect keeps its own view of the editor).
  const state = () => frame.evaluate(() => window.kitScene.state())
  const transform = async () => (await state()).transform
  const named = async (name) => (await state()).objects.find((item) => item.name === name)
  const undo = async () => (await details()).undo
  const modal = () => frame.evaluate(() => window.sceneModal?.answer ?? { active: false })
  const active = async () => (await modal()).active === true
  const gizmo = () => frame.evaluate(() => window.sceneGizmo)
  const viewState = async () => JSON.parse(await frame.evaluate(async () => window.kitScene.engine().execute('view.state', {}))).state
  const back = async (label) => {
    await page.keyboard.press(`${mod}+KeyZ`)
    await until(async () => (await undo()) !== label, `undo takes the ${label} back`)
  }
  /** A document pixel of the viewport on the page. */
  const onPage = (point) => [box.x + point[0] / dpr, box.y + point[1] / dpr]
  /** Where a world point shows on the page, by the engine's own projection. */
  const project = async (point) => {
    const shown = await state()
    const m = shown.viewMatrix
    const [x, y, z] = point
    const clip = [0, 1, 2, 3].map((row) => m[row] * x + m[4 + row] * y + m[8 + row] * z + m[12 + row])
    return onPage([((clip[0] / clip[3]) * 0.5 + 0.5) * shown.viewWidth, (0.5 - (clip[1] / clip[3]) * 0.5) * shown.viewHeight])
  }
  const shot = async (name) => {
    if (shots) await page.screenshot({ path: `${shots}/${name}.png` })
  }

  // N4: the cursor starts at the world origin, with nothing run when the document opens.
  assert.deepEqual((await transform()).cursor, [0, 0, 0], 'the cursor starts at the origin')
  assert.deepEqual((await viewState()).cursor, [0, 0, 0], 'the view state keeps the cursor')
  await until(async () => (await gizmo())?.cursor != null, 'the cursor is drawn')
  const base = await undo()

  // Shift+right click on the cube: the cursor goes onto its surface; no menu, no undo step.
  const cube = await project([0, 0, 0])
  await page.mouse.move(cube[0], cube[1])
  await page.keyboard.down('Shift')
  await page.mouse.click(cube[0], cube[1], { button: 'right' })
  await page.keyboard.up('Shift')
  await until(async () => Math.max(...(await transform()).cursor.map(Math.abs)) > 0.5, async () => `the cursor goes onto the cube (${(await transform()).cursor})`)
  const onCube = (await transform()).cursor
  assert.ok(near(Math.max(...onCube.map(Math.abs)), 1, 1e-3), `the cursor is on the cube's surface (${onCube})`)
  await page.waitForTimeout(200)
  assert.equal(await kit.getByTestId('scene-context-menu').count(), 0, 'Shift+right click opens no context menu')
  assert.equal(await undo(), base, 'placing the cursor is no undo step')
  assert.deepEqual((await viewState()).cursor.map((value) => Math.round(value * 1e4) / 1e4), onCube.map((value) => Math.round(value * 1e4) / 1e4), 'the view state follows the cursor')
  await shot('01-cursor-on-cube')
  // Over nothing: onto the plane through the cursor facing the view.
  const sky = [box.x + 60, box.y + 60]
  await page.keyboard.down('Shift')
  await page.mouse.click(sky[0], sky[1], { button: 'right' })
  await page.keyboard.up('Shift')
  await until(async () => !nearAll((await transform()).cursor, onCube), 'Shift+right click over nothing moves the cursor too')
  const inAir = await project((await transform()).cursor)
  assert.ok(Math.hypot(inAir[0] - sky[0], inAir[1] - sky[1]) < 2, `the cursor is under the pointer (${inAir} vs ${sky})`)
  passed('Shift+right click places the 3D cursor')

  // An object added from the menus lands at the cursor, selected and active.
  await run([{ command: 'cursor.set', params: { location: [3, -2, 0.5] } }])
  await until(async () => nearAll((await transform()).cursor, [3, -2, 0.5]), 'cursor.set puts the cursor')
  await kit.getByTestId('scene-menu-Add').click()
  await kit.locator('[role="menuitem"][data-kit-ui-id="menu:Add/Mesh"]').hover()
  await kit.getByTestId('scene-cmd-object.addCylinder').click()
  await until(async () => (await named('Cylinder')) != null, 'Add ▸ Mesh ▸ Cylinder adds a cylinder')
  const cylinder = await named('Cylinder')
  assert.ok(nearAll(cylinder.location, [3, -2, 0.5]), `the cylinder is at the cursor (${cylinder.location})`)
  const shown = await state()
  assert.deepEqual([shown.selected, shown.active], [[cylinder.id], cylinder.id], 'the new object is the selection and active')
  await shot('02-added-at-cursor')
  await back('Add Cylinder')
  await until(async () => (await named('Cylinder')) == null, 'undo takes the cylinder away')
  passed('new objects land at the 3D cursor')

  // Shift+S: the cursor to the selection, the selection to the cursor, the cursor to the origin.
  await run([{ command: 'select.objects', params: { objects: ['Cube'] } }, { command: 'object.transform', params: { objects: ['Cube'], location: [1, 2, 0] } }])
  await until(async () => nearAll((await named('Cube')).location, [1, 2, 0]), 'the cube is placed')
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
  await page.keyboard.press('Shift+KeyS')
  await kit.getByTestId('scene-snap-menu').waitFor()
  await kit.getByTestId('scene-popup-cursor-selected').click()
  await until(async () => nearAll((await transform()).cursor, [1, 2, 0]), 'Cursor to Selected')
  await run([{ command: 'cursor.set', params: { location: [-1, 0, 2] } }])
  await page.keyboard.press('Shift+KeyS')
  await kit.getByTestId('scene-snap-menu').waitFor()
  await kit.getByTestId('scene-popup-selection-cursor').click()
  await until(async () => nearAll((await named('Cube')).location, [-1, 0, 2]), 'Selection to Cursor moves the cube')
  assert.equal(await undo(), 'Set Transform', 'Selection to Cursor is a Set Transform step')
  await page.keyboard.press(`${mod}+KeyZ`)
  await until(async () => nearAll((await named('Cube')).location, [1, 2, 0]), 'one undo puts the cube back')
  await page.keyboard.press('Shift+KeyS')
  await kit.getByTestId('scene-snap-menu').waitFor()
  await kit.getByTestId('scene-popup-cursor-origin').click()
  await until(async () => nearAll((await transform()).cursor, [0, 0, 0]), 'Cursor to World Origin')
  // Shift+C: the cursor at the origin and everything framed.
  await run([{ command: 'cursor.set', params: { location: [5, 5, 5] } }, { command: 'view.set', params: { distance: 60 } }])
  await until(async () => near((await details()).view.distance, 60, 0.01), 'the view is far away')
  await page.keyboard.press('Shift+KeyC')
  await until(async () => nearAll((await transform()).cursor, [0, 0, 0]) && (await details()).view.distance < 59, async () => `Shift+C puts the cursor back and frames all (${(await transform()).cursor}, ${(await details()).view.distance})`)
  await run([{ command: 'object.transform', params: { objects: ['Cube'], location: [0, 0, 0] } }])
  // The change of view Shift+C made is over before the gizmo is measured.
  let lastView = ''
  await until(async () => {
    const now = JSON.stringify((await details()).view)
    const same = now === lastView
    lastView = now
    await page.waitForTimeout(250)
    return same
  }, 'the view settles')
  passed('the Shift+S menu and Shift+C')

  // N3: the Move gizmo. A handle lights up under the pointer.
  await run([{ command: 'select.objects', params: { objects: ['Cube'] } }])
  await kit.getByTestId('scene-tool-move').click()
  await until(async () => (await gizmo())?.kind === 'move' && (await gizmo()).handles['move-x'] != null, 'the move tool shows its gizmo')
  const layerPixel = (point) =>
    frame.evaluate(([x, y]) => {
      const layer = document.querySelector('[data-testid="scene-transform-layer"]')
      const pixel = layer.getContext('2d').getImageData(Math.round(x), Math.round(y), 1, 1).data
      return [...pixel]
    }, point)
  const handle = async (name) => (await gizmo()).handles[name]
  // A point on the X arrow's shaft, a little short of its end.
  const shaft = async (name) => {
    const g = await gizmo()
    const centre = await project((await transform()).gizmo.centre)
    const end = onPage(g.handles[name])
    return [centre[0] + (end[0] - centre[0]) * 0.75, centre[1] + (end[1] - centre[1]) * 0.75]
  }
  const xShaft = await shaft('move-x')
  const doc = (point) => [(point[0] - box.x) * dpr, (point[1] - box.y) * dpr]
  await page.mouse.move(box.x + 20, box.y + box.height - 20)
  await until(async () => (await gizmo()).hovered == null, 'nothing is hovered')
  const dim = await layerPixel(doc(xShaft))
  await page.mouse.move(xShaft[0], xShaft[1])
  await until(async () => (await gizmo()).hovered === 'move-x', async () => `the X arrow is hovered (${(await gizmo()).hovered})`)
  const lit = await layerPixel(doc(xShaft))
  assert.ok(lit[1] > dim[1] + 60 && lit[3] > 0, `the hovered arrow is bright (${dim} → ${lit})`)
  await shot('03-move-gizmo-hover')
  passed('a gizmo handle lights up under the pointer')

  /** Drags from `from` by `by` pixels with the left button, waiting for the transform each way. */
  const drag = async (from, by) => {
    await page.mouse.move(from[0], from[1])
    await page.mouse.down()
    await until(active, 'the handle starts a transform')
    await page.mouse.move(from[0] + by[0], from[1] + by[1], { steps: 10 })
    await page.waitForTimeout(150)
    await page.mouse.up()
    await until(async () => !(await active()), 'the release ends it')
  }
  // The X arrow changes only x.
  await drag(xShaft, [70, 0])
  await until(async () => (await undo()) === 'Move', 'the drag is one Move step')
  let moved = (await named('Cube')).location
  assert.ok(Math.abs(moved[0]) > 0.2 && moved[1] === 0 && moved[2] === 0, `the X arrow moves along X only (${moved})`)
  await back('Move')
  // A square moves in its plane: the Z square keeps z.
  const zSquare = onPage(await handle('move-plane-z'))
  await drag(zSquare, [40, 30])
  await until(async () => (await undo()) === 'Move', 'the square drag is one Move step')
  moved = (await named('Cube')).location
  assert.ok(moved[2] === 0 && Math.abs(moved[0]) + Math.abs(moved[1]) > 0.2, `the XY square keeps z (${moved})`)
  await back('Move')
  // The right button during a handle's drag cancels it: no step.
  const stepBefore = await undo()
  const xAgain = await shaft('move-x')
  await page.mouse.move(xAgain[0], xAgain[1])
  await page.mouse.down()
  await until(active, 'the handle starts a transform')
  await page.mouse.move(xAgain[0] + 60, xAgain[1], { steps: 6 })
  await page.mouse.down({ button: 'right' })
  await page.mouse.up({ button: 'right' })
  await page.mouse.up()
  await until(async () => !(await active()), 'the right button ends it')
  assert.deepEqual((await named('Cube')).location, [0, 0, 0], 'the right button puts the cube back')
  assert.equal(await undo(), stepBefore, 'no undo step')
  // A press on the gizmo's middle is the tool's own: a free move, as without a gizmo.
  const middle = await project([0, 0, 0])
  await page.mouse.move(middle[0], middle[1])
  await page.mouse.down()
  await page.mouse.move(middle[0] + 50, middle[1] + 20, { steps: 8 })
  await page.waitForTimeout(150)
  await page.mouse.up()
  await until(async () => (await undo()) === 'Move', 'the middle drags freely')
  moved = (await named('Cube')).location
  assert.ok(moved.filter((value) => value !== 0).length >= 2, `the middle moves in the view's plane (${moved})`)
  await back('Move')
  passed('the Move gizmo: an arrow keeps to its axis, a square to its plane, the right button cancels')

  // The Rotate gizmo: the Z ring turns about Z.
  await kit.getByTestId('scene-tool-rotate').click()
  await until(async () => (await gizmo())?.kind === 'rotate' && (await gizmo()).handles['rotate-z'] != null, 'the rotate tool shows its gizmo')
  const ring = onPage(await handle('rotate-z'))
  const centre = await project([0, 0, 0])
  // Across the ring's radius: the pointer turns about the middle.
  const [rx, ry] = [ring[0] - centre[0], ring[1] - centre[1]]
  await drag(ring, [-ry * 0.6, rx * 0.6])
  await until(async () => (await undo()) === 'Rotate', 'the ring drag is one Rotate step')
  const turned = (await named('Cube')).rotation
  assert.ok(turned[0] === 0 && turned[1] === 0 && Math.abs(turned[2]) > 5, `the Z ring turns about Z only (${turned})`)
  await back('Rotate')
  await shot('04-rotate-gizmo')
  // The Scale gizmo: the X axis scales along X.
  await kit.getByTestId('scene-tool-scale').click()
  await until(async () => (await gizmo())?.kind === 'scale' && (await gizmo()).handles['scale-x'] != null, 'the scale tool shows its gizmo')
  const scaleShaft = await shaft('scale-x')
  await drag(scaleShaft, [(scaleShaft[0] - centre[0]) * 0.8, (scaleShaft[1] - centre[1]) * 0.8])
  await until(async () => (await undo()) === 'Scale', 'the axis drag is one Scale step')
  const scaled = (await named('Cube')).scale
  assert.ok(Math.abs(scaled[0] - 1) > 0.1 && scaled[1] === 1 && scaled[2] === 1, `the X axis scales along X only (${scaled})`)
  await back('Scale')
  passed('the Rotate and Scale gizmos keep to their axis')
  await page.keyboard.press('KeyW')
  await until(async () => (await gizmo())?.kind == null, 'the select tool shows no gizmo')

  // The pivot point at the 3D cursor (`.`): R Z 90 turns the cube about the cursor.
  await run([{ command: 'cursor.set', params: { location: [2, 0, 0] } }])
  await page.mouse.move(box.x + box.width / 2 + 80, box.y + box.height / 2 + 60)
  await page.keyboard.press('Period')
  await kit.getByTestId('scene-pivot-menu').waitFor()
  await kit.getByTestId('scene-popup-pivot-cursor').click()
  await until(async () => (await transform()).pivot === 'cursor', 'the pivot point is the 3D cursor')
  assert.equal(await kit.getByTestId('scene-pivot').inputValue(), 'cursor', 'the strip shows it')
  await page.mouse.move(box.x + box.width / 2 + 80, box.y + box.height / 2 + 60)
  await page.keyboard.press('KeyR')
  await until(active, 'R starts a rotation')
  for (const key of ['KeyZ', 'Digit9', 'Digit0', 'Enter']) await page.keyboard.press(key)
  await until(async () => (await undo()) === 'Rotate', 'R Z 90 is one Rotate step')
  const turnedAbout = await named('Cube')
  assert.ok(nearAll(turnedAbout.location, [2, -2, 0]) && near(turnedAbout.rotation[2], 90), `R Z 90 about the cursor (${turnedAbout.location}, ${turnedAbout.rotation})`)
  await back('Rotate')
  await run([{ command: 'transform.set', params: { pivot: 'median' } }, { command: 'cursor.reset', params: {} }])
  passed('the pivot point at the 3D cursor')

  // The orientation (`,`): View, and an axis key keeps to the view's X first.
  await page.keyboard.press('Comma')
  await kit.getByTestId('scene-orientation-menu').waitFor()
  await kit.getByTestId('scene-popup-orientation-view').click()
  await until(async () => (await transform()).orientation === 'view', 'the orientation is View')
  await page.keyboard.press('KeyG')
  await until(active, 'G starts a move')
  await page.keyboard.press('KeyX')
  await until(async () => (await modal()).constraint?.orientation === 'view', async () => `X keeps to the view's X (${JSON.stringify((await modal()).constraint)})`)
  await page.keyboard.press('KeyX')
  await until(async () => (await modal()).constraint?.orientation === 'global', 'X again: the global X')
  await page.keyboard.press('Escape')
  await until(async () => !(await active()), 'Escape ends it')
  await run([{ command: 'transform.set', params: { orientation: 'global' } }])
  passed('the view orientation and the axis keys')

  // Snapping: Shift+Tab turns it on; to a vertex, the cube's corner lands on the other cube's.
  await run([{ command: 'object.addCube', params: { name: 'Other', location: [4, 0, 0] } }, { command: 'select.objects', params: { objects: ['Cube'] } }])
  await until(async () => (await named('Other')) != null && (await state()).selected.length === 1, 'a second cube')
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
  await page.keyboard.press('Shift+Tab')
  await until(async () => (await transform()).snap === true, 'Shift+Tab turns snapping on')
  await kit.getByTestId('scene-snap-target').selectOption('vertex')
  await until(async () => (await transform()).snapTarget === 'vertex', 'snap to vertices')
  const corner = await project([3, 1, 1])
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
  await page.keyboard.press('KeyG')
  await until(active, 'G starts a move')
  await page.mouse.move(corner[0] + 3, corner[1] + 2, { steps: 12 })
  await until(async () => (await modal()).snap?.point != null, 'the move snaps to a vertex')
  assert.ok(nearAll((await modal()).snap.point, [3, 1, 1]), `it snaps to the other cube's corner (${(await modal()).snap.point})`)
  await until(async () => (await gizmo()).snapped != null, 'the snapped point is marked')
  await shot('05-vertex-snap')
  await page.mouse.click(corner[0] + 3, corner[1] + 2)
  await until(async () => (await undo()) === 'Move', 'one Move step')
  const snappedTo = (await named('Cube')).location
  const corners = [0, 1, 2, 3, 4, 5, 6, 7].map((bit) => [snappedTo[0] + (bit & 1 ? 1 : -1), snappedTo[1] + (bit & 2 ? 1 : -1), snappedTo[2] + (bit & 4 ? 1 : -1)])
  assert.ok(corners.some((corner) => nearAll(corner, [3, 1, 1])), `a corner of the cube is on the other's vertex (${snappedTo})`)
  await back('Move')
  // Ctrl inverts it: snapping off while held.
  await page.keyboard.press('KeyG')
  await until(active, 'G starts a move')
  await page.keyboard.down(mod === 'Meta' ? 'Meta' : 'Control')
  await page.mouse.move(corner[0] + 3, corner[1] + 3, { steps: 6 })
  await until(async () => (await modal()).snap?.on === false, 'Ctrl turns snapping off while held')
  assert.equal((await modal()).snap.point, null, 'nothing snapped')
  await page.keyboard.up(mod === 'Meta' ? 'Meta' : 'Control')
  await page.keyboard.press('Escape')
  await until(async () => !(await active()), 'Escape ends it')
  // To a face: the cube's lowest corner lands on the other cube's top.
  await kit.getByTestId('scene-snap-target').selectOption('face')
  await until(async () => (await transform()).snapTarget === 'face', 'snap to faces')
  const top = await project([4, 0, 1])
  await page.keyboard.press('KeyG')
  await until(active, 'G starts a move')
  await page.mouse.move(top[0], top[1], { steps: 12 })
  await until(async () => (await modal()).snap?.point != null, 'the move snaps to a face')
  assert.ok(near((await modal()).snap.point[2], 1, 1e-3), `onto the top face (${(await modal()).snap.point})`)
  await page.mouse.click(top[0], top[1])
  await until(async () => (await undo()) === 'Move', 'one Move step')
  const onTop = (await named('Cube')).location
  assert.ok(near(onTop[2] + 1, 1, 1e-3) || near(onTop[2] - 1, 1, 1e-3), `a corner of the cube is on the top face (${onTop})`)
  await back('Move')
  // To an edge: onto the other cube's top front edge.
  await kit.getByTestId('scene-snap-target').selectOption('edge')
  await until(async () => (await transform()).snapTarget === 'edge', 'snap to edges')
  const edge = await project([4, -1, 1])
  await page.keyboard.press('KeyG')
  await until(active, 'G starts a move')
  await page.mouse.move(edge[0], edge[1], { steps: 12 })
  await until(async () => (await modal()).snap?.point != null, 'the move snaps to an edge')
  const onEdge = (await modal()).snap.point
  assert.ok(near(onEdge[2], 1, 0.02) && (near(onEdge[1], -1, 0.02) || near(onEdge[0], 3, 0.02) || near(onEdge[0], 5, 0.02)), `onto an edge of the other cube's top (${onEdge})`)
  await page.keyboard.press('Escape')
  await until(async () => !(await active()), 'Escape ends it')
  await page.keyboard.press('Shift+Tab')
  await until(async () => (await transform()).snap === false, 'Shift+Tab turns snapping off')
  passed('snapping to vertices, faces and edges; Ctrl inverts it')

  assert.deepEqual(errors, [], `no page errors (${errors.join('; ')})`)
  console.log(`gizmo: all passed (${which})`)
} finally {
  await browser.close()
  await server.close()
}
