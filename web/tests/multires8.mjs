// Actual modifier controls, person undo/redo, worker execution, and host save/reopen.
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { chromium, webkit } from 'playwright'
import { createServer } from 'vite'

const which = process.argv[2] ?? 'chromium'
const server = await createServer({ root: new URL('..', import.meta.url).pathname, configFile: new URL('../vite.config.ts', import.meta.url).pathname, server: { port: 0, host: '127.0.0.1' }, logLevel: 'error' })
await server.listen()
const browser = await (which === 'webkit' ? webkit : chromium).launch()
const page = await browser.newPage({ viewport: { width: 1600, height: 1100 } })
const errors = []
page.on('pageerror', error => errors.push(error.message))
const fixture = JSON.parse(await readFile(new URL('../../crates/engine/model/tests/fixtures/multires8-reference.json', import.meta.url), 'utf8')).cases[2].input
const run = async (command, params = {}) => page.evaluate(({ command, params }) => window.devHost.connection().ai.run([{ command, params }], false), { command, params })
const details = async () => (await page.evaluate(() => window.devHost.connection().ai.inspect())).details
const cube = async () => (await details()).objects.find(o => o.name === 'Cube')
const until = async check => {
  for (let i = 0; i < 200; i += 1) { if (await check()) return; await page.waitForTimeout(50) }
  throw new Error('Timed out waiting for Multires recovery')
}
try {
  await page.goto(`http://127.0.0.1:${server.httpServer.address().port}/dev.html`)
  await page.waitForFunction(() => document.getElementById('status')?.textContent?.includes(' open in '), null, { timeout: 60000 })
  const kit = page.frameLocator('#kit')
  await run('select.objects', { objects: 'Cube' })
  await run('mesh.setGeometry', { object: 'Cube', positions: fixture.positions, faces: fixture.faces })
  await kit.getByTestId('scene-add-modifier').click()
  await kit.getByTestId('scene-add-modifier-multires').click()
  await until(async () => (await cube()).modifiers?.[0]?.type === 'multires')
  await kit.getByTestId('scene-multires-unsubdivide').click()
  await until(async () => (await cube()).mesh.multiresLevels === 1)
  assert.equal((await cube()).mesh.vertices, 26)
  assert.equal((await cube()).modifiers[0].levels, 0)
  assert.equal(await kit.getByTestId('scene-multires-rebuild').isEnabled(), false)
  await kit.getByTestId('scene-undo').click()
  await until(async () => !(await cube()).mesh.multiresLevels)
  assert.equal((await cube()).mesh.vertices, 98)
  await kit.getByTestId('scene-redo').click()
  await until(async () => (await cube()).mesh.multiresLevels === 1)
  await kit.getByTestId('scene-undo').click()
  await until(async () => !(await cube()).mesh.multiresLevels)
  await kit.getByTestId('scene-multires-rebuild').click()
  await until(async () => (await cube()).mesh.multiresLevels === 2)
  assert.equal((await cube()).mesh.vertices, 8)
  assert.equal((await cube()).mesh.evaluated.vertices, 98)
  assert.equal((await cube()).modifiers[0].levels, 2)
  assert.equal((await cube()).modifiers[0].sculptLevels, 2)
  const rebuilt = await cube()
  await kit.getByTestId('scene-undo').click()
  await until(async () => !(await cube()).mesh.multiresLevels)
  assert.equal((await cube()).mesh.vertices, 98)
  await kit.getByTestId('scene-redo').click()
  await until(async () => (await cube()).mesh.multiresLevels === 2)
  if (process.env.KITION_MULTIRES_SCREENSHOT) await page.screenshot({ path: process.env.KITION_MULTIRES_SCREENSHOT })
  console.log(`PASS ${which}: both panel operators and person undo/redo`)

  // A base cube cannot be recovered again. Its failed operator must retain history and shape.
  const undo = (await details()).undo
  await kit.getByTestId('scene-multires-unsubdivide').click()
  await page.waitForTimeout(200)
  assert.deepEqual((await cube()).mesh, rebuilt.mesh)
  assert.deepEqual((await cube()).dimensions, rebuilt.dimensions)
  assert.equal((await details()).undo, undo)
  await kit.getByTestId('scene-save').click()
  await until(async () => (await page.evaluate(() => window.devHost.files.get('Sample.kscene').version)) > 1)
  await page.evaluate(async () => {
    const file = window.devHost.files.get('Sample.kscene')
    await window.devHost.open('Sample.kscene', file.bytes)
  })
  await until(async () => (await cube()).mesh.multiresLevels === 2)
  assert.deepEqual((await cube()).mesh, rebuilt.mesh)
  assert.deepEqual((await cube()).dimensions, rebuilt.dimensions)
  assert.deepEqual((await cube()).modifiers, rebuilt.modifiers)
  assert.deepEqual(errors, [])
  console.log(`PASS ${which}: unsupported topology stays unchanged; host save/reopen retains detail`)
} finally {
  await browser.close()
  await server.close()
}
