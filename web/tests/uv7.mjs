// Real worker, host writes, live pointer preview, cancellation and one-step undo in both engines.
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { chromium, webkit } from 'playwright'
import { createServer } from 'vite'

const which = process.argv[2] ?? 'chromium'
const server = await createServer({ root: new URL('..', import.meta.url).pathname, configFile: new URL('../vite.config.ts', import.meta.url).pathname, server: { port: 0, host: '127.0.0.1' }, logLevel: 'error' })
await server.listen()
const browser = await (which === 'webkit' ? webkit : chromium).launch()
const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } })
const errors = []
page.on('pageerror', error => errors.push(error.message))
const origin = `http://127.0.0.1:${server.httpServer.address().port}`
const fixture = JSON.parse(await readFile(new URL('../../crates/engine/model/tests/fixtures/uv7-cases.json', import.meta.url), 'utf8'))[0]
const run = async (command, params = {}) => (await page.evaluate(({ command, params }) => window.devHost.connection().ai.run([{ command, params }], false), { command, params })).completed[0].result
const layout = () => run('uv.layout', { all: true })
const until = async (check) => {
  for (let i = 0; i < 150; i += 1) { if (await check()) return; await page.waitForTimeout(40) }
  throw new Error('Timed out waiting for UV editor state')
}
const vertex = (layout, id) => {
  const face = layout.faces.find(face => face.vertices.includes(id))
  return face.uvs[face.vertices.indexOf(id)]
}
try {
  await page.goto(`${origin}/dev.html`)
  await page.waitForFunction(() => document.getElementById('status')?.textContent?.includes(' open in '), null, { timeout: 60000 })
  const kit = page.frameLocator('#kit')
  const frame = page.frames().find(frame => frame.url().includes('index.html'))
  await run('select.objects', { objects: 'Cube' })
  await run('mesh.setGeometry', { object: 'Cube', positions: fixture.positions, faces: fixture.faces })
  await run('mode.set', { mode: 'edit' })
  await kit.getByTestId('scene-uv-toggle').click()
  await kit.getByTestId('scene-uv-method').selectOption('minimumStretch')
  await until(async () => (await layout()).method === 'minimumStretch')
  await kit.getByTestId('scene-uv-unwrap').click()
  await until(async () => (await layout()).hasUvs)
  const unwrapped = await layout()
  assert.equal(unwrapped.faces.length, 32)
  assert(unwrapped.faces.every(face => face.uvs.every(uv => uv.every(Number.isFinite))))
  await run('mesh.select', { vertices: [0, 4] })
  await kit.getByTestId('scene-uv-pin').click()
  await until(async () => (await layout()).pinned.length === 2)
  await run('mesh.select', { vertices: [4] })
  await kit.getByTestId('scene-uv-live').click()
  await until(async () => (await layout()).liveUnwrap)
  const before = await layout()
  if (process.env.KITION_UV_SCREENSHOT) await page.screenshot({ path: process.env.KITION_UV_SCREENSHOT })
  const xy = id => frame.evaluate(id => {
    const element = document.querySelector(`.scene-uv-vertex[data-vertex="${id}"]`)
    return [Number(element.getAttribute('x')) + Number(element.getAttribute('width')) / 2, Number(element.getAttribute('y')) + Number(element.getAttribute('height')) / 2]
  }, id)
  const beforeInterior = await xy(12)
  const fixed = await xy(0)
  const pin = await kit.locator('.scene-uv-vertex[data-vertex="4"]').first().boundingBox()
  const x = pin.x + pin.width / 2
  const y = pin.y + pin.height / 2
  await page.mouse.move(x, y)
  await page.mouse.down()
  await page.mouse.move(x + 30, y - 15, { steps: 6 })
  await until(async () => JSON.stringify(await xy(12)) !== JSON.stringify(beforeInterior))
  assert.deepEqual(await xy(0), fixed)
  // A preview relaxes the picture while the saved model still contains its original UVs.
  assert.deepEqual((await layout()).faces, before.faces)
  await page.mouse.up()
  await until(async () => JSON.stringify((await layout()).faces) !== JSON.stringify(before.faces))
  const moved = await layout()
  assert.deepEqual(vertex(moved, 0), vertex(before, 0))
  assert.notDeepEqual(vertex(moved, 12), vertex(before, 12))
  await kit.locator('.kui-app').focus()
  await page.keyboard.press('Meta+z')
  await until(async () => JSON.stringify((await layout()).faces) === JSON.stringify(before.faces))
  await kit.getByTestId('scene-redo').click()
  await until(async () => !(await kit.getByTestId('scene-redo').isEnabled()))
  await until(async () => JSON.stringify((await layout()).faces) === JSON.stringify(moved.faces))
  console.log(`PASS ${which}: live pointer preview, fixed pins, one-step undo and redo`)

  // A selection arriving during a quick drag cancels its preview and prevents a later reply
  // from reviving either that preview or a final transform against a different selection.
  const currentPin = await kit.locator('.scene-uv-vertex[data-vertex="4"]').first().boundingBox()
  await page.mouse.move(currentPin.x + currentPin.width / 2, currentPin.y + currentPin.height / 2)
  await page.mouse.down()
  await page.mouse.move(currentPin.x + 60, currentPin.y + 20)
  await run('mesh.select', { vertices: [0] })
  await page.mouse.up()
  await page.waitForTimeout(150)
  assert.deepEqual((await layout()).faces.map(f => f.uvs), moved.faces.map(f => f.uvs))
  console.log(`PASS ${which}: selection change discards in-flight previews`)

  await page.evaluate(() => window.devHost.connection().save())
  await page.evaluate(async () => {
    const file = window.devHost.files.get('Sample.kscene')
    await window.devHost.open('Sample.kscene', file.bytes)
  })
  await run('mode.set', { mode: 'edit' })
  const reopened = await layout()
  assert.equal(reopened.liveUnwrap, true)
  assert.equal(reopened.method, 'minimumStretch')
  assert.deepEqual(reopened.faces.map(f => f.uvs), moved.faces.map(f => f.uvs))
  assert.deepEqual(errors, [])
  console.log(`PASS ${which}: host save and reopen preserve UVs and local preferences`)
} finally {
  await browser.close()
  await server.close()
}
