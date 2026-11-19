// A click in the viewport before the document is the viewport's size: node tests/early-input.mjs [chromium|webkit]
// A document opens at the size it was saved with, and the editor resizes it to its viewport
// (`view.resize`) a moment later. Until then the engine would pick under the pointer in a picture
// framed for another size, one the person never saw: a click in the middle selected nothing and
// cleared the selection, and the Shift+D that followed duplicated nothing. The viewport says when
// it is ready (`data-ready`), shows no picture and takes no pointer input before, and a click once
// it is ready selects what is drawn under it. The test holds the engine's list of commands, which
// the editor's first `view.resize` waits for (as it did, briefly, when the click came), to keep the
// viewport unready for as long as it needs.
import assert from 'node:assert/strict'
import { chromium, webkit } from 'playwright'
import { createServer } from 'vite'

const which = process.argv[2] ?? 'chromium'
const server = await createServer({ root: new URL('..', import.meta.url).pathname, configFile: new URL('../vite.config.ts', import.meta.url).pathname, server: { port: 0, host: '127.0.0.1' }, logLevel: 'error' })
await server.listen()
const browser = await (which === 'webkit' ? webkit : chromium).launch()
const context = await browser.newContext({ viewport: { width: 1440, height: 900 } })
// In the kit's page: the engine's `commands` requests wait in the page until `__release()`; the
// pointer presses sent to the engine's tools are counted, and the last `view.resize` is kept.
await context.addInitScript(() => {
  const held = []
  let holding = true
  const sent = { down: 0, resize: 0, size: null }
  window.__engineSent = sent
  window.__release = () => {
    holding = false
    for (const send of held.splice(0)) send()
  }
  const post = Worker.prototype.postMessage
  Worker.prototype.postMessage = function (data, ...rest) {
    const request = data?.request
    const send = () => post.call(this, data, ...rest)
    if (request?.method === 'tool' && JSON.stringify(request).includes('\\"action\\":\\"down\\"')) sent.down += 1
    if (request?.method === 'execute' && request.command === 'view.resize') {
      sent.resize += 1
      sent.size = request.params
    }
    if (request?.method === 'commands' && holding) return void held.push(send)
    return send()
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
  const viewport = kit.getByTestId('scene-viewport')
  const status = kit.getByTestId('scene-status')
  await until(async () => /3 objects/.test(await status.textContent()), 'the status bar counts the objects')
  await page.waitForTimeout(400)
  assert.equal(await frame.evaluate(() => window.__engineSent.resize), 0, 'the document is not resized yet')
  assert.equal(await viewport.getAttribute('data-ready'), 'false', 'the viewport is not ready before the document is its size')
  const before = await status.textContent()
  const box = await viewport.boundingBox()
  const middle = { x: box.x + box.width * 0.47, y: box.y + box.height * 0.53 }
  await page.mouse.click(middle.x, middle.y)
  await page.waitForTimeout(400)
  assert.equal(await frame.evaluate(() => window.__engineSent.down), 0, 'a click before the viewport is ready never reaches the engine')
  assert.equal(await status.textContent(), before, 'a click before the viewport is ready changes nothing')

  await frame.evaluate(() => window.__release())
  await until(async () => (await viewport.getAttribute('data-ready')) === 'true', 'the viewport is ready once the document is its size')
  const sized = await frame.evaluate(() => window.__engineSent.size)
  const dpr = await frame.evaluate(() => window.devicePixelRatio)
  assert.ok(Math.abs(sized.width - Math.round(box.width * dpr)) <= 2 && Math.abs(sized.height - Math.round(box.height * dpr)) <= 2, `ready means the document is the viewport (${sized.width}×${sized.height} for ${box.width}×${box.height})`)
  // Ready, the cube drawn in the middle is what a click there selects, and Shift+D duplicates it.
  const cube = kit.locator('[data-testid="scene-outliner-item"][data-name="Cube"]')
  await kit.locator('[data-testid="scene-outliner-item"][data-name="Light"] .scene-outliner-name').click()
  await until(async () => (await cube.getAttribute('data-selected')) === null, 'the light alone is selected')
  await page.mouse.click(middle.x, middle.y)
  await until(async () => (await cube.getAttribute('data-selected')) === 'true' && /3 objects · 1 selected/.test(await status.textContent()), 'a click on the cube selects it')
  await page.keyboard.press('Shift+D')
  await page.keyboard.press('Enter')
  await until(async () => /4 objects/.test(await status.textContent()), 'Shift+D duplicates the cube')
  assert.equal(await kit.locator('[data-testid="scene-outliner-item"][data-name="Cube.001"]').count(), 1, 'the copy is the cube')
  assert.deepEqual(errors, [], 'no page errors')
  console.log(`PASS a click before the viewport is ready (${which})`)
} finally {
  await browser.close()
  await server.close()
}
