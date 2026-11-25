// The editor's display modes, past the mode smoke (tests/modes.mjs): node tests/focus.mjs [chromium|webkit]
// Single line opens on Add, not File; with the Graph, NLA, and Node editors open, Focus shows the
// viewport alone (no timeline, no editor below it) and its bar covers none of the kit's
// controls; Escape leaves Focus and the editors come back.
import assert from 'node:assert/strict'
import { chromium, webkit } from 'playwright'
import { createServer } from 'vite'
import { focusBarReport, lineRowReport } from '@kition/sdk/dev-host/line-row'

const which = process.argv[2] ?? 'chromium'
const server = await createServer({ configFile: new URL('../vite.config.ts', import.meta.url).pathname, server: { port: 0, host: '127.0.0.1' }, logLevel: 'error' })
await server.listen()
const browser = await (which === 'webkit' ? webkit : chromium).launch()
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } })
// The editor's shortcuts take Cmd where the page says it runs on a Mac (kit-editor reads
// navigator.platform), else Ctrl. Playwright's ControlOrMeta follows the machine running the test
// instead, and its WebKit says Mac on Linux too, so the test asks the page.
const mod = (await page.evaluate(() => /Mac|iPhone|iPad/i.test(navigator.platform))) ? 'Meta' : 'Control'
const errors = []
page.on('pageerror', (error) => errors.push(error.message))
const until = async (check, message) => {
  const end = Date.now() + 30000
  while (Date.now() < end) {
    if (await check().catch(() => false)) return
    await page.waitForTimeout(100)
  }
  assert.fail(message)
}
try {
  await page.goto(`http://127.0.0.1:${server.httpServer.address().port}/dev.html?modes=line,menus,classic,focus&locale=en-US`)
  await page.waitForFunction(() => document.getElementById('status')?.textContent?.includes(' open in '), null, { timeout: 60000 })
  const kit = page.frameLocator('#kit')
  const app = kit.locator('.kui-app')
  // The frame draws its tabs once the engine's commands are listed.
  await until(async () => (await app.getAttribute('data-mode')) === 'line', 'Single line shows')
  const frame = page.frames().find((item) => item.url().includes('index.html'))
  assert.equal(await kit.locator('.kui-ribbon-tab[aria-selected="true"]').getAttribute('data-tab'), 'Add', 'Single line opens on Add')
  const row = await frame.evaluate(lineRowReport, ['scene-cmd-object.addCube', 'scene-cmd-object.addUvSphere', 'scene-cmd-object.addLight', 'scene-cmd-object.addCamera'])
  assert.deepEqual(row.missing, [], `Add's row shows its primary commands (${row.buttons} buttons)`)

  for (const id of ['scene-graph-toggle', 'scene-nla-toggle', 'scene-nodes-toggle']) await kit.getByTestId(id).check()
  for (const id of ['scene-timeline', 'scene-graph', 'scene-nla', 'scene-nodes']) await until(async () => (await kit.getByTestId(id).count()) === 1, `${id} shows`)

  await app.focus()
  await page.keyboard.press(`${mod}+Backslash`)
  await until(async () => (await app.getAttribute('data-mode')) === 'focus', 'Cmd+\\ enters Focus')
  for (const id of ['scene-timeline', 'scene-graph', 'scene-nla', 'scene-nodes']) assert.equal(await kit.getByTestId(id).count(), 0, `Focus hides ${id}`)
  assert.ok((await kit.locator('.scene-stage canvas').count()) > 0, 'Focus shows the viewport')
  assert.deepEqual(await frame.evaluate(focusBarReport), [], 'the Focus bar covers none of the kit controls')

  await page.keyboard.press('Escape')
  await until(async () => (await app.getAttribute('data-mode')) === 'line', 'Escape leaves Focus')
  for (const id of ['scene-timeline', 'scene-graph', 'scene-nla', 'scene-nodes']) assert.equal(await kit.getByTestId(id).count(), 1, `leaving Focus shows ${id} again`)
  assert.deepEqual(errors, [], 'no page errors')
  console.log(`PASS scene display modes (${which})`)
} finally {
  await browser.close()
  await server.close()
}
