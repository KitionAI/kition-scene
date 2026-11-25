// Worker-driven rendered shading, exercised through the same UI and bridge as a host.
import assert from 'node:assert/strict'
import { chromium, webkit } from 'playwright'
import { createServer } from 'vite'
const which = process.argv[2] ?? 'chromium'
const root = new URL('../', import.meta.url).pathname
const server = await createServer({ root, configFile: new URL('../vite.config.ts', import.meta.url).pathname, server: { port: 0, host: '127.0.0.1' }, logLevel: 'error' })
await server.listen()
const browser = await (which === 'webkit' ? webkit : chromium).launch(which === 'webkit' ? {} : { args: ['--use-angle=metal', '--enable-gpu', '--ignore-gpu-blocklist'] })
const page = await browser.newPage({ viewport: { width: 1000, height: 700 } })
const errors = []
page.on('pageerror', (error) => errors.push(error.message))
const until = async (check, message) => {
  const end = Date.now() + 60000
  while (Date.now() < end) {
    if (await check()) return
    await page.waitForTimeout(100)
  }
  assert.fail(message)
}
try {
  await page.goto(`http://127.0.0.1:${server.httpServer.address().port}/dev.html`)
  await page.waitForFunction(() => document.getElementById('status')?.textContent?.includes(' open in '), null, { timeout: 60000 })
  const kit = page.frameLocator('#kit')
  const frame = page.frames().find((item) => item.url().includes('index.html'))
  await frame.evaluate(() => {
    window.viewport7Alerts = []
    new MutationObserver(() => {
      for (const alert of document.querySelectorAll('[role="alert"]')) window.viewport7Alerts.push(alert.textContent)
    }).observe(document.body, { childList: true, subtree: true })
  })
  const inspect = async () => (await page.evaluate(() => window.devHost.connection().ai.inspect())).details
  const run = (command, params = {}) => page.evaluate(({ command, params }) => window.devHost.connection().ai.run([{ command, params }], false), { command, params })
  const sample = () => frame.evaluate(() => {
    const canvas = document.querySelector('canvas.kui-canvas-surface')
    const data = canvas.getContext('2d').getImageData(0, 0, canvas.width, canvas.height).data
    let digest = 0
    for (let i = 0; i < data.length; i += 97) digest = (digest * 31 + data[i]) >>> 0
    return digest
  })
  await run('view.grid', { on: false })
  const dirty = (await inspect()).canUndo
  await kit.getByRole('button', { name: 'Rendered', exact: true }).click()
  const progress = kit.getByTestId('scene-rendered-progress')
  await progress.waitFor()
  await until(async () => Number(await progress.getAttribute('data-samples')) >= 1, 'rendered shading did not progress')
  assert.equal((await inspect()).view.shading, 'rendered')
  assert.equal((await inspect()).canUndo, dirty)
  assert.notEqual(await kit.getByTestId('scene-viewport').getAttribute('data-gpu'), 'on')
  const firstGeneration = Number(await progress.getAttribute('data-generation'))
  await until(async () => Number(await progress.getAttribute('data-samples')) >= 2, 'second sample did not arrive')
  assert.equal(Number(await progress.getAttribute('data-generation')), firstGeneration, 'sampling rebuilt the scene')
  const first = await sample()
  // Navigation is a normal pointer gesture while sampling, followed by a settled new scene.
  const box = await kit.getByTestId('scene-viewport').boundingBox()
  await page.mouse.move(box.x + box.width * 0.5, box.y + box.height * 0.5)
  await page.mouse.down({ button: 'middle' })
  await page.mouse.move(box.x + box.width * 0.6, box.y + box.height * 0.55, { steps: 5 })
  await page.mouse.up({ button: 'middle' })
  await until(async () => Number(await progress.getAttribute('data-generation')) > firstGeneration && Number(await progress.getAttribute('data-samples')) >= 1, 'orbit did not invalidate the render')
  assert.notEqual(await sample(), first)
  const generation = Number(await progress.getAttribute('data-generation'))
  await run('object.transform', { objects: 'Cube', location: [1, 0, 0] })
  await until(async () => Number(await progress.getAttribute('data-generation')) > generation && Number(await progress.getAttribute('data-samples')) >= 1, 'an edit kept old samples')
  await run('edit.undo')
  await run('view.front')
  await run('edit.deselectAll')
  const selection = await kit.getByTestId('scene-viewport').boundingBox()
  await page.mouse.click(selection.x + selection.width / 2, selection.y + selection.height / 2)
  await until(async () => (await inspect()).selection.objects.length === 1, 'rendered viewport did not pick the cube')
  const beforeCamera = Number(await progress.getAttribute('data-generation'))
  await kit.getByTestId('scene-nav-view.camera').click()
  await until(async () => (await inspect()).view.camera != null && Number(await progress.getAttribute('data-generation')) > beforeCamera && Number(await progress.getAttribute('data-samples')) >= 1, 'camera render did not progress')
  const beforeFrame = Number(await progress.getAttribute('data-generation'))
  await run('frame.set', { frame: 2 })
  await until(async () => Number(await progress.getAttribute('data-generation')) > beforeFrame && Number(await progress.getAttribute('data-samples')) >= 1, 'frame change stalled rendering')
  await kit.getByRole('button', { name: 'Solid', exact: true }).click()
  await until(async () => await progress.count() === 0, 'switching to solid left the tracer running')
  assert.equal((await inspect()).view.shading, 'solid')
  await page.waitForTimeout(300)
  assert.equal((await inspect()).view.shading, 'solid', 'a stale completion restored rendered mode')
  await kit.getByRole('button', { name: 'Rendered', exact: true }).click()
  await until(async () => Number(await progress.getAttribute('data-samples')) >= 1, 'rendered mode failed to resume')
  // Repeated handoffs exercise the window between an engine mode change and React cleanup.
  for (let attempt = 0; attempt < 4; attempt += 1) {
    await kit.getByRole('button', { name: 'Solid', exact: true }).click()
    await until(async () => await progress.count() === 0, 'solid handoff stalled')
    await kit.getByRole('button', { name: 'Rendered', exact: true }).click()
    await until(async () => Number(await progress.getAttribute('data-samples')) >= 1, 'rendered handoff stalled')
  }
  if (process.env.KITION_SCENE_VIEWPORT_SNAPSHOT) await page.screenshot({ path: process.env.KITION_SCENE_VIEWPORT_SNAPSHOT })
  assert.equal(await kit.getByRole('alert').count(), 0, 'normal rendered navigation must not report a stale-step error')
  assert.deepEqual(await frame.evaluate(() => window.viewport7Alerts), [], 'no transient error should be shown during a mode handoff')
  assert.deepEqual(errors, [])
  console.log(`PASS ${which}: rendered samples reuse preparation; navigation, editing, picking, camera/frame changes, and cancellation work`)
} finally {
  await browser.close()
  await server.close()
}
