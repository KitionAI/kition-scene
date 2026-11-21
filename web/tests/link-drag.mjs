// Links in the node editor by two drags in a row, through the development host:
// node tests/link-drag.mjs [chromium|webkit]
// A compositing group's Render Layers is linked to a new Exposure node and the Exposure node to the
// output, each by a drag from socket to socket; neither drag leaves a text selection on the page.
// Before, WebKit (which honours only the prefixed -webkit-user-select) selected the node texts
// under the first drag, and on Linux the second drag, which started inside that selection, dragged
// the selection instead of a link, so the Exposure node was never linked to the output.
import { chromium, webkit } from 'playwright'
import { createServer } from 'vite'

const which = process.argv[2] ?? 'chromium'
const server = await createServer({ configFile: new URL('../vite.config.ts', import.meta.url).pathname, server: { port: 0, host: '127.0.0.1' }, logLevel: 'error' })
await server.listen()
const origin = `http://127.0.0.1:${server.httpServer.address().port}`
const browser = await (which === 'webkit' ? webkit : chromium).launch()
const page = await (await browser.newContext({ viewport: { width: 1440, height: 900 } })).newPage()
const errors = []
let failures = 0
page.on('pageerror', (error) => errors.push(error.message))
const step = (name, ok, detail = '') => {
  if (!ok) failures += 1
  console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ` - ${detail}` : ''}`)
}
const until = async (check, tries = 80) => {
  for (let i = 0; i < tries; i += 1) {
    if (await check().catch(() => false)) return true
    await page.waitForTimeout(100)
  }
  return false
}
const details = async () => (await page.evaluate(async () => window.devHost.connection().ai.inspect())).details

try {
  await page.goto(`${origin}/dev.html`)
  await page.waitForFunction(() => document.getElementById('status')?.textContent?.includes(' open in '), null, { timeout: 60000 })
  const kit = page.frameLocator('#kit')
  const frame = page.frames().find((item) => item.url().includes('index.html'))
  const selection = () => frame.evaluate(() => String(window.getSelection() ?? ''))
  await kit.getByTestId('scene-nodes-toggle').click()
  await kit.getByTestId('scene-nodes').waitFor()
  await kit.getByTestId('scene-nodes-new-compositor').click()
  await until(async () => (await details()).render?.compositor != null)
  const compositor = async () => {
    const now = await details()
    return now.nodeGroups?.find((group) => group.id === now.render?.compositor)
  }
  await kit.getByTestId('scene-nodes-add').click()
  await kit.getByTestId('scene-nodes-search').fill('expos')
  await until(async () => (await kit.getByTestId('scene-nodes-add-exposure').count()) === 1)
  await kit.getByTestId('scene-nodes-search').press('Enter')
  await until(async () => (await compositor())?.nodes?.some((node) => node.kind === 'exposure'))
  const nodes = (await compositor()).nodes
  const exposure = nodes.find((node) => node.kind === 'exposure')
  const layers = nodes.find((node) => node.kind === 'renderLayers')
  const output = nodes.find((node) => node.kind === 'compositorOutput')
  // Move the Exposure node clear of the others by its title, then frame them all.
  const title = await kit.locator(`[data-testid="scene-node-${exposure.id}"] .scene-node-header`).boundingBox()
  await page.mouse.move(title.x + 20, title.y + 8)
  await page.mouse.down()
  for (let i = 1; i <= 6; i += 1) await page.mouse.move(title.x + 20, title.y + 8 + i * 25)
  await page.mouse.up()
  await until(async () => JSON.stringify((await compositor())?.nodes?.find((node) => node.id === exposure.id)?.location) !== JSON.stringify(exposure.location))
  step('dragging a title selects no text', (await selection()) === '', JSON.stringify(await selection()))
  await kit.getByTestId('scene-nodes-frame').click()
  const sockets = [`scene-socket-${layers.id}-out-Image`, `scene-socket-${exposure.id}-in-Image`, `scene-socket-${exposure.id}-out-Image`, `scene-socket-${output.id}-in-Image`]
  const centre = async (id) => {
    const box = await kit.getByTestId(id).boundingBox()
    return [box.x + box.width / 2, box.y + box.height / 2]
  }
  const settled = () =>
    until(async () => {
      const before = JSON.stringify(await Promise.all(sockets.map(centre)))
      await page.waitForTimeout(100)
      return before === JSON.stringify(await Promise.all(sockets.map(centre)))
    })
  const dragLink = async (from, to) => {
    const [ax, ay] = await centre(from)
    const [bx, by] = await centre(to)
    await page.mouse.move(ax, ay)
    await page.mouse.down()
    for (let i = 1; i <= 10; i += 1) await page.mouse.move(ax + ((bx - ax) * i) / 10, ay + ((by - ay) * i) / 10)
    await page.mouse.up()
  }
  const linked = (from, to) => async () => ((await compositor())?.links ?? []).some((link) => link.from === from && link.to === to)
  await settled()
  await dragLink(sockets[0], sockets[1])
  step('a drag links Render Layers to Exposure', await until(linked(layers.id, exposure.id)), JSON.stringify((await compositor())?.links))
  step('the drag selects no text', (await selection()) === '', JSON.stringify(await selection()))
  await settled()
  await dragLink(sockets[2], sockets[3])
  step('the next drag links Exposure to the output', await until(linked(exposure.id, output.id)), JSON.stringify((await compositor())?.links))
  step('the next drag selects no text', (await selection()) === '', JSON.stringify(await selection()))
  step('no page errors', errors.length === 0, errors.join(' | '))
} catch (error) {
  failures += 1
  console.log(`FAIL ${error instanceof Error ? error.message : String(error)}`)
} finally {
  await browser.close()
  await server.close()
}
console.log(failures ? `${failures} step(s) failed` : 'all steps passed')
process.exit(failures ? 1 : 0)
