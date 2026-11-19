// Cmd+S right after deleting an object from the outliner: node tests/delete-focus.mjs [chromium|webkit]
// An outliner row keeps the keyboard after a click. X there deletes the object, and the row goes
// with it; the editor must have the keyboard by then, or the focus falls to the page and the Cmd+S
// that follows reaches nothing: the deletion stays unsaved. When the engine's tools were busy with a
// pointer event (the mouse moving over the viewport), they held the X and ran it after, and the
// keyboard stayed on the row. The test slows the tools' pointer events so they are busy every time.
import assert from 'node:assert/strict'
import { chromium, webkit } from 'playwright'
import { createServer } from 'vite'

const which = process.argv[2] ?? 'chromium'
const server = await createServer({ root: new URL('..', import.meta.url).pathname, configFile: new URL('../vite.config.ts', import.meta.url).pathname, server: { port: 0, host: '127.0.0.1' }, logLevel: 'error' })
await server.listen()
const browser = await (which === 'webkit' ? webkit : chromium).launch()
const context = await browser.newContext({ viewport: { width: 1440, height: 900 } })
// In the kit's page: each pointer event for the engine's tools reaches the engine 300 ms late.
await context.addInitScript(() => {
  const post = Worker.prototype.postMessage
  Worker.prototype.postMessage = function (data, ...rest) {
    const request = data?.request
    if (request?.method === 'tool' && JSON.stringify(request).includes('pointer')) return void setTimeout(() => post.call(this, data, ...rest), 300)
    return post.call(this, data, ...rest)
  }
})
const page = await context.newPage()
const errors = []
page.on('pageerror', (error) => errors.push(error.message))
const until = async (check, message) => {
  const end = Date.now() + 30000
  while (Date.now() < end) {
    if (await check().catch(() => false)) return
    await page.waitForTimeout(50)
  }
  assert.fail(message)
}
try {
  await page.goto(`http://127.0.0.1:${server.httpServer.address().port}/dev.html`)
  await page.waitForFunction(() => document.getElementById('status')?.textContent?.includes(' open in '), null, { timeout: 60000 })
  const kit = page.frameLocator('#kit')
  const frame = page.frames().find((item) => item.url().includes('index.html'))
  const status = kit.getByTestId('scene-status')
  await until(async () => /3 objects/.test(await status.textContent()), 'the status bar counts the objects')
  await until(async () => (await kit.locator('.kui-tool-rail button').count()) > 0, "the engine's tools are on the rail")
  const row = kit.locator('[data-testid="scene-outliner-item"][data-name="Cube"]')
  await row.locator('.scene-outliner-name').click()
  await until(async () => (await row.getAttribute('data-selected')) === 'true', 'the outliner selects the cube')
  const viewport = await kit.getByTestId('scene-viewport').boundingBox()
  // The pointer moves over the viewport (a pointer event for the tools), and X comes at once.
  await page.mouse.move(viewport.x + viewport.width / 2, viewport.y + viewport.height / 2, { steps: 2 })
  await page.keyboard.press('x')
  await until(async () => /2 objects/.test(await status.textContent()), 'X deletes the cube')
  assert.equal(await row.count(), 0, "the cube's row is gone")
  const focused = await frame.evaluate(() => (document.activeElement === document.body ? 'body' : (document.activeElement?.className ?? 'none')))
  assert.ok(String(focused).includes('kui-app'), `the editor keeps the keyboard after the row goes (focus on ${focused})`)
  await kit.getByTestId('scene-unsaved').waitFor()
  await page.keyboard.press('ControlOrMeta+s')
  await until(async () => (await page.evaluate(() => window.devHost.files.get('Sample.kscene').version)) > 1, 'Cmd+S saves the deletion')
  await until(async () => (await kit.getByTestId('scene-unsaved').count()) === 0, 'nothing is left unsaved')
  assert.deepEqual(errors, [], 'no page errors')
  console.log(`PASS Cmd+S after deleting from the outliner (${which})`)
} finally {
  await browser.close()
  await server.close()
}
