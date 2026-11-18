// Actual camera controls, person undo/redo, worker execution, and host save/reopen.
import assert from 'node:assert/strict'
import { chromium, webkit } from 'playwright'
import { createServer } from 'vite'

const which = process.argv[2] ?? 'chromium'
const server = await createServer({ root: new URL('..', import.meta.url).pathname, configFile: new URL('../vite.config.ts', import.meta.url).pathname, server: { port: 0, host: '127.0.0.1' }, logLevel: 'error' })
await server.listen()
const browser = await (which === 'webkit' ? webkit : chromium).launch()
const page = await browser.newPage({ viewport: { width: 1600, height: 1100 } })
const errors = []
page.on('pageerror', error => errors.push(error.message))
const details = async () => (await page.evaluate(() => window.devHost.connection().ai.inspect())).details
const camera = async () => (await details()).objects.find(object => object.name === 'Camera').camera
const until = async check => {
  for (let i = 0; i < 200; i += 1) { if (await check()) return; await page.waitForTimeout(50) }
  throw new Error('Timed out waiting for camera editor state')
}
try {
  await page.goto(`http://127.0.0.1:${server.httpServer.address().port}/dev.html`)
  await page.waitForFunction(() => document.getElementById('status')?.textContent?.includes(' open in '), null, { timeout: 60000 })
  const kit = page.frameLocator('#kit')
  const openSection = async testId => {
    const section = kit.getByTestId(testId)
    await section.waitFor()
    if ((await section.getAttribute('data-open')) !== 'true') await section.locator('.kui-panel-section-toggle').first().click()
  }
  const setNumber = async (testId, value) => {
    const input = kit.getByTestId(testId).locator('input')
    await input.fill(String(value))
    await input.press('Enter')
  }
  await kit.locator('[data-testid="scene-outliner-item"][data-name="Camera"] .scene-outliner-name').click()
  await openSection('scene-camera')
  await kit.getByTestId('scene-nav-view.camera').click()
  const fit = kit.getByTestId('scene-camera-sensor-fit')
  assert.equal(await fit.inputValue(), 'auto')
  await setNumber('scene-camera-sensor-width', 39)
  await until(async () => (await camera()).sensor === 39)
  await fit.selectOption('vertical')
  await until(async () => (await camera()).sensorFit === 'vertical')
  await setNumber('scene-camera-sensor-height', 21)
  await until(async () => (await camera()).sensorHeight === 21)
  await kit.getByTestId('scene-undo').click()
  await until(async () => (await camera()).sensorHeight === 24)
  assert.equal((await camera()).sensorFit, 'vertical')
  await kit.getByTestId('scene-redo').click()
  await until(async () => (await camera()).sensorHeight === 21)
  await openSection('scene-settings')
  await setNumber('scene-render-width', 200)
  await until(async () => (await details()).render.width === 200)
  await setNumber('scene-render-height', 320)
  await until(async () => (await details()).render.height === 320)
  await fit.selectOption('horizontal')
  await until(async () => (await camera()).sensorFit === 'horizontal')
  assert.equal((await camera()).sensor, 39)
  await kit.getByTestId('scene-camera').getByText('Orthographic', { exact: true }).click()
  await until(async () => (await camera()).orthographic)
  await setNumber('scene-camera-ortho-scale', 7)
  await until(async () => (await camera()).orthoScale === 7)
  await fit.selectOption('auto')
  await until(async () => (await camera()).sensorFit === 'auto')
  await fit.selectOption('vertical')
  await until(async () => (await camera()).sensorFit === 'vertical')
  await kit.getByTestId('scene-camera').getByText('Perspective', { exact: true }).click()
  await until(async () => !(await camera()).orthographic)
  assert.equal((await camera()).sensorHeight, 21)
  if (process.env.KITION_CAMERA_SCREENSHOT) await page.screenshot({ path: process.env.KITION_CAMERA_SCREENSHOT })
  const before = await camera()
  await kit.getByTestId('scene-save').click()
  await until(async () => (await page.evaluate(() => window.devHost.files.get('Sample.kscene').version)) > 1)
  await page.evaluate(async () => {
    const file = window.devHost.files.get('Sample.kscene')
    await window.devHost.open('Sample.kscene', file.bytes)
  })
  await until(async () => (await camera()).sensorFit === 'vertical')
  assert.deepEqual(await camera(), before)
  assert.equal((await details()).render.width, 200)
  assert.equal((await details()).render.height, 320)
  const sourceBytes = await page.evaluate(() => Array.from(new Uint8Array(window.devHost.files.get('Sample.kscene').bytes)))
  assert(sourceBytes.length > 0, 'the saved document byte snapshot is nonempty')
  await page.evaluate(async () => {
    history.replaceState(null, '', '?role=viewer')
    const file = window.devHost.files.get('Sample.kscene')
    await window.devHost.open('Sample.kscene', file.bytes)
  })
  await openSection('scene-camera')
  assert(await kit.getByTestId('scene-camera-sensor-fit').isDisabled())
  assert(await kit.getByTestId('scene-camera-sensor-height').locator('input').isDisabled())
  assert.deepEqual(await camera(), before)
  const blocked = await page.evaluate(async () => {
    try {
      const result = await window.devHost.connection().ai.run([{ command: 'camera.set', params: { object: 'Camera', sensorFit: 'horizontal', sensorHeight: 35 } }], false)
      return result.error != null || result.failed != null || (result.completed?.length ?? 0) === 0
    } catch { return true }
  })
  assert(blocked, 'the read-only worker rejects camera edits')
  assert.deepEqual(await camera(), before)
  assert.deepEqual(await page.evaluate(() => Array.from(new Uint8Array(window.devHost.files.get('Sample.kscene').bytes))), sourceBytes)
  // Save an orthographic variant, then reopen it read-only.
  await page.evaluate(async () => {
    history.replaceState(null, '', location.pathname)
    const file = window.devHost.files.get('Sample.kscene')
    await window.devHost.open('Sample.kscene', file.bytes)
    await window.devHost.connection().ai.run([{ command: 'camera.set', params: { object: 'Camera', orthographic: true } }], false)
    await window.devHost.connection().save()
    history.replaceState(null, '', '?role=viewer')
    const saved = window.devHost.files.get('Sample.kscene')
    await window.devHost.open('Sample.kscene', saved.bytes)
  })
  await openSection('scene-camera')
  assert(await kit.getByTestId('scene-camera-sensor-fit').isDisabled())
  assert(await kit.getByTestId('scene-camera-ortho-scale').locator('input').isDisabled())
  const orthoBytes = await page.evaluate(() => Array.from(new Uint8Array(window.devHost.files.get('Sample.kscene').bytes)))
  assert(orthoBytes.length > 0, 'the saved document byte snapshot is nonempty')
  await kit.getByTestId('scene-camera-sensor-fit').click({ force: true })
  assert.deepEqual(await page.evaluate(() => Array.from(new Uint8Array(window.devHost.files.get('Sample.kscene').bytes))), orthoBytes)
  assert.equal((await camera()).orthoScale, 7)
  await page.evaluate(async () => {
    history.replaceState(null, '', location.pathname)
    const file = window.devHost.files.get('Sample.kscene')
    await window.devHost.open('Sample.kscene', file.bytes)
    await window.devHost.connection().ai.run([{ command: 'camera.set', params: { object: 'Camera', orthographic: false, sensorFit: 'horizontal' } }], false)
    await window.devHost.connection().save()
    history.replaceState(null, '', '?role=viewer')
    const saved = window.devHost.files.get('Sample.kscene')
    await window.devHost.open('Sample.kscene', saved.bytes)
  })
  await openSection('scene-camera')
  assert(await kit.getByTestId('scene-camera-sensor-width').locator('input').isDisabled())
  assert(await kit.getByTestId('scene-camera-sensor-fit').isDisabled())
  assert.deepEqual(errors, [])
  console.log(`PASS ${which}: camera fit, sensor dimensions, orthographic scale, render aspect, person undo/redo and host save/reopen`)
} finally {
  await browser.close()
  await server.close()
}
