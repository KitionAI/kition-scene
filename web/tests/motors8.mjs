// Actual Physics controls, worker bake, person undo/redo and host save/reopen in both browsers.
import assert from 'node:assert/strict'
import { chromium, webkit } from 'playwright'
import { createServer } from 'vite'

const which = process.argv[2] ?? 'chromium'
const server = await createServer({ root: new URL('..', import.meta.url).pathname, configFile: new URL('../vite.config.ts', import.meta.url).pathname, server: { port: 0, host: '127.0.0.1' }, logLevel: 'error' })
await server.listen()
const browser = await (which === 'webkit' ? webkit : chromium).launch()
const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } })
const errors = []
page.on('pageerror', error => errors.push(error.message))
const run = async (command, params = {}) => (await page.evaluate(({ command, params }) => window.devHost.connection().ai.run([{ command, params }], false), { command, params })).completed[0].result
const details = async () => (await page.evaluate(() => window.devHost.connection().ai.inspect())).details
const motor = async () => (await details()).physics.objects.find(object => object.rigidConstraint)?.rigidConstraint
const body = async name => (await details()).objects.find(object => object.name === name)
const until = async check => {
  for (let i = 0; i < 200; i += 1) { if (await check()) return; await page.waitForTimeout(40) }
  throw new Error('Timed out waiting for motor editor state')
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
  const setNumber = async (key, value) => {
    const input = kit.getByTestId(`scene-constraint-${key}`).locator('input')
    await input.fill(String(value))
    await input.press('Enter')
    await until(async () => (await motor())[key] === value)
  }
  await run('select.all')
  await run('edit.delete')
  await run('add.cube', { name: 'Anchor', size: 1, location: [-2, 0, 0] })
  await run('physics.rigidBody.add', { type: 'passive' })
  await run('add.cube', { name: 'Body', size: 1, location: [2, 0, 0] })
  await run('physics.rigidBody.add', { linearDamping: 0, angularDamping: 0 })
  await run('add.empty', { name: 'Motor' })
  await run('physics.world.set', { frameEnd: 25, gravity: [0, 0, 0] })
  await openSection('scene-physics')
  await kit.getByTestId('scene-physics-add-constraint').click()
  await until(async () => await motor())
  await kit.getByTestId('scene-constraint-type').selectOption('motor')
  await until(async () => (await motor()).kind === 'motor')
  for (const [key, name] of [['object1', 'Anchor'], ['object2', 'Body']]) {
    const id = (await body(name)).id
    await kit.getByTestId(`scene-constraint-${key}`).selectOption(String(id))
    await until(async () => (await motor())[key] === id)
  }
  assert.equal((await motor()).linearMotor, false)
  assert.equal((await motor()).angularMotor, false)
  await kit.getByTestId('scene-constraint-linearMotor').click()
  await until(async () => (await motor()).linearMotor)
  await kit.getByTestId('scene-constraint-angularMotor').click()
  await until(async () => (await motor()).angularMotor)
  await setNumber('linearTargetVelocity', -2)
  await setNumber('linearMaxImpulse', 100)
  await setNumber('angularTargetVelocity', -1)
  await setNumber('angularMaxImpulse', 10)
  await kit.locator('.kui-app').focus()
  await page.keyboard.press('Meta+z')
  await until(async () => (await motor()).angularMaxImpulse === 1)
  await kit.getByTestId('scene-redo').click()
  await until(async () => (await motor()).angularMaxImpulse === 10)
  await openSection('scene-physics-world')
  await kit.getByTestId('scene-physics-bake').click()
  await until(async () => (await details()).physics.state === 'baked')
  assert.match(await kit.getByTestId('scene-physics-progress').textContent(), /25/)
  await run('frame.set', { frame: 25 })
  assert(Math.abs((await body('Body')).worldLocation[0] - 4) < 0.03)
  await setNumber('linearTargetVelocity', 2)
  await until(async () => (await details()).physics.state === 'baked' && (await body('Body')).worldLocation[0] < 0.03)
  const saved = await motor()
  await page.evaluate(() => window.devHost.connection().save())
  await page.evaluate(async () => {
    const file = window.devHost.files.get('Sample.kscene')
    await window.devHost.open('Sample.kscene', file.bytes)
  })
  assert.deepEqual(await motor(), saved)
  assert.equal((await details()).physics.state, 'none')
  await openSection('scene-physics')
  await openSection('scene-physics-world')
  await kit.getByTestId('scene-physics-bake').click()
  await until(async () => (await details()).physics.state === 'baked')
  assert((await body('Body')).worldLocation[0] < 0.03)
  if (process.env.KITION_MOTOR_SCREENSHOT) await page.screenshot({ path: process.env.KITION_MOTOR_SCREENSHOT })
  await kit.getByTestId('scene-constraint-linearMotor').click()
  await until(async () => !(await motor()).linearMotor && (await details()).physics.state === 'baked')
  assert(Math.abs((await body('Body')).worldLocation[0] - 2) < 1e-5)
  assert.deepEqual(errors, [])
  await page.evaluate(async () => {
    history.replaceState(null, '', '?role=viewer')
    const file = window.devHost.files.get('Sample.kscene')
    await window.devHost.open('Sample.kscene', file.bytes)
  })
  await openSection('scene-physics')
  assert.deepEqual(await motor(), saved)
  for (const key of ['linearMotor', 'angularMotor']) assert(await kit.getByTestId(`scene-constraint-${key}`).isDisabled())
  for (const key of ['linearTargetVelocity', 'angularTargetVelocity', 'linearMaxImpulse', 'angularMaxImpulse']) {
    assert(await kit.getByTestId(`scene-constraint-${key}`).locator('input').isDisabled())
  }
  assert(await kit.getByTestId('scene-constraint-type').isDisabled())
  assert.deepEqual(errors, [])
  console.log(`PASS ${which}: motor controls, person undo/redo, bake invalidation, toggles, host save/reopen and read-only controls`)
} finally {
  await browser.close()
  await server.close()
}
