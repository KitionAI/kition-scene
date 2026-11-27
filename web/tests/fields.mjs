// Number fields and animation basics by hand (plan scene-usable-by-hand, phase 8): node tests/fields.mjs [chromium|webkit]
// Real mouse and keyboard events on the development host, read back through the editor's state:
// units and arithmetic typed into a field, an entry that does not read (the field shakes, nothing
// is sent), the wheel over a field changing nothing, a label drag with Ctrl/Cmd (whole steps) and
// Shift (fine), a double click on the label typing into the field, Backspace over a field
// resetting its vector, Ctrl/Cmd+C and V over fields, the field's right-click menu (Insert
// Keyframe, Reset to Default from the engine's description), a tap of Space playing and pausing
// (a hold does not), Escape stopping playback where it began, the arrows stepping through frames
// and keys, I and K keying, auto-keying a modal move, a field and Alt+G in their own steps, a
// material shared by two objects copied for one by its user count, and a mesh without a material
// getting one from its swatch.
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
  const mac = await page.evaluate(() => /Mac|iPhone|iPad/i.test(navigator.platform))
  const mod = mac ? 'Meta' : 'Control'
  const run = (steps) => frame.evaluate((steps) => window.kitScene.run(steps), steps)
  const state = () => frame.evaluate(() => window.kitScene.state())
  const named = async (name) => (await state()).objects.find((item) => item.name === name)
  const undo = async () => (await page.evaluate(async () => window.devHost.connection().ai.inspect())).details.undo
  const animation = async () => (await page.evaluate(async () => window.devHost.connection().ai.inspect())).details.animation
  const shot = async (name) => {
    if (shots) await page.screenshot({ path: `${shots}/${name}.png` })
  }
  const field = (id) => kit.getByTestId(id).locator('input')
  const type = async (id, text) => {
    await field(id).fill(text)
    await field(id).press('Enter')
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
  const hover = async (id) => {
    // Panels scroll: the field is brought into the panel's view first (the pointer must be over it).
    await kit.getByTestId(id).scrollIntoViewIfNeeded()
    const target = await kit.getByTestId(id).boundingBox()
    await page.mouse.move(target.x + target.width * 0.7, target.y + target.height / 2)
  }
  const centre = [box.x + box.width / 2, box.y + box.height * 0.6]

  await run([{ command: 'select.objects', params: { objects: ['Cube'] } }])
  await until(async () => (await state()).active === (await named('Cube')).id, 'the cube is active')
  await field('scene-location-x').waitFor()

  // N17: units and arithmetic, in the field's own unit when bare.
  const typed = [
    ['scene-location-x', '50cm', 0, 0.5],
    ['scene-location-y', '2mm', 1, 0.002],
    ['scene-location-z', '1.5m', 2, 1.5],
    ['scene-location-x', '1+1', 0, 2],
    ['scene-location-x', '2*3', 0, 6],
    ['scene-location-x', '1/3', 0, 0.333],
    ['scene-location-y', '2 + 2mm', 1, 0.004],
    ['scene-location-x', '1m 50cm', 0, 1.5],
    ['scene-location-y', '(1+2)*50cm', 1, 1.5],
  ]
  for (const [id, text, index, expected] of typed) {
    await type(id, text)
    await until(async () => near((await named('Cube')).location[index], expected), async () => `${text} gives ${expected} (${(await named('Cube')).location})`)
  }
  await type('scene-rotation-z', '45deg')
  await until(async () => near((await named('Cube')).rotation[2], 45), '45deg gives 45°')
  await type('scene-rotation-z', '1rad')
  await until(async () => near((await named('Cube')).rotation[2], 57.3, 0.01), async () => `1rad gives 57.3° (${(await named('Cube')).rotation[2]})`)
  await type('scene-rotation-z', '0')
  await until(async () => near((await named('Cube')).rotation[2], 0), 'back to 0°')
  passed('typed units and arithmetic')

  // An entry that does not read shakes the field, puts the value back and sends nothing.
  const before = await steadyUndo()
  const locationBefore = (await named('Cube')).location
  for (const text of ['abc', '1+', '45deg', '2//3']) {
    await field('scene-location-x').fill(text)
    await field('scene-location-x').press('Enter')
    await until(async () => (await kit.getByTestId('scene-location-x').getAttribute('data-invalid')) === 'true', `${text} shakes the field`)
    assert.equal(await field('scene-location-x').inputValue(), '1.5', `${text}: the field shows its value again`)
    await until(async () => (await kit.getByTestId('scene-location-x').getAttribute('data-invalid')) == null, 'the shake ends')
  }
  await page.waitForTimeout(300)
  assert.deepEqual((await named('Cube')).location, locationBefore, 'nothing moved')
  assert.equal(await undo(), before, 'no command was sent')
  passed('an entry that does not read shakes the field and sends nothing')

  // The wheel over a field changes nothing.
  await hover('scene-location-x')
  await page.mouse.wheel(0, -200)
  await page.mouse.wheel(0, 200)
  await page.waitForTimeout(300)
  assert.deepEqual((await named('Cube')).location, locationBefore, 'the wheel over a field changes nothing')
  passed('the wheel over a field changes nothing')

  // Backspace over a field resets its vector to the default (location 0, 0, 0), one step.
  await hover('scene-location-y')
  await page.keyboard.press('Backspace')
  await until(async () => (await named('Cube')).location.every((value) => value === 0), async () => `Backspace resets the location (${(await named('Cube')).location})`)
  assert.equal(await steadyUndo(), 'Set Transform', 'one Set Transform step')
  assert.ok(await named('Cube'), 'Backspace over a field deletes nothing')
  passed('Backspace over a field resets it')

  // Label drags: Ctrl/Cmd in whole steps, Shift finely (the field's step is 0.1 m per 2 px).
  const drag = async (id, dx, modifier) => {
    await kit.getByTestId(`${id}-label`).scrollIntoViewIfNeeded()
    const label = await kit.getByTestId(`${id}-label`).boundingBox()
    const y = label.y + label.height / 2
    await page.mouse.move(label.x + label.width / 2, y)
    if (modifier) await page.keyboard.down(modifier)
    await page.mouse.down()
    for (let step = 1; step <= 4; step += 1) {
      await page.mouse.move(label.x + label.width / 2 + (dx * step) / 4, y)
      await page.waitForTimeout(30)
    }
    await page.mouse.up()
    if (modifier) await page.keyboard.up(modifier)
  }
  await drag('scene-location-x', 46)
  await until(async () => near((await named('Cube')).location[0], 2.3), async () => `a 46 px drag moves 2.3 m (${(await named('Cube')).location[0]})`)
  await hover('scene-location-x')
  await page.keyboard.press('Backspace')
  await until(async () => (await named('Cube')).location[0] === 0, 'reset')
  await drag('scene-location-x', 46, mod)
  await until(async () => near((await named('Cube')).location[0], 2), async () => `Ctrl/Cmd steps to 2 m (${(await named('Cube')).location[0]})`)
  await hover('scene-location-x')
  await page.keyboard.press('Backspace')
  await until(async () => (await named('Cube')).location[0] === 0, 'reset')
  await drag('scene-location-x', 46, 'Shift')
  await until(async () => near((await named('Cube')).location[0], 0.23), async () => `Shift moves finely, 0.23 m (${(await named('Cube')).location[0]})`)
  passed('a label drag steps with Ctrl/Cmd and goes finely with Shift')

  // A double click on the label types into the field, its value selected.
  await kit.getByTestId('scene-location-x-label').dblclick()
  await until(async () => frame.evaluate(() => document.activeElement?.closest?.('[data-testid="scene-location-x"]') != null), 'the field takes the keyboard')
  await page.keyboard.type('4')
  await page.keyboard.press('Enter')
  await until(async () => (await named('Cube')).location[0] === 4, 'what is typed replaces the value')
  passed('a double click on the label types into the field')

  // Ctrl/Cmd+C over one field and Ctrl/Cmd+V over another copy the value.
  await hover('scene-location-x')
  await page.keyboard.press(`${mod}+KeyC`)
  await until(async () => (await frame.evaluate(() => window.sceneFieldCopied?.())) === '4', 'Ctrl/Cmd+C copies the value under the pointer')
  await hover('scene-scale-z')
  await page.keyboard.press(`${mod}+KeyV`)
  await until(async () => (await named('Cube')).scale[2] === 4, 'Ctrl/Cmd+V pastes it into the field under the pointer')
  assert.equal(await steadyUndo(), 'Set Transform')
  passed('Ctrl/Cmd+C and V over fields')

  // The right button: Copy, Paste, Reset to Default, Insert Keyframe.
  await kit.getByTestId('scene-scale-x').scrollIntoViewIfNeeded()
  const fieldBox = await kit.getByTestId('scene-scale-x').boundingBox()
  await page.mouse.click(fieldBox.x + fieldBox.width * 0.7, fieldBox.y + fieldBox.height / 2, { button: 'right' })
  await kit.getByTestId('scene-field-menu').waitFor()
  for (const row of ['copy', 'paste', 'reset', 'insertKey']) assert.equal(await kit.getByTestId(`scene-popup-${row}`).count(), 1, `the menu has ${row}`)
  await shot('01-field-menu')
  await kit.getByTestId('scene-popup-reset').click()
  await until(async () => (await named('Cube')).scale.every((value) => value === 1), 'Reset to Default puts the scale back to 1')
  await page.mouse.click(fieldBox.x + fieldBox.width * 0.7, fieldBox.y + fieldBox.height / 2, { button: 'right' })
  await kit.getByTestId('scene-popup-insertKey').click()
  await until(async () => (await named('Cube')).animated?.join() === 'scale', async () => `Insert Keyframe keys the scale only (${(await named('Cube')).animated})`)
  assert.equal(await steadyUndo(), 'Insert Keyframe')
  await run([{ command: 'edit.undo', params: {} }])
  await until(async () => !(await named('Cube')).animated?.length, 'undone')
  passed("the field's right-click menu")

  // A default the engine describes: the index of refraction resets to 1.5.
  await type('scene-ior', '2.2')
  const cubeMaterial = async () => {
    const material = (await named('Cube')).material
    return (await state()).materials.find((item) => item.id === material)
  }
  await until(async () => (await cubeMaterial())?.ior === 2.2, 'IOR 2.2')
  await hover('scene-ior')
  await page.keyboard.press('Backspace')
  await until(async () => (await cubeMaterial())?.ior === 1.5, "Backspace puts the IOR back to the engine's 1.5")
  passed("Reset to Default takes the engine's described default")

  // N18: a tap of Space plays and pauses; a hold does not; Escape stops at where it began.
  await page.mouse.move(centre[0], centre[1])
  await kit.locator('.kui-app').focus()
  const play = kit.getByTestId('scene-timeline-play')
  const startFrame = (await animation()).frame
  await page.keyboard.press('Space')
  await until(async () => (await play.getAttribute('data-playing')) === 'true', 'a tap of Space plays')
  await until(async () => (await animation()).frame > startFrame + 2, 'the frames advance')
  await page.keyboard.press('Space')
  await until(async () => (await play.getAttribute('data-playing')) == null, 'a second tap pauses')
  // A frame asked for just before the pause lands first (on a slow machine a moment later).
  await until(async () => (await kit.getByTestId('scene-timeline').getAttribute('data-seeking')) == null, 'no frame on its way')
  const paused = (await animation()).frame
  await page.waitForTimeout(400)
  assert.equal((await animation()).frame, paused, 'paused, the frame stays')
  await page.keyboard.down('Space')
  await page.waitForTimeout(500)
  await page.keyboard.up('Space')
  await page.waitForTimeout(300)
  assert.equal(await play.getAttribute('data-playing'), null, 'a hold of Space does not play')
  await page.keyboard.press('Space')
  await until(async () => (await play.getAttribute('data-playing')) === 'true', 'plays again')
  await until(async () => (await animation()).frame > paused + 2, 'advancing')
  await page.keyboard.press('Escape')
  await until(async () => (await play.getAttribute('data-playing')) == null && (await animation()).frame === paused, async () => `Escape stops at the frame it began at (${(await animation()).frame}, began at ${paused})`)
  passed('Space plays and pauses, Escape stops')

  // The arrows: a frame, the start and end, and the keys.
  await run([{ command: 'frame.set', params: { frame: 5 } }])
  await until(async () => (await animation()).frame === 5, 'frame 5')
  await page.keyboard.press('ArrowRight')
  await until(async () => (await animation()).frame === 6, '→ goes to the next frame')
  await page.keyboard.press('ArrowLeft')
  await page.keyboard.press('ArrowLeft')
  await until(async () => (await animation()).frame === 4, '← goes back')
  await page.keyboard.press('Shift+ArrowRight')
  await until(async () => (await animation()).frame === (await animation()).end, 'Shift+→ goes to the end')
  await page.keyboard.press('Shift+ArrowLeft')
  await until(async () => (await animation()).frame === (await animation()).start, 'Shift+← goes to the start')
  passed('the arrows step through frames')

  // I keys location, rotation and scale with no menu; K offers the keying sets.
  await page.keyboard.press('KeyI')
  await until(async () => (await named('Cube')).animated?.join() === 'location,rotation,scale', async () => `I keys location, rotation and scale (${(await named('Cube')).animated})`)
  await run([{ command: 'frame.set', params: { frame: 30 } }])
  await until(async () => (await animation()).frame === 30, 'frame 30')
  await page.mouse.move(centre[0], centre[1])
  await page.keyboard.press('KeyK')
  await kit.getByTestId('scene-keying-menu').waitFor()
  await shot('02-keying-menu')
  await kit.getByTestId('scene-popup-ks-location').click()
  await until(async () => JSON.stringify((await animation()).selectedKeyFrames) === '[1,30]', async () => `K ▸ Location keys frame 30 (${(await animation()).selectedKeyFrames})`)
  await kit.locator('.kui-app').focus()
  await page.keyboard.press('ArrowDown')
  await until(async () => (await animation()).frame === 1, '↓ jumps to the key before')
  await page.keyboard.press('ArrowUp')
  await until(async () => (await animation()).frame === 30, '↑ jumps to the next key')
  await run([{ command: 'animation.clear', params: {} }])
  await until(async () => !(await named('Cube')).animated?.length, 'keys cleared')
  passed('I, K and the up and down arrows')

  // Auto-keying: the timeline's record button; a modal move, a field and Alt+G key in their own step.
  await kit.getByTestId('scene-timeline-autokey').click()
  await until(async () => (await state()).transform.autoKey === true, 'the record button turns auto-keying on')
  await run([{ command: 'frame.set', params: { frame: 20 } }])
  await until(async () => (await animation()).frame === 20, 'frame 20')
  await page.mouse.move(centre[0], centre[1])
  await kit.locator('.kui-app').focus()
  await page.keyboard.press('KeyG')
  await until(async () => (await state()).transform != null && (await frame.evaluate(() => window.sceneModal?.answer?.active)) === true, 'G moves')
  await page.keyboard.press('KeyZ')
  await page.keyboard.press('Digit2')
  await page.keyboard.press('Enter')
  await until(async () => near((await named('Cube')).location[2], 2) && (await named('Cube')).animated?.join() === 'location', async () => `the move keys the location (${(await named('Cube')).animated})`)
  assert.equal(await steadyUndo(), 'Move', 'the keys are in the Move step')
  assert.deepEqual((await animation()).selectedKeyFrames, [20])
  await shot('03-auto-key')
  await run([{ command: 'edit.undo', params: {} }])
  await until(async () => near((await named('Cube')).location[2], 0) && !(await named('Cube')).animated?.length, 'one undo takes the move and its keys back')
  await type('scene-rotation-x', '30')
  await until(async () => (await named('Cube')).animated?.join() === 'rotation', async () => `a field keys its property (${(await named('Cube')).animated})`)
  // A batch of commands is one step, which the engine names Batch Edit (case regressions/issue-batch-undo-labels).
  assert.equal(await steadyUndo(), 'Batch Edit', 'in one step with the field change')
  await run([{ command: 'edit.undo', params: {} }])
  await until(async () => (await named('Cube')).rotation[0] === 0 && !(await named('Cube')).animated?.length, 'one undo takes the change and its key back')
  await type('scene-location-x', '3')
  await until(async () => (await named('Cube')).location[0] === 3, 'x 3')
  await run([{ command: 'animation.clear', params: {} }])
  await until(async () => !(await named('Cube')).animated?.length, 'keys cleared')
  await page.mouse.move(centre[0], centre[1])
  await kit.locator('.kui-app').focus()
  await page.keyboard.press('Alt+KeyG')
  await until(async () => (await named('Cube')).location[0] === 0 && (await named('Cube')).animated?.join() === 'location', async () => `Alt+G clears and keys the location (${(await named('Cube')).animated})`)
  await run([{ command: 'animation.clear', params: {} }, { command: 'transform.set', params: { autoKey: false } }])
  await until(async () => (await state()).transform.autoKey === false && !(await named('Cube')).animated?.length, 'auto-keying off')
  await type('scene-location-x', '1')
  await until(async () => (await named('Cube')).location[0] === 1, 'x 1')
  await page.waitForTimeout(300)
  assert.ok(!(await named('Cube')).animated?.length, 'off, a field keys nothing')
  passed('auto-keying keys a confirmed move, a field and Alt+G in their own step')

  // A12: a copy shares its material; its user count makes it a material of its own, one step.
  const materialOf = async (name) => {
    const material = (await named(name)).material
    return (await state()).materials.find((item) => item.id === material)
  }
  await run([{ command: 'material.set', params: { material: (await materialOf('Cube')).id, metallic: 0.25, emission: [0.1, 0.2, 0.3] } }])
  await run([{ command: 'edit.duplicate', params: {} }])
  await until(async () => (await named('Cube.001')) != null, 'the cube is copied')
  await run([{ command: 'select.objects', params: { objects: ['Cube.001'] } }])
  await until(async () => (await state()).active === (await named('Cube.001')).id, 'the copy is active')
  const users = kit.getByTestId('scene-material-users')
  await users.waitFor()
  assert.equal((await users.textContent()).trim(), '2', 'two objects use the material')
  await shot('04-material-users')
  const shared = await materialOf('Cube')
  await users.click()
  await until(async () => (await materialOf('Cube.001'))?.id !== shared.id, 'the copy gets a material of its own')
  const own = await materialOf('Cube.001')
  assert.equal(own.name, `${shared.name}.001`)
  assert.equal(own.metallic, 0.25, 'with the same settings')
  assert.deepEqual(own.emission.map((value) => Math.round(value * 100) / 100), [0.1, 0.2, 0.3], 'its emission too')
  assert.equal((await materialOf('Cube')).id, shared.id, 'the cube keeps its material')
  assert.equal(await steadyUndo(), 'Batch Edit', 'one step')
  await until(async () => (await kit.getByTestId('scene-material-users').count()) === 0, 'one user now: no count')
  await run([{ command: 'edit.undo', params: {} }])
  await until(async () => (await materialOf('Cube.001'))?.id === shared.id && !(await state()).materials.some((item) => item.name === `${shared.name}.001`), 'one undo shares it again')
  passed('a shared material copied for one object')

  // A new mesh has no material: its swatch makes one for it.
  await run([{ command: 'object.addCylinder', params: {} }])
  await until(async () => (await named('Cylinder'))?.id === (await state()).active, 'a cylinder, active')
  const cylinder = await named('Cylinder')
  assert.equal(cylinder.material ?? null, null, 'with no material')
  const swatch = kit.getByTestId('scene-material-add-color')
  await swatch.waitFor()
  await shot('05-material-swatch')
  const materialsBefore = (await state()).materials.length
  await swatch.click()
  await until(async () => (await state()).objects.find((item) => item.id === cylinder.id)?.material != null, 'the swatch gives it a material')
  assert.equal((await state()).materials.length, materialsBefore + 1, 'a new one')
  assert.equal(await steadyUndo(), 'Batch Edit', 'one step')
  await kit.getByTestId('scene-base-color').waitFor()
  passed('a mesh without a material gets one from its swatch')

  assert.deepEqual(errors, [], 'no page errors')
} catch (error) {
  console.log(said.slice(-40).join('\n'))
  if (shots) await page.screenshot({ path: `${shots}/failure.png` }).catch(() => undefined)
  throw error
} finally {
  await browser.close()
  await server.close()
}
