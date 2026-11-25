// Real editor creation, settings, socket dragging, scoped undo, and host persistence.
import assert from 'node:assert/strict'
import { chromium, webkit } from 'playwright'
import { createServer } from 'vite'

const which = process.argv[2] ?? 'chromium'
const server = await createServer({ root: new URL('..', import.meta.url).pathname, configFile: new URL('../vite.config.ts', import.meta.url).pathname, server: { port: 0, host: '127.0.0.1' }, logLevel: 'error' })
await server.listen()
const browser = await (which === 'webkit' ? webkit : chromium).launch()
const page = await browser.newPage({ viewport: { width: 1700, height: 1100 } })
const errors = []
page.on('pageerror', error => errors.push(error.message))
const details = async () => (await page.evaluate(() => window.devHost.connection().ai.inspect())).details
const group = async () => (await details()).nodeGroups[0]
const node = async kind => (await group()).nodes.find(node => node.kind === kind)
const vertices = async () => (await details()).objects.find(object => object.name === 'Cube').mesh.evaluated.vertices
const until = async check => {
  for (let i = 0; i < 240; i += 1) { if (await check()) return; await page.waitForTimeout(50) }
  throw new Error('Timed out waiting for spline node state')
}
try {
  await page.goto(`http://127.0.0.1:${server.httpServer.address().port}/dev.html`)
  await page.waitForFunction(() => document.getElementById('status')?.textContent?.includes(' open in '), null, { timeout: 60000 })
  const kit = page.frameLocator('#kit')
  await kit.locator('[data-testid="scene-outliner-item"][data-name="Cube"] .scene-outliner-name').click()
  await kit.getByTestId('scene-nodes-toggle').click()
  await kit.getByTestId('scene-nodes-new').click()
  await until(async () => (await details()).nodeGroups.length === 1)
  const add = async (kind, search) => {
    await kit.getByTestId('scene-nodes-add').click()
    await kit.getByTestId('scene-nodes-search').fill(search)
    await kit.getByTestId(`scene-nodes-add-${kind}`).click()
    await until(async () => Boolean(await node(kind)))
    return (await node(kind)).id
  }
  const source = await add('curveBezierSegment', 'Segment')
  const kind = await add('setSplineType', 'Set Spline Type')
  const order = await add('setNurbsOrder', 'Set NURBS Order')
  const weight = await add('setNurbsWeight', 'Set NURBS Weight')
  const output = (await node('groupOutput')).id
  // Position the nodes through the same public command the editor's drag uses, then connect
  // the visible sockets with actual pointer drags.
  await page.evaluate(async ids => {
    const connection = window.devHost.connection()
    const group = (await connection.ai.inspect()).details.nodeGroups[0]
    const ops = ids.map((id, index) => {
      const node = group.nodes.find(node => node.id === id)
      return { command: 'node.moveNodes', params: { nodes: [id], offset: [index * 230 - node.location[0], 40 - node.location[1]] } }
    })
    await connection.ai.run(ops, false)
  }, [source, kind, order, weight, output])
  await kit.getByTestId('scene-nodes-frame').click()
  const connect = async (from, fromSocket, to, toSocket) => {
    const a = await kit.getByTestId(`scene-socket-${from}-out-${fromSocket}`).boundingBox()
    const b = await kit.getByTestId(`scene-socket-${to}-in-${toSocket}`).boundingBox()
    assert(a && b, 'both sockets are visible')
    const start = [a.x + a.width / 2, a.y + a.height / 2]
    const end = [b.x + b.width / 2, b.y + b.height / 2]
    await page.mouse.move(...start)
    await page.mouse.down()
    await page.mouse.move(...end, { steps: 12 })
    await page.mouse.up()
    await until(async () => (await group()).links.some(link => link.from === from && link.to === to))
  }
  await connect(source, 'Curve', kind, 'Curve')
  await connect(kind, 'Curve', order, 'Curves')
  await connect(order, 'Curves', weight, 'Curves')
  await connect(weight, 'Curves', output, 'Geometry')
  const number = async (id, name, value) => {
    const field = kit.getByTestId(new RegExp(`^scene-node-${id}-${name}-`))
    await field.fill(String(value))
    await field.press('Enter')
  }
  await number(source, 'Resolution', 4)
  const select = kit.getByTestId(`scene-node-${kind}-splineType`)
  await select.selectOption('catmullRom')
  await until(async () => (await vertices()) === 5)
  await number(order, 'Order', 3)
  await number(weight, 'Weight', 0.5)
  await until(async () => (await node('setNurbsWeight')).values.Weight === 0.5)
  await select.selectOption('nurbs')
  await until(async () => (await vertices()) === 9)
  await kit.getByTestId('scene-undo').click()
  await until(async () => (await node('setSplineType')).values.splineType === 'catmullRom')
  assert.equal(await vertices(), 5)
  await kit.getByTestId('scene-redo').click()
  await until(async () => (await node('setSplineType')).values.splineType === 'nurbs')
  assert.equal(await vertices(), 9)
  if (process.env.KITION_CURVES_SCREENSHOT) await page.screenshot({ path: process.env.KITION_CURVES_SCREENSHOT })
  const saved = await group()
  await kit.getByTestId('scene-save').click()
  await until(async () => (await page.evaluate(() => window.devHost.files.get('Sample.kscene').version)) > 1)
  await page.evaluate(async () => {
    const file = window.devHost.files.get('Sample.kscene')
    await window.devHost.open('Sample.kscene', file.bytes)
  })
  await until(async () => (await vertices()) === 9)
  assert.deepEqual(await group(), saved)
  const bytes = await page.evaluate(() => Array.from(new Uint8Array(window.devHost.files.get('Sample.kscene').bytes)))
  assert(bytes.length > 0)
  await page.evaluate(async () => {
    history.replaceState(null, '', '?role=viewer')
    const file = window.devHost.files.get('Sample.kscene')
    await window.devHost.open('Sample.kscene', file.bytes)
  })
  await until(async () => (await vertices()) === 9)
  if (!(await kit.getByTestId('scene-nodes').isVisible())) await kit.getByTestId('scene-nodes-toggle').click()
  assert(await kit.getByTestId(`scene-node-${kind}-splineType`).isDisabled())
  assert(await kit.getByTestId(new RegExp(`^scene-node-${order}-Order-`)).isDisabled())
  assert(await kit.getByTestId(new RegExp(`^scene-node-${weight}-Weight-`)).isDisabled())
  const blocked = await page.evaluate(async kind => {
    try {
      const result = await window.devHost.connection().ai.run([{ command: 'node.setValue', params: { node: kind, id: 'splineType', value: 'poly' } }], false)
      return result.error != null || result.failed != null || (result.completed?.length ?? 0) === 0
    } catch { return true }
  }, kind)
  assert(blocked, 'read-only worker rejects spline edits')
  assert.deepEqual(await group(), saved)
  assert.deepEqual(await page.evaluate(() => Array.from(new Uint8Array(window.devHost.files.get('Sample.kscene').bytes))), bytes)
  assert.deepEqual(errors, [])
  console.log(`Spline types ${which}: passed`)
} finally {
  await browser.close()
  await server.close()
}
