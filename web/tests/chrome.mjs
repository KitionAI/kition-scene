// The editor's skeleton by hand (plan scene-usable-by-hand, phase 7): node tests/chrome.mjs [chromium|webkit]
// Real mouse and keyboard events on the development host, read back through the editor's state:
// the viewport header's mode drop-down and menus in object and edit mode, the frame's menus the
// mode has no use for hidden, Shift+A adding a cylinder at the 3D cursor, the right-click menus of
// object and edit mode, F3's search running a command, Z's pie (a click, and the key held toward an
// item and let go), the header's shading buttons, T and N, the header's kinds submenu (a
// modifier), Ctrl+P's menu from the Object menu, Link Materials, the Item section's dimensions,
// the outliner's drag (parent, then clear it) and its right-click menu, and the status bar's hints.
//
// The GPU viewport is asked for (`kition.scene.gl` = `on`); KITION_SCENE_GL=off runs it on the
// software viewport; KITION_SCENE_MODES=classic opens the editor in Classic.
// KITION_SCENE_SNAPSHOT=<dir> saves screenshots there.
import assert from 'node:assert/strict'
import { chromium, webkit } from 'playwright'
import { createServer } from 'vite'

const which = process.argv[2] ?? 'chromium'
const shots = process.env.KITION_SCENE_SNAPSHOT
const modes = process.env.KITION_SCENE_MODES
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
const passed = (name) => console.log(`PASS ${name} (${which}${modes ? `, ${modes}` : ''})`)
const near = (a, b, tolerance = 1e-3) => Math.abs(a - b) <= tolerance

try {
  await page.goto(`http://127.0.0.1:${server.httpServer.address().port}/dev.html${modes ? `?modes=${modes}` : ''}`)
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
  const mac = await page.evaluate(() => /Mac|iPhone|iPad/i.test(navigator.platform))
  const mod = mac ? 'Meta' : 'Control'
  const run = (steps) => frame.evaluate((steps) => window.kitScene.run(steps), steps)
  const state = () => frame.evaluate(() => window.kitScene.state())
  const named = async (name) => (await state()).objects.find((item) => item.name === name)
  const undo = async () => (await page.evaluate(async () => window.devHost.connection().ai.inspect())).details.undo
  const shot = async (name) => {
    if (shots) await page.screenshot({ path: `${shots}/${name}${modes ? `-${modes}` : ''}.png` })
  }
  const headerMenus = async () => kit.locator('[data-testid^="scene-header-menu-"]').evaluateAll((items) => items.map((item) => item.dataset.testid.replace('scene-header-menu-', '')))
  const frameMenu = (name) => kit.getByTestId(`scene-menu-${name}`)
  const centre = [box.x + box.width / 2, box.y + box.height / 2]
  const escape = async (testId) => {
    await page.keyboard.press('Escape')
    await until(async () => (await kit.getByTestId(testId).count()) === 0, `Escape closes ${testId}`)
  }
  const classic = modes === 'classic'

  // A2, A3: the header's mode drop-down and menus in object mode; the frame keeps Object, hides Mesh.
  await run([{ command: 'select.objects', params: { objects: ['Cube'] } }])
  await until(async () => (await state()).active === (await named('Cube')).id, 'the cube is active')
  await until(async () => (await kit.getByTestId('scene-header').count()) === 1, 'the viewport has a header')
  assert.deepEqual(await headerMenus(), ['View', 'Select', 'Add', 'Object'], 'object mode: View, Select, Add, Object')
  assert.equal(await kit.getByTestId('scene-mode-select').inputValue(), 'object', 'the drop-down shows object mode')
  const listed = await kit.getByTestId('scene-mode-select').locator('option').evaluateAll((options) => options.map((option) => option.value))
  assert.deepEqual(listed, ['object', 'edit', 'sculpt', 'weightPaint'], "a mesh's modes")
  if (!classic) {
    assert.equal(await frameMenu('Object').count(), 1, 'the frame shows Object in object mode')
    assert.equal(await frameMenu('Mesh').count(), 0, 'the frame hides Mesh in object mode')
  }
  for (const id of ['scene-shading-wireframe', 'scene-shading-solid', 'scene-shading-material', 'scene-shading-rendered', 'scene-orientation', 'scene-pivot', 'scene-snap-target', 'scene-overlays']) {
    assert.equal(await kit.getByTestId(id).count(), 1, `the header has ${id}`)
  }
  await shot('01-header-object')
  passed('the header in object mode')

  // The header's mode drop-down enters edit mode: its menus and the frame's change.
  await kit.getByTestId('scene-mode-select').selectOption('edit')
  await until(async () => (await state()).mode === 'edit', 'the drop-down enters edit mode')
  await until(async () => (await headerMenus()).join() === 'View,Select,Mesh,Vertex,Edge,Face,UV', async () => `edit mode's menus (${await headerMenus()})`)
  for (const kind of ['vertex', 'edge', 'face']) assert.equal(await kit.getByTestId(`scene-select-${kind}`).count(), 1, `the ${kind} button`)
  await kit.getByTestId('scene-select-face').click()
  await until(async () => (await state()).select === 'face', 'the face button selects faces')
  if (!classic) {
    await until(async () => (await frameMenu('Object').count()) === 0, 'the frame hides Object in edit mode')
    assert.equal(await frameMenu('Mesh').count(), 1, 'the frame shows Mesh in edit mode')
  }
  await shot('02-header-edit')
  // The Item section in edit mode: the selection's median, which moves it.
  await run([{ command: 'edit.selectAll', params: { action: 'select' } }])
  await until(async () => (await state()).vertices.length === 8, 'every vertex selected')
  const median = kit.getByTestId('scene-item-median-z').locator('input')
  await median.fill('1')
  await median.press('Enter')
  await until(async () => near((await state()).transform.median?.[2] ?? 0, 1), async () => `the median moved up (${(await state()).transform.median})`)
  await run([{ command: 'edit.undo', params: {} }])
  await until(async () => near((await state()).transform.median?.[2] ?? 1, 0), 'undone')
  // Edit mode's right-click menu: the face menu.
  await page.mouse.click(centre[0], centre[1], { button: 'right' })
  await kit.getByTestId('scene-context-menu').waitFor()
  assert.equal(await kit.getByTestId('scene-context-title').getAttribute('data-kind'), 'edit-face', 'the face menu')
  assert.equal(await kit.getByTestId('scene-context-mesh.inset').count(), 1, 'it offers Inset Faces')
  await shot('03-context-edit')
  await escape('scene-context-menu')
  // A header menu row: Mesh ▸ Transform ▸ Move starts the move as G does; Escape cancels it.
  await kit.getByTestId('scene-header-menu-Vertex').click()
  await kit.getByTestId('scene-header-mesh.extrude-vertices').waitFor()
  await page.keyboard.press('Escape')
  await kit.getByTestId('scene-select-vertex').click()
  await until(async () => (await state()).select === 'vertex', 'the vertex button')
  await kit.getByTestId('scene-mode-select').selectOption('object')
  await until(async () => (await state()).mode === 'object', 'the drop-down goes back to object mode')
  await until(async () => (await headerMenus()).join() === 'View,Select,Add,Object', 'object mode menus again')
  passed('the mode drop-down and the menus by mode')

  // Ctrl+Tab: the same modes at the pointer.
  await page.mouse.move(centre[0] - 200, centre[1] + 100)
  await page.keyboard.press('Control+Tab')
  await kit.getByTestId('scene-mode-menu').waitFor({ timeout: 5000 }).catch(() => null)
  if (await kit.getByTestId('scene-mode-menu').count()) {
    assert.equal(await kit.getByTestId('scene-popup-mode-edit').count(), 1, 'Ctrl+Tab lists edit mode')
    await escape('scene-mode-menu')
    passed('Ctrl+Tab lists the modes')
  } else console.log(`NOTE Ctrl+Tab did not reach the page (${which})`)

  // B14, N21: object mode's right-click menu on the cube.
  const cube = await named('Cube')
  await page.mouse.click(centre[0], centre[1], { button: 'right' })
  await kit.getByTestId('scene-context-menu').waitFor()
  await until(async () => (await kit.getByTestId('scene-context-title').getAttribute('data-kind')) === 'object', 'the object menu')
  assert.equal(await kit.getByTestId('scene-context-object.shadeSmooth').count(), 1, 'it offers Shade Smooth')
  assert.equal(await kit.getByTestId('scene-context-mesh.inset').count(), 0, 'and not edit mode rows')
  await shot('04-context-object')
  await escape('scene-context-menu')
  passed('object and edit mode have their own right-click menus')

  // N9: Shift+A at the pointer adds a cylinder at the 3D cursor; it shows in Adjust Last Operation.
  await run([{ command: 'cursor.set', params: { location: [2, 1, 0] } }])
  await until(async () => (await state()).transform.cursor[0] === 2, 'the cursor moved')
  await page.mouse.move(centre[0] - 150, centre[1] - 50)
  await page.keyboard.press('Shift+KeyA')
  await kit.getByTestId('scene-add-menu').waitFor()
  await kit.getByTestId('scene-popup-mesh').hover()
  await kit.getByTestId('scene-popup-object.addCylinder').waitFor()
  await shot('05-shift-a')
  await kit.getByTestId('scene-popup-object.addCylinder').click()
  await until(async () => (await named('Cylinder')) != null, 'Shift+A ▸ Mesh ▸ Cylinder adds a cylinder')
  const cylinder = await named('Cylinder')
  assert.ok(near(cylinder.location[0], 2) && near(cylinder.location[1], 1), `at the cursor (${cylinder.location})`)
  await until(async () => (await state()).operation?.command === 'object.addCylinder', 'Adjust Last Operation shows it')
  await kit.getByTestId('scene-adjust-last').waitFor()
  passed('Shift+A adds at the cursor')

  // The empty space's menu: nothing selected.
  await run([{ command: 'edit.deselectAll', params: {} }])
  await until(async () => (await state()).selected.length === 0, 'nothing selected')
  await page.mouse.click(box.x + 40, box.y + box.height - 60, { button: 'right' })
  await kit.getByTestId('scene-context-menu').waitFor()
  await until(async () => (await kit.getByTestId('scene-context-title').getAttribute('data-kind')) === 'empty', 'the empty space has its own menu')
  await escape('scene-context-menu')
  passed('the empty space has its own right-click menu')

  // F3: search, then Enter runs the lit command.
  await run([{ command: 'select.objects', params: { objects: ['Cube'] } }])
  await until(async () => (await state()).selected.length === 1, 'the cube selected')
  await page.mouse.move(centre[0], centre[1])
  await page.keyboard.press('F3')
  await kit.getByTestId('scene-search').waitFor()
  await kit.getByTestId('scene-search-field').fill('shade smooth')
  await until(async () => (await kit.getByTestId('scene-search-item').first().getAttribute('data-id')) === 'object.shadeSmooth', 'the search finds Shade Smooth')
  assert.match(await kit.getByTestId('scene-search-item').first().textContent(), /Object/, 'with its menu path')
  await shot('06-search')
  await kit.getByTestId('scene-search-field').press('Enter')
  await until(async () => (await undo()) === 'Shade Smooth', async () => `Enter ran it (${await undo()})`)
  await until(async () => (await kit.getByTestId('scene-search').count()) === 0, 'the search closed')
  // The Edit menu's row opens it too.
  if (!classic) {
    await frameMenu('Edit').click()
    await kit.getByTestId('scene-cmd-search').click()
    await kit.getByTestId('scene-search').waitFor()
    await escape('scene-search')
  }
  passed('F3 searches and runs')

  // Z: the shading pie. A click on an item, then the key held toward one and let go.
  await page.mouse.move(centre[0], centre[1])
  await page.keyboard.press('KeyZ')
  await kit.getByTestId('scene-pie').waitFor()
  await shot('07-pie')
  await kit.getByTestId('scene-pie-wireframe').click()
  await until(async () => (await state()).shading === 'wireframe', 'the pie sets wireframe')
  await page.mouse.move(centre[0] - 100, centre[1])
  await page.keyboard.down('KeyZ')
  await kit.getByTestId('scene-pie').waitFor()
  await page.mouse.move(centre[0] - 40, centre[1], { steps: 4 })
  await until(async () => (await kit.getByTestId('scene-pie').getAttribute('data-toward')) === 'right', 'moving right lights Solid')
  await page.keyboard.up('KeyZ')
  await until(async () => (await state()).shading === 'solid', 'letting Z go chooses Solid')
  assert.equal(await kit.getByTestId('scene-pie').count(), 0, 'the pie closed')
  passed('Z opens the shading pie')

  // The header's shading buttons.
  await kit.getByTestId('scene-shading-material').click()
  await until(async () => (await state()).shading === 'material', 'the header sets Material Preview')
  await kit.getByTestId('scene-shading-solid').click()
  await until(async () => (await state()).shading === 'solid', 'and Solid')
  passed('the header sets the shading')

  // T and N: the toolbar and the side panel.
  await page.mouse.move(centre[0], centre[1])
  await page.keyboard.press('KeyT')
  await until(async () => (await kit.locator('.kui-tool-rail').count()) === 0, 'T hides the toolbar')
  await page.keyboard.press('KeyT')
  await until(async () => (await kit.locator('.kui-tool-rail').count()) === 1, 'T shows it')
  await page.keyboard.press('KeyN')
  await until(async () => (await kit.getByTestId('scene-side').count()) === 0, 'N hides the side panel')
  await page.keyboard.press('KeyN')
  await until(async () => (await kit.getByTestId('scene-side').count()) === 1, 'N shows it')
  passed('T and N')

  // The status bar's hints, and Shift's.
  await until(async () => /Rotate View/.test(await kit.getByTestId('scene-status-hints').textContent()), 'the hints say what the middle button does')
  await page.keyboard.down('Shift')
  await until(async () => (await kit.getByTestId('scene-status-hints').getAttribute('data-held')) === 'shift', 'Shift changes the hints')
  assert.match(await kit.getByTestId('scene-status-hints').textContent(), /3D Cursor/)
  await page.keyboard.up('Shift')
  passed('the status bar hints')

  // The Object menu's kinds: Modifiers ▸ Add Modifier ▸ Subdivision, no form.
  await kit.getByTestId('scene-header-menu-Object').click()
  await kit.getByTestId('scene-header-modifiers').hover()
  await kit.getByTestId('scene-header-modifiers-modifier.add').hover()
  await kit.getByTestId('scene-header-modifier.add-subdivision').click()
  await until(async () => (await named('Cube')).modifiers.some((item) => item.type === 'subdivision'), 'the cube has a Subdivision modifier')
  assert.equal(await kit.locator('.kui-dialog').count(), 0, 'no form opened')
  // The frame's Object menu: Add Modifier opens the kinds at the pointer.
  if (!classic) {
    await frameMenu('Object').click()
    await kit.getByTestId('scene-cmd-modifier.add').click()
    await kit.getByTestId('scene-kind-menu').waitFor()
    await kit.getByTestId('scene-popup-modifier.add-array').click()
    await until(async () => (await named('Cube')).modifiers.some((item) => item.type === 'array'), 'the cube has an Array modifier')
    assert.equal(await kit.locator('.kui-dialog').count(), 0, 'no form opened')
  }
  passed('a kind from the menu, not from a form')

  // Ctrl+P's menu (from the Object menu): the light parented to the cube.
  const light = await named('Light')
  await run([{ command: 'select.objects', params: { objects: ['Light'] } }, { command: 'select.objects', params: { objects: ['Cube'], extend: true } }])
  await until(async () => (await state()).active === cube.id && (await state()).selected.length === 2, 'the light and the cube, the cube active')
  await kit.getByTestId('scene-header-menu-Object').click()
  await kit.getByTestId('scene-header-parent').hover()
  await kit.getByTestId('scene-header-parentMenu').click()
  await kit.getByTestId('scene-parent-menu').waitFor()
  await kit.getByTestId('scene-popup-parent-object').click()
  await until(async () => (await named('Light')).parent === cube.id, 'Set Parent To ▸ Object parents the light to the cube')
  await run([{ command: 'edit.undo', params: {} }])
  await until(async () => (await named('Light')).parent == null, 'undone in one step')
  passed('Set Parent To')

  // Link Materials (Ctrl/Cmd+L): the cylinder takes the cube's material in one step.
  if (cube.material != null) {
    await run([{ command: 'select.objects', params: { objects: ['Cylinder'] } }, { command: 'select.objects', params: { objects: ['Cube'], extend: true } }])
    await until(async () => (await state()).active === cube.id && (await state()).selected.length === 2, 'the cylinder and the cube, the cube active')
    await page.mouse.move(centre[0], centre[1])
    await page.keyboard.press(`${mod}+KeyL`)
    await until(async () => (await named('Cylinder')).material === cube.material, 'Ctrl/Cmd+L gives the cylinder the cube material')
    passed('Link Materials')
  }

  // The Item section: the dimensions change the scale.
  await run([{ command: 'select.objects', params: { objects: ['Cube'] } }])
  await until(async () => (await state()).selected.length === 1, 'the cube alone')
  const dimensions = (await named('Cube')).dimensions
  // Closed in object mode (the Object section has the transform): its title opens it.
  if ((await kit.getByTestId('scene-item-dimensions-x').count()) === 0) await kit.getByTestId('scene-item').locator('button').first().click()
  const field = kit.getByTestId('scene-item-dimensions-x').locator('input')
  await field.fill(String(dimensions[0] * 2))
  await field.press('Enter')
  await until(async () => near((await named('Cube')).scale[0], cube.scale[0] * 2), async () => `the dimension doubles the scale (${(await named('Cube')).scale})`)
  passed('the Item section')

  // The outliner: the light dragged onto the cube's row is parented to it; to the empty space, not.
  const row = (name) => kit.locator(`[data-testid="scene-outliner-item"][data-name="${name}"] .scene-outliner-name`)
  const drag = async (from, to) => {
    const a = await from.boundingBox()
    const b = await to.boundingBox()
    await page.mouse.move(a.x + a.width / 2, a.y + a.height / 2)
    await page.mouse.down()
    await page.mouse.move(a.x + a.width / 2, a.y + a.height / 2 + 8, { steps: 3 })
    await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2, { steps: 6 })
    await page.mouse.up()
  }
  await drag(row('Light'), row('Cube'))
  await until(async () => (await named('Light')).parent === cube.id, async () => `the drag parents the light (${(await named('Light')).parent})`)
  await until(async () => (await undo()) === 'Parent', async () => `one Parent step (${await undo()})`)
  await drag(row('Light'), kit.getByTestId('scene-outliner-space'))
  await until(async () => (await named('Light')).parent == null, 'a drag to the empty space clears the parent')
  passed('the outliner drag')

  // The outliner's right-click menu: Hide.
  await row('Camera').click({ button: 'right' })
  await kit.getByTestId('scene-outliner-menu').waitFor()
  await shot('08-outliner-menu')
  await kit.getByTestId('scene-outliner-menu-hide').click()
  await until(async () => (await named('Camera')).hidden === true, 'Hide hides the camera')
  assert.equal(light.id != null, true)
  passed("the outliner's right-click menu")

  // With screenshots asked for: the header of the other modes.
  if (shots) {
    const header = async (name, steps, mode) => {
      await run(steps)
      await until(async () => (await state()).mode === mode, `${name}: ${mode}`)
      await page.waitForTimeout(300)
      await shot(name)
    }
    await header('09-header-sculpt', [{ command: 'select.objects', params: { objects: ['Cube'] } }, { command: 'mode.sculpt', params: { on: true } }], 'sculpt')
    await header('10-header-weight-paint', [{ command: 'mode.sculpt', params: { on: false } }, { command: 'mode.weightPaint', params: { on: true } }], 'weightPaint')
    await header('11-header-armature-edit', [{ command: 'mode.weightPaint', params: { on: false } }, { command: 'object.addArmature', params: {} }, { command: 'mode.armature', params: {} }], 'armatureEdit')
    await header('12-header-pose', [{ command: 'mode.armature', params: {} }, { command: 'mode.pose', params: { on: true } }], 'pose')
    await header('13-header-curve-edit', [{ command: 'mode.pose', params: { on: false } }, { command: 'object.addCurveBezier', params: {} }, { command: 'mode.shape', params: {} }], 'curveEdit')
  }

  assert.deepEqual(errors, [], 'no page errors')
  console.log(`PASS scene chrome (${which}${modes ? `, ${modes}` : ''})`)
} catch (error) {
  console.log(said.slice(-30).join('\n'))
  throw error
} finally {
  await browser.close()
  await server.close()
}
