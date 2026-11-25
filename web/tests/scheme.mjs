// The viewport follows the host's color scheme, through the development host:
// node tests/scheme.mjs [chromium|webkit] [software] (software, or KITION_SCENE_GL=off: the software viewport).
// The viewport's background, grid and frame shade are the editor's, not the scene's (as a
// modeller's theme is): opened in the dark scheme the viewport is dark; switched to light and back
// without a reload it repaints each time. The scene's own colours (the cube) are drawn the same in
// both, the document stays clean, a file saved in the dark scheme holds no editor state, and the
// view state the host keeps holds no scheme. Every frame the ready viewport
// shows from the open is checked: none is light. Needs the engine built into public/ (pnpm run engine).
// SHOT=<dir> writes the viewport in both schemes there.
import assert from 'node:assert/strict'
import { inflateRawSync } from 'node:zlib'
import { chromium, webkit } from 'playwright'
import { createServer } from 'vite'

const which = process.argv[2] ?? 'chromium'
const software = process.argv[3] === 'software' || process.env.KITION_SCENE_GL === 'off'
const shots = process.env.SHOT ?? null
const server = await createServer({ configFile: new URL('../vite.config.ts', import.meta.url).pathname, server: { port: 0, host: '127.0.0.1' }, logLevel: 'error' })
await server.listen()
const origin = `http://127.0.0.1:${server.httpServer.address().port}`
// Chromium on the machine's GPU, so the GPU viewport draws (headless Chromium otherwise emulates
// the GPU and the software viewport takes over).
const browser = await (which === 'webkit' ? webkit : chromium).launch(which === 'webkit' || software ? {} : { args: ['--use-angle=metal', '--enable-gpu', '--ignore-gpu-blocklist'] })
const context = await browser.newContext({ viewport: { width: 1280, height: 860 } })
if (software) await context.addInitScript(() => window.localStorage.setItem('kition.scene.gl', 'off'))
// What the viewport shows on every animation frame from the kit's start, while it is ready (before,
// it shows nothing): the GPU viewport's background (the top colour of the last frame it drew, read
// from its uniform) or the software picture's median luminance.
await context.addInitScript(() => {
  if (window === window.top) return
  const seen = []
  window.__sceneFrames = seen
  let top = null
  const names = new WeakMap()
  const locate = WebGL2RenderingContext.prototype.getUniformLocation
  WebGL2RenderingContext.prototype.getUniformLocation = function (program, name) {
    const location = locate.call(this, program, name)
    if (location) names.set(location, name)
    return location
  }
  const uniform = WebGL2RenderingContext.prototype.uniform3fv
  WebGL2RenderingContext.prototype.uniform3fv = function (location, value, ...rest) {
    if (location && names.get(location) === 'uTop') top = Array.from(value)
    return uniform.call(this, location, value, ...rest)
  }
  const linear = (byte) => {
    const value = byte / 255
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4
  }
  const luminance = ([r, g, b]) => 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
  const look = () => {
    const viewport = document.querySelector('[data-testid="scene-viewport"]')
    if (viewport?.dataset.ready === 'true') {
      if (viewport.dataset.gpu === 'on') {
        if (top) seen.push({ gpu: true, luminance: luminance(top.map((channel) => channel * 255)) })
      } else {
        const canvas = viewport.querySelector('[data-testid="scene-canvas"] canvas')
        const context = canvas?.width && canvas.height ? canvas.getContext('2d', { willReadFrequently: true }) : null
        if (context) {
          const { data } = context.getImageData(0, 0, canvas.width, canvas.height)
          const values = []
          for (let at = 0; at < data.length; at += 4 * 61) if (data[at + 3] > 0) values.push(luminance([data[at], data[at + 1], data[at + 2]]))
          values.sort((a, b) => a - b)
          if (values.length) seen.push({ gpu: false, luminance: values[Math.floor(values.length / 2)] })
        }
      }
    }
    requestAnimationFrame(look)
  }
  requestAnimationFrame(look)
})
const page = await context.newPage()
const errors = []
page.on('pageerror', (error) => errors.push(error.message))
const until = async (check, message, ms = 30000) => {
  const end = Date.now() + ms
  let last
  while (Date.now() < end) {
    last = await check().catch((error) => error)
    if (last === true) return
    await page.waitForTimeout(100)
  }
  assert.fail(`${message} (${JSON.stringify(last)})`)
}

const kit = page.frameLocator('#kit')
const viewport = kit.getByTestId('scene-viewport')
const canvas = kit.getByTestId('scene-canvas')
/** The development host's status line: the open file's revision, and `unsaved` when it is dirty. */
const status = () => page.evaluate(() => document.getElementById('status')?.textContent ?? '')

/** Relative luminance (WCAG) of the viewport's pixels: the median, and the share brighter than 0.4. */
async function luminance() {
  const png = (await canvas.screenshot({ animations: 'disabled' })).toString('base64')
  return page.evaluate(async (png) => {
    const bitmap = await createImageBitmap(await (await fetch(`data:image/png;base64,${png}`)).blob())
    const board = new OffscreenCanvas(bitmap.width, bitmap.height)
    const context = board.getContext('2d')
    context.drawImage(bitmap, 0, 0)
    const { data } = context.getImageData(0, 0, bitmap.width, bitmap.height)
    const linear = (byte) => {
      const value = byte / 255
      return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4
    }
    const values = []
    for (let at = 0; at < data.length; at += 16) values.push(0.2126 * linear(data[at]) + 0.7152 * linear(data[at + 1]) + 0.0722 * linear(data[at + 2]))
    values.sort((a, b) => a - b)
    return { median: values[Math.floor(values.length / 2)], light: values.filter((value) => value > 0.4).length / values.length }
  }, png)
}

/** The colour of the cube's top face, from a screenshot (above the grid floor, which shows through
 * the cube's sides near the floor in the scheme's grid colour). */
async function middle() {
  const png = (await canvas.screenshot({ animations: 'disabled' })).toString('base64')
  return page.evaluate(async (png) => {
    const bitmap = await createImageBitmap(await (await fetch(`data:image/png;base64,${png}`)).blob())
    const board = new OffscreenCanvas(bitmap.width, bitmap.height)
    const context = board.getContext('2d')
    context.drawImage(bitmap, 0, 0)
    return [...context.getImageData(Math.round(bitmap.width * 0.48), Math.round(bitmap.height * 0.43), 1, 1).data.slice(0, 3)]
  }, png)
}

const isDark = async () => {
  const { median, light } = await luminance()
  return median <= 0.1 && light <= 0.15 ? true : { median, light }
}
const isLight = async () => {
  const { median } = await luminance()
  return median >= 0.5 ? true : { median }
}
/** An entry of a ZIP file (a `.kscene`), by its central directory. */
function zipEntry(bytes, name) {
  const data = Buffer.from(bytes)
  let end = data.length - 22
  while (end >= 0 && data.readUInt32LE(end) !== 0x06054b50) end -= 1
  assert.ok(end >= 0, 'the file is a ZIP')
  let at = data.readUInt32LE(end + 16)
  for (let index = data.readUInt16LE(end + 10); index > 0; index -= 1) {
    const [method, size, nameLength, extra, comment, local] = [data.readUInt16LE(at + 10), data.readUInt32LE(at + 20), data.readUInt16LE(at + 28), data.readUInt16LE(at + 30), data.readUInt16LE(at + 32), data.readUInt32LE(at + 42)]
    if (data.toString('utf8', at + 46, at + 46 + nameLength) === name) {
      const start = local + 30 + data.readUInt16LE(local + 26) + data.readUInt16LE(local + 28)
      const raw = data.subarray(start, start + size)
      return (method === 8 ? inflateRawSync(raw) : raw).toString('utf8')
    }
    at += 46 + nameLength + extra + comment
  }
  return assert.fail(`the file has no ${name}`)
}
const near = (a, b) => a.every((value, index) => Math.abs(value - b[index]) <= 6)

/** The saved file after the cube is duplicated, in `scheme`: the sample opened anew, the cube copied, saved. */
async function savedWith(scheme, sample) {
  await page.evaluate(async ([scheme, sample]) => {
    window.devHost.setScheme(scheme)
    await window.devHost.open('Sample.kscene', new Uint8Array(sample).buffer)
  }, [scheme, sample])
  await until(async () => (await viewport.getAttribute('data-ready')) === 'true', `the viewport is ready again (${scheme})`)
  await until(async () => (scheme === 'dark' ? isDark() : isLight()), `the reopened viewport is ${scheme}`)
  // The cube in the middle, clicked and duplicated in place (Shift+D, Enter), then saved.
  const box = await viewport.boundingBox()
  await page.mouse.click(box.x + box.width * 0.47, box.y + box.height * 0.53)
  await until(async () => /1 selected/.test(await kit.getByTestId('scene-status').textContent()), `a click selects the cube (${scheme})`)
  await page.keyboard.press('Shift+D')
  await page.keyboard.press('Enter')
  await until(async () => (await kit.locator('[data-testid="scene-outliner-item"][data-name="Cube.001"]').count()) === 1, `Shift+D duplicates the cube (${scheme})`)
  await until(async () => (await status()).includes('unsaved'), `the copy makes the document dirty (${scheme})`)
  return page.evaluate(async () => {
    await window.devHost.connection().save()
    return Array.from(new Uint8Array(window.devHost.files.get('Sample.kscene').bytes))
  })
}

try {
  await page.goto(`${origin}/dev.html?scheme=dark`)
  await page.waitForFunction(() => document.getElementById('status')?.textContent?.includes(' open in '), null, { timeout: 90000 })
  await until(async () => (await viewport.getAttribute('data-ready')) === 'true', 'the viewport is ready')
  assert.equal(await kit.locator('html').getAttribute('data-color-scheme'), 'dark', 'the host marks the editor dark')
  const gpu = await page.frames().find((frame) => frame.url().includes('index.html')).evaluate(() => window.sceneGl?.active === true)
  if (which === 'chromium') assert.equal(gpu, !software, `the ${software ? 'software' : 'GPU'} viewport draws`)
  const sample = await page.evaluate(() => Array.from(new Uint8Array(window.devHost.files.get('Sample.kscene').bytes)))
  const before = await status()
  assert.ok(!before.includes('unsaved'), `the document opens clean: ${before}`)

  await until(isDark, 'opened in the dark scheme, the viewport is dark')
  // From the first frame it shows, the viewport is dark: no light frame before the scheme reaches the engine.
  const shown = await page.frames().find((frame) => frame.url().includes('index.html')).evaluate(() => window.__sceneFrames)
  assert.ok(shown.length > 0, 'the viewport was seen while ready')
  const lit = shown.filter((frame) => frame.luminance > 0.4)
  assert.equal(lit.length, 0, `no frame the viewport shows is light: ${lit.length} of ${shown.length}, first ${JSON.stringify(shown.slice(0, 4))}`)
  const cubeInDark = await middle()
  if (shots) await canvas.screenshot({ path: `${shots}/scene-dark-${which}${software ? '-software' : ''}.png` })

  await page.evaluate(() => window.devHost.setScheme('light'))
  await until(isLight, 'switched live to light, the viewport is light')
  const cubeInLight = await middle()
  if (shots) await canvas.screenshot({ path: `${shots}/scene-light-${which}${software ? '-software' : ''}.png` })
  assert.ok(near(cubeInDark, cubeInLight), `the cube is drawn the same in both schemes: ${cubeInDark} and ${cubeInLight}`)

  await page.evaluate(() => window.devHost.setScheme('dark'))
  await until(isDark, 'and back to dark')

  const after = await status()
  assert.ok(!after.includes('unsaved'), `the scheme leaves the document clean: ${after}`)

  // A file saved in the dark scheme holds no editor state (the view state is the host's, Kit UI
  // protocol 7n), and the view state kept with it holds no scheme.
  const dark = await savedWith('dark', sample)
  assert.ok(zipEntry(dark, 'content.json') && !Buffer.from(dark).includes('editor.json'), 'no editor state in the file')
  const kept = JSON.stringify((await page.evaluate(() => window.devHost.history.viewState('Sample.kscene'))).state)
  assert.ok(!/scheme|dark|light/i.test(kept), `no scheme in the view state: ${kept}`)

  assert.deepEqual(errors, [], 'no page errors')
  console.log(`PASS the viewport follows the host's color scheme (${which}, ${gpu ? 'GPU' : 'software'} viewport)`)
} finally {
  await browser.close()
  await server.close()
}
