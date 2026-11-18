// A completed rendered sample must not cancel a live UV drag in the same editor.
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { chromium, webkit } from 'playwright'
import { createServer } from 'vite'

const which = process.argv[2] ?? 'chromium'
const root = new URL('../', import.meta.url).pathname
const server = await createServer({ root, configFile: new URL('../vite.config.ts', import.meta.url).pathname, server: { port: 0, host: '127.0.0.1' }, logLevel: 'error' })
await server.listen()
const browser = await (which === 'webkit' ? webkit : chromium).launch()
const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } })
const errors = []
page.on('pageerror', error => errors.push(error.message))
const fixture = JSON.parse(await readFile(new URL('../../crates/engine/model/tests/fixtures/uv7-cases.json', import.meta.url), 'utf8'))[0]
const run = async (command, params = {}) => (await page.evaluate(({ command, params }) => window.devHost.connection().ai.run([{ command, params }], false), { command, params })).completed[0].result
const layout = () => run('uv.layout', { all: true })
const until = async (check, message) => {
  const end = Date.now() + 15000
  while (Date.now() < end) {
    if (await check()) return
    await page.waitForTimeout(40)
  }
  assert.fail(message)
}
try {
  await page.goto(`http://127.0.0.1:${server.httpServer.address().port}/dev.html`)
  await page.waitForFunction(() => document.getElementById('status')?.textContent?.includes(' open in '), null, { timeout: 60000 })
  const kit = page.frameLocator('#kit')
  const frame = page.frames().find(frame => frame.url().includes('index.html'))
  await run('select.objects', { objects: 'Cube' })
  await run('mesh.setGeometry', { object: 'Cube', positions: fixture.positions, faces: fixture.faces })
  await run('mode.set', { mode: 'edit' })
  await run('uv.unwrap', { method: 'minimumStretch' })
  await run('mesh.select', { vertices: [0, 4] })
  await run('uv.pin')
  await run('mesh.select', { vertices: [4] })
  await run('uv.liveUnwrap', { on: true, method: 'minimumStretch' })
  await kit.getByTestId('scene-uv-toggle').click()
  await run('view.rendered')
  await run('view.renderedStep', { samples: 1, wait: true })
  const before = await layout()
  const xy = id => frame.evaluate(id => {
    const element = document.querySelector(`.scene-uv-vertex[data-vertex="${id}"]`)
    return [Number(element.getAttribute('x')) + Number(element.getAttribute('width')) / 2, Number(element.getAttribute('y')) + Number(element.getAttribute('height')) / 2]
  }, id)
  const fixed = await xy(0)
  const interior = await xy(12)
  const pin = await kit.locator('.scene-uv-vertex[data-vertex="4"]').first().boundingBox()
  await page.mouse.move(pin.x + pin.width / 2, pin.y + pin.height / 2)
  await page.mouse.down()
  await page.mouse.move(pin.x + pin.width / 2 + 30, pin.y + pin.height / 2 - 15, { steps: 6 })
  await until(async () => JSON.stringify(await xy(12)) !== JSON.stringify(interior), 'live UV preview did not relax the interior')
  const preview = await xy(12)
  assert.deepEqual(await xy(0), fixed)
  const progress = await run('view.renderedStep', { samples: 2, wait: true })
  assert.equal(progress.samples, 2)
  assert.deepEqual((await layout()).faces, before.faces, 'sampling and UV preview must leave the saved mesh unchanged')
  await page.waitForTimeout(150)
  assert.deepEqual(await xy(12), preview, 'a display-only revision cancelled the UV preview')
  await page.mouse.up()
  await until(async () => JSON.stringify((await layout()).faces) !== JSON.stringify(before.faces), 'the UV drag was cancelled by rendered sampling')
  await kit.getByTestId('scene-undo').click()
  await until(async () => JSON.stringify((await layout()).faces) === JSON.stringify(before.faces), 'the person\'s undo did not restore the UV layout')
  assert.deepEqual((await layout()).faces, before.faces, 'UV move and relaxation must remain one undo step')
  assert.deepEqual(errors, [])
  console.log(`PASS ${which}: rendered sampling preserves live UV preview and one-step commit`)
} finally {
  await browser.close()
  await server.close()
}
