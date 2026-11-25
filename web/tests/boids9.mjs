// Real particle controls, worker simulation, person history, and host save/reopen.
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
const run = async (command, params = {}) => (await page.evaluate(({ command, params }) => window.devHost.connection().ai.run([{ command, params }], false), { command, params })).completed[0].result
const inspect = async () => (await page.evaluate(() => window.devHost.connection().ai.inspect())).details
const settings = async () => (await inspect()).particles[0]
const until = async check => { for (let i = 0; i < 200; i += 1) { if (await check()) return; await page.waitForTimeout(40) } throw new Error('Timed out waiting for boid controls') }
try {
  await page.goto(`http://127.0.0.1:${server.httpServer.address().port}/dev.html`)
  await page.waitForFunction(() => document.getElementById('status')?.textContent?.includes(' open in '), null, { timeout: 60000 })
  const kit = page.frameLocator('#kit')
  const open = async () => { const section = kit.getByTestId('scene-particles'); await section.waitFor(); if ((await section.getAttribute('data-open')) !== 'true') await section.locator('.kui-panel-section-toggle').first().click() }
  const number = async (id, value, check) => { const input = kit.getByTestId(id).locator('input'); await input.fill(String(value)); await input.press('Enter'); await until(check) }
  await run('object.addEmpty', { name: 'Goal', location: [8, 0, 8] })
  const goal = (await inspect()).objects.find(object => object.name === 'Goal').id
  await run('select.objects', { objects: 'Cube' })
  await open()
  await kit.getByTestId('scene-add-particles').click()
  await kit.getByTestId('scene-add-particles-emitter').click()
  await until(async () => Boolean(await settings()))
  await number('scene-particles-count', 8, async () => (await settings()).count === 8)
  await number('scene-particles-frameEnd', 1, async () => (await settings()).frameEnd === 1)
  await kit.getByTestId('scene-particles-physics').selectOption('boids')
  await until(async () => (await settings()).physics === 'boids')
  assert.equal(await kit.getByTestId('scene-particles-normalVelocity').count(), 0)
  await kit.getByTestId('scene-boid-kind-0').selectOption('goal')
  await until(async () => (await settings()).boids.rules[0].kind === 'goal')
  await kit.getByTestId('scene-boid-target-0').selectOption(String(goal))
  await until(async () => (await settings()).boids.rules[0].target === goal)
  await kit.getByTestId('scene-boid-kind-0').selectOption('avoid')
  await until(async () => (await settings()).boids.rules[0].kind === 'avoid')
  assert.equal((await settings()).boids.rules[0].target, goal, 'Goal to Avoid retains the selected target')
  await kit.getByTestId('scene-undo').click()
  await until(async () => (await settings()).boids.rules[0].kind === 'goal')
  assert.equal((await settings()).boids.rules[0].target, goal, 'Undo restores Goal with its target')
  await kit.getByTestId('scene-boid-remove-1').click()
  await until(async () => (await settings()).boids.rules.length === 1)
  await number('scene-boids-airSpeedMax', 6, async () => (await settings()).boids.airSpeedMax === 6)
  await kit.locator('.kui-app').focus()
  await page.keyboard.press('Meta+z')
  await until(async () => (await settings()).boids.airSpeedMax === 10)
  await kit.getByTestId('scene-redo').click()
  await until(async () => (await settings()).boids.airSpeedMax === 6)
  await kit.getByTestId('scene-boid-add').click()
  await until(async () => (await settings()).boids.rules.length === 2)
  await kit.getByTestId('scene-boid-kind-1').selectOption('separate')
  await until(async () => (await settings()).boids.rules[1].kind === 'separate')
  await run('frame.set', { frame: 31 })
  const at31 = await run('particle.stats', { object: 'Cube' })
  assert.equal(at31.alive, 8)
  assert(at31.meanPosition[0] > 1 && at31.meanPosition[2] > 1)
  const saved = await settings()
  await page.evaluate(() => window.devHost.connection().save())
  await page.evaluate(async () => { const file = window.devHost.files.get('Sample.kscene'); await window.devHost.open('Sample.kscene', file.bytes) })
  assert.deepEqual(await settings(), saved)
  assert.deepEqual(await run('particle.stats', { object: 'Cube', frame: 31 }), at31)
  await open()
  if (process.env.KITION_BOIDS_SCREENSHOT) {
    await kit.getByTestId('scene-boids').scrollIntoViewIfNeeded()
    await page.screenshot({ path: process.env.KITION_BOIDS_SCREENSHOT })
  }
  await kit.getByTestId('scene-particles-physics').selectOption('newtonian')
  await until(async () => (await settings()).physics === 'newtonian')
  assert.equal((await settings()).normalVelocity, saved.normalVelocity)
  assert(await kit.getByTestId('scene-particles-normalVelocity').isVisible())
  await kit.getByTestId('scene-particles-physics').selectOption('boids')
  await until(async () => (await settings()).physics === 'boids')
  assert.deepEqual((await settings()).boids, saved.boids)
  await page.evaluate(async () => { history.replaceState(null, '', '?role=viewer'); const file = window.devHost.files.get('Sample.kscene'); await window.devHost.open('Sample.kscene', file.bytes) })
  await open()
  assert(await kit.getByTestId('scene-particles-physics').isDisabled())
  assert(await kit.getByTestId('scene-boid-kind-0').isDisabled())
  assert(await kit.getByTestId('scene-boid-target-0').isDisabled())
  assert(await kit.getByTestId('scene-boid-add').isDisabled())
  assert(await kit.getByTestId('scene-boids-airSpeedMax').locator('input').isDisabled())
  assert.deepEqual(errors, [])
  console.log(`PASS ${which}: boid controls, airborne motion, person history, physics switching, host save/reopen and view-only controls`)
} finally { await browser.close(); await server.close() }
