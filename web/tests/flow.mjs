// The editor's flow in a real browser, through the development host: node tests/flow.mjs [chromium|webkit]
// Starts the Vite dev server itself, opens dev.html, and prints PASS or FAIL per step.
// KITION_SCENE_SNAPSHOT=<path.png> also writes a screenshot of the editor (1440 × 900),
// KITION_SCENE_TIMELINE_SNAPSHOT one with the cube keyed and the timeline on frame 10, and
// KITION_SCENE_GL_SNAPSHOTS=<directory> one of the viewport per GPU state compared.
import { chromium, webkit } from 'playwright'
import { createServer } from 'vite'

const which = process.argv[2] ?? 'chromium'
const server = await createServer({ configFile: new URL('../vite.config.ts', import.meta.url).pathname, server: { port: 0, host: '127.0.0.1' }, logLevel: 'error' })
await server.listen()
const origin = `http://127.0.0.1:${server.httpServer.address().port}`
// Chromium on the machine's GPU (headless Chromium otherwise emulates it on the CPU, where the GPU
// viewport steps aside for the software one).
const browser = await (which === 'webkit' ? webkit : chromium).launch(which === 'webkit' ? {} : { args: ['--use-angle=metal', '--enable-gpu', '--ignore-gpu-blocklist'] })
const context = await browser.newContext({ viewport: { width: 1440, height: 900 } })
// KITION_SCENE_GL=off runs the whole flow on the software viewport (the GPU viewport's fallback).
let gpu = process.env.KITION_SCENE_GL !== 'off'
if (!gpu) await context.addInitScript(() => window.localStorage.setItem('kition.scene.gl', 'off'))
const page = await context.newPage()
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
const object = async (name) => (await details()).objects.find((item) => item.name === name)

try {
  await page.goto(`${origin}/dev.html`)
  await page.waitForFunction(() => document.getElementById('status')?.textContent?.includes(' open in '), null, { timeout: 60000 })
  const kit = page.frameLocator('#kit')
  const frame = page.frames().find((item) => item.url().includes('index.html'))
  // The viewport's picture, sampled from its canvas: [r, g, b] at fractions of its size, and a digest.
  const sample = () =>
    frame.evaluate(() => {
      const canvas = document.querySelector('[data-testid="scene-viewport"] canvas.kui-canvas-surface')
      const context = canvas.getContext('2d')
      const at = (fx, fy) => Array.from(context.getImageData(Math.floor(canvas.width * fx), Math.floor(canvas.height * fy), 1, 1).data).slice(0, 3)
      const all = context.getImageData(0, 0, canvas.width, canvas.height).data
      let digest = 0
      for (let index = 0; index < all.length; index += 97) digest = (digest * 31 + all[index]) >>> 0
      return { centre: at(0.47, 0.53), corner: at(0.03, 0.05), digest }
    })
  const painted = await until(async () => {
    const { centre } = await sample()
    return centre[0] < 200 && centre[0] > 40
  }, 150)
  step('the viewport paints the cube in its centre', painted, JSON.stringify((await sample()).centre))
  step('the status bar counts the objects', /3 objects/.test(await kit.getByTestId('scene-status').textContent()))
  await kit.getByTestId('scene-menu-File').click()
  await kit.getByTestId('scene-cmd-file.export').click()
  await kit.getByTestId('scene-export-format').selectOption('usdc')
  await kit.getByTestId('scene-export-run').click()
  const binaryExported = await until(() => page.evaluate(() => window.devHost.files.has('Sample.usdc')))
  const binaryHeader = binaryExported ? await page.evaluate(() => new TextDecoder().decode(new Uint8Array(window.devHost.files.get('Sample.usdc').bytes).slice(0, 8))) : ''
  step('Export writes a binary USD layer through the host', binaryHeader === 'PXR-USDC', binaryHeader)
  const view = await details()
  const box = await kit.getByTestId('scene-viewport').boundingBox()
  const dpr = await frame.evaluate(() => window.devicePixelRatio)
  step('the document is the viewport, one to one', Math.abs(view.view.width - Math.round(box.width * dpr)) <= 2 && Math.abs(view.view.height - Math.round(box.height * dpr)) <= 2, `${view.view.width}×${view.view.height} for ${box.width}×${box.height}`)
  const probe = () => frame.evaluate(() => window.sceneGl ?? null)
  // The GPU picture against the software one at the same view: the mean difference per channel
  // at 320 × 200, and a digest of both to tell when they have stopped changing.
  const compareGpu = () =>
    frame.evaluate(() => {
      const gl = document.querySelector('[data-testid="scene-viewport"] canvas.scene-gl-canvas')
      const surface = document.querySelector('[data-testid="scene-viewport"] canvas.kui-canvas-surface')
      const read = (source) => {
        const copy = document.createElement('canvas')
        copy.width = 320
        copy.height = 200
        const context = copy.getContext('2d')
        context.drawImage(source, 0, 0, 320, 200)
        return context.getImageData(0, 0, 320, 200).data
      }
      const [a, b] = [read(gl), read(surface)]
      let total = 0
      let digest = 0
      for (let index = 0; index < a.length; index += 4) {
        total += Math.abs(a[index] - b[index]) + Math.abs(a[index + 1] - b[index + 1]) + Math.abs(a[index + 2] - b[index + 2])
        digest = (digest * 31 + a[index] + 7 * b[index + 1]) >>> 0
      }
      return { mean: total / ((a.length / 4) * 3), digest }
    })
  /** Runs `commands`, waits for both pictures to settle, and steps whether the GPU viewport draws
   * the state itself and matches the software picture within `limit` levels on average. */
  const gpuDraws = async (name, commands, limit = 3) => {
    await page.evaluate((commands) => window.devHost.connection().ai.run(commands, false), commands)
    let last = null
    for (let same = 0, tries = 0; same < 4 && tries < 150; tries += 1) {
      await page.waitForTimeout(150)
      const now = await compareGpu()
      same = last && now.digest === last.digest ? same + 1 : 0
      last = now
    }
    const state = await probe()
    if (process.env.KITION_SCENE_GL_SNAPSHOTS && which === 'chromium') {
      await page.screenshot({ path: `${process.env.KITION_SCENE_GL_SNAPSHOTS}/${name.replace(/[^a-z]+/gi, '-')}.png`, clip: await kit.getByTestId('scene-viewport').boundingBox() })
    }
    step(`the GPU viewport draws ${name} as the software one does`, state?.active === true && last.mean < limit, `mean difference ${last.mean.toFixed(2)} levels${state?.active ? '' : `, software because ${state?.reason}`}`)
  }
  if (gpu) await until(async () => (await probe())?.active === true || /emulated/.test((await probe())?.reason ?? ''), 150)
  if (gpu && /emulated/.test((await probe())?.reason ?? '')) {
    // No real GPU here: the GPU viewport leaves the drawing to the software one.
    step('the GPU viewport steps aside on a GPU emulated on the CPU', (await probe())?.active === false)
    gpu = false
  }
  if (gpu) {
    await until(async () => (await probe())?.frames > 0, 150)
    step('the GPU viewport draws the scene', (await probe())?.active === true, JSON.stringify(await probe()))
    const { mean: parity } = await compareGpu()
    step('the GPU picture matches the software one', parity < 6, `mean difference ${parity.toFixed(2)} levels`)
  } else {
    step('the software viewport draws when the GPU one is off', (await probe())?.active !== true)
  }

  const cx = box.x + box.width / 2
  const cy = box.y + box.height / 2
  await page.mouse.click(box.x + 40, box.y + box.height - 40)
  await until(async () => (await details()).selection.objects.length === 0)
  step('a click on nothing deselects', (await details()).selection.objects.length === 0)
  await page.mouse.click(cx, cy)
  await until(async () => (await kit.locator('[data-testid="scene-outliner-item"][data-name="Cube"]').getAttribute('data-selected')) != null)
  step('a click on the cube selects it in the outliner', (await kit.locator('[data-testid="scene-outliner-item"][data-name="Cube"]').getAttribute('data-selected')) != null)

  // Presence: the selection goes out as a KDM id; other people (and the agent) come back drawn.
  await page.mouse.move(cx + 6, cy + 4)
  await until(async () => await page.evaluate(() => { const last = window.devHost.presences.at(-1); return last?.selection?.length === 1 && last?.viewport?.pointer != null }))
  const shared = await page.evaluate(() => window.devHost.presences.at(-1))
  step('presence shares the cube as a KDM id and the pointer as a scene point', shared?.selection?.length === 1 && /-/.test(shared.selection[0]) && shared.viewport?.pointer?.length === 3, JSON.stringify(shared))
  const peerPixelsBefore = await sample()
  await page.evaluate((shared) => window.devHost.setPeers([
    { principal: 'u:alice', name: 'Alice', color: '#0a84ff', selection: shared.selection, viewport: shared.viewport },
    { principal: 'agent', name: 'Agent', color: '#7c5cff', selection: [], agent: { origin: 'agent' } },
  ]), shared)
  await until(async () => (await kit.getByTestId('scene-presence-cursor').count()) === 1)
  step("a peer's pointer shows as a named cursor", (await kit.getByTestId('scene-presence-cursor').count()) === 1 && /Alice/.test(await kit.getByTestId('scene-presence').textContent()))
  step('the outliner badges the cube with the peer', (await kit.locator('[data-testid="scene-outliner-item"][data-name="Cube"] .scene-presence-badge[data-name="Alice"]').count()) === 1)
  step('the agent is a peer named Agent', (await kit.locator('[data-testid="scene-presence-peer"][data-name="Agent"][data-agent]').count()) === 1)
  await until(async () => (await sample()).digest !== peerPixelsBefore.digest)
  step("the viewport outlines the peer's object", (await sample()).digest !== peerPixelsBefore.digest)
  if (process.env.KITION_SCENE_PRESENCE_SNAPSHOT && which === 'chromium') await page.screenshot({ path: process.env.KITION_SCENE_PRESENCE_SNAPSHOT })
  await page.evaluate(() => window.devHost.setPeers([]))
  await until(async () => (await kit.getByTestId('scene-presence-peer').count()) === 0)
  await page.keyboard.press('Shift+D')
  await until(async () => (await kit.locator('[data-testid="scene-outliner-item"][data-name="Cube.001"]').count()) === 1)
  step('Shift+D duplicates the cube', (await kit.locator('[data-testid="scene-outliner-item"][data-name="Cube.001"]').count()) === 1)
  await page.keyboard.press('ControlOrMeta+z')
  await until(async () => (await kit.locator('[data-testid="scene-outliner-item"][data-name="Cube.001"]').count()) === 0)

  await kit.getByTestId('scene-menu-Add').click()
  await kit.locator('[role="menuitem"][data-kit-ui-id="menu:Add/Mesh"]').hover()
  await kit.getByTestId('scene-cmd-add.uvSphere').click()
  await until(async () => (await kit.locator('[data-testid="scene-outliner-item"][data-name="Sphere"]').count()) === 1)
  step('Add ▸ Mesh ▸ UV Sphere adds a sphere', (await kit.locator('[data-testid="scene-outliner-item"][data-name="Sphere"]').count()) === 1)
  // Delete removes what the outliner selected, as in Blender, while the outliner row keeps the focus.
  await kit.locator('[data-testid="scene-outliner-item"][data-name="Sphere"] .scene-outliner-name').click()
  await page.keyboard.press('Delete')
  await until(async () => (await kit.locator('[data-testid="scene-outliner-item"][data-name="Sphere"]').count()) === 0)
  step('Delete removes the object selected in the outliner', (await kit.locator('[data-testid="scene-outliner-item"][data-name="Sphere"]').count()) === 0)
  await page.keyboard.press('ControlOrMeta+z')
  await until(async () => (await kit.locator('[data-testid="scene-outliner-item"][data-name="Sphere"]').count()) === 1)
  await kit.locator('[data-testid="scene-outliner-item"][data-name="Sphere"] .scene-outliner-name').click()

  const field = kit.getByTestId('scene-location-x').locator('input')
  await field.fill('2.5')
  await field.press('Enter')
  await until(async () => (await object('Sphere'))?.location[0] === 2.5)
  step('the properties move the sphere', (await object('Sphere'))?.location[0] === 2.5)
  await kit.getByTestId('scene-add-modifier').click()
  await kit.getByTestId('scene-add-modifier-bevel').click()
  await until(async () => (await object('Sphere'))?.modifiers?.[0]?.type === 'bevel')
  step('the modifier panel adds a bevel', (await object('Sphere'))?.modifiers?.[0]?.type === 'bevel')
  if (process.env.KITION_SCENE_SNAPSHOT && which === 'chromium') {
    await page.waitForTimeout(600)
    await page.screenshot({ path: process.env.KITION_SCENE_SNAPSHOT })
  }

  const before = await sample()
  const turn = (await details()).view.turn
  const history = JSON.stringify([(await details()).undo, (await details()).canRedo])
  await page.mouse.move(cx + 100, cy + 100)
  const framesBefore = gpu ? (await probe())?.frames ?? 0 : 0
  await page.mouse.down({ button: 'middle' })
  for (let i = 1; i <= 30; i += 1) {
    await page.mouse.move(cx + 100 + i * 4, cy + 100 + i)
    if (gpu) await page.waitForTimeout(16)
  }
  const framesDuring = gpu ? ((await probe())?.frames ?? 0) - framesBefore : 0
  await page.mouse.up({ button: 'middle' })
  if (gpu) step('an orbit drag draws a GPU frame per move', framesDuring >= 20, `${framesDuring} frames in a 30-move drag`)
  await until(async () => (await details()).view.turn !== turn)
  await until(async () => (await sample()).digest !== before.digest)
  step('a middle drag orbits the view', (await details()).view.turn !== turn && (await sample()).digest !== before.digest)
  step('orbiting adds no undo step', JSON.stringify([(await details()).undo, (await details()).canRedo]) === history)

  const wheelBefore = (await details()).view.distance
  await page.mouse.move(cx, cy)
  await page.mouse.wheel(0, 240)
  await until(async () => (await details()).view.distance > wheelBefore)
  step('the wheel zooms the view', (await details()).view.distance > wheelBefore)

  await kit.getByTestId('scene-nav-view.front').click()
  await until(async () => (await details()).view.perspective === false)
  step('the navigation widget looks from the front', (await details()).view.perspective === false && Math.abs((await details()).view.tilt - 90) < 0.01)

  await page.keyboard.press('Tab')
  await until(async () => /Edit/.test(await kit.getByTestId('scene-status').textContent()))
  step('Tab enters edit mode', /Edit/.test(await kit.getByTestId('scene-status').textContent()) && (await details()).mode.mode === 'edit')
  await page.keyboard.press('2')
  await until(async () => (await details()).mode.select === 'edge')
  step('2 picks edge select', (await details()).mode.select === 'edge')
  // E on a lone vertex pulls out an edge (no faces selected), and undo takes it back.
  const edited = async () => {
    const now = await details()
    return now.objects.find((item) => item.id === now.mode.object)
  }
  const vertexCount = (await edited())?.mesh?.vertices ?? 0
  await page.keyboard.press('1')
  await until(async () => (await details()).mode.select === 'vertex')
  await page.evaluate(async () => window.devHost.connection().ai.run([{ command: 'mesh.select', params: { vertices: [0] } }], false))
  await kit.getByTestId('scene-viewport').hover()
  await page.keyboard.press('e')
  await until(async () => (await edited())?.mesh?.vertices === vertexCount + 1)
  step('E extrudes a lone vertex', (await edited())?.mesh?.vertices === vertexCount + 1)
  await page.keyboard.press('ControlOrMeta+z')
  await until(async () => (await edited())?.mesh?.vertices === vertexCount)
  step('undo takes the extruded vertex back', (await edited())?.mesh?.vertices === vertexCount)
  await kit.getByTestId('scene-proportional').click()
  await until(async () => (await details()).mode.proportional != null)
  step('the status bar turns proportional editing on', (await details()).mode.proportional?.falloff === 'smooth')
  await kit.getByTestId('scene-proportional-connected').click()
  await until(async () => (await details()).mode.proportional?.connected === true)
  step('connected only follows along the edges', (await details()).mode.proportional?.connected === true)
  // During a Move drag the wheel resizes the radius instead of zooming.
  const radiusBefore = (await details()).mode.proportional.radius
  const distanceBefore = (await details()).view.distance
  await kit.getByTestId('scene-viewport').hover()
  await page.keyboard.press('g')
  await until(async () => (await details()).tool?.id === 'move')
  const dragBox = await kit.getByTestId('scene-viewport').boundingBox()
  await page.mouse.move(dragBox.x + dragBox.width / 2, dragBox.y + dragBox.height / 2)
  await page.mouse.down()
  await page.mouse.move(dragBox.x + dragBox.width / 2 + 30, dragBox.y + dragBox.height / 2, { steps: 4 })
  await page.mouse.wheel(0, -240)
  await until(async () => (await details()).mode.proportional?.radius > radiusBefore)
  const grown = (await details()).mode.proportional?.radius
  await page.mouse.up()
  step('the wheel grows the radius during a drag, and the view stays', grown > radiusBefore && (await details()).view.distance === distanceBefore, `${radiusBefore} → ${grown}`)
  // Take the drag back only if it moved something (Move is the last step then).
  if ((await details()).undo === 'Move') {
    await page.keyboard.press('ControlOrMeta+z')
    await until(async () => (await details()).undo !== 'Move')
  }
  await page.keyboard.press('w')
  await until(async () => (await details()).tool?.id === 'select')
  await kit.getByTestId('scene-viewport').hover()
  await page.keyboard.press('o')
  await until(async () => (await details()).mode.proportional == null)
  step('O turns proportional editing off', (await details()).mode.proportional == null)
  // The UV editor beside the viewport: a click inside a face selects its island, and a drag on
  // the selection moves its coordinates in one step.
  await page.evaluate(() => window.devHost.connection().ai.run([{ command: 'select.all', params: { action: 'deselect' } }], false))
  await kit.getByTestId('scene-uv-toggle').click()
  await until(async () => (await kit.locator('.scene-uv-face').count()) > 0)
  step('the UV editor shows the faces', (await kit.locator('.scene-uv-face').count()) === (await edited())?.mesh?.faces)
  const frameBox = await page.locator('#kit').boundingBox()
  // The panel opening resizes the viewport and refits the UV view: click once the face stays put.
  const quadAt = () => frame.evaluate(() => {
    const polygon = [...document.querySelectorAll('.scene-uv-face')].find((item) => item.points.numberOfItems === 4)
    const matrix = polygon.getScreenCTM()
    let x = 0
    let y = 0
    for (let index = 0; index < 4; index += 1) {
      const point = polygon.points.getItem(index).matrixTransform(matrix)
      x += point.x / 4
      y += point.y / 4
    }
    return { x, y }
  })
  let quad = await quadAt()
  for (let tries = 0; tries < 30; tries += 1) {
    await page.waitForTimeout(100)
    const again = await quadAt()
    const settled = Math.abs(again.x - quad.x) < 0.5 && Math.abs(again.y - quad.y) < 0.5
    quad = again
    if (settled) break
  }
  // Nothing selected first, so only the click can select the island.
  await page.evaluate(() => window.devHost.connection().ai.run([{ command: 'mesh.select', params: { vertices: [] } }]))
  await until(async () => (await details()).selection.vertices?.length === 0 && (await kit.locator('.scene-uv-face[data-selected]').count()) === 0)
  const hitBefore = await frame.evaluate(({ x, y }) => {
    const element = document.elementFromPoint(x, y)
    return element ? `${element.tagName.toLowerCase()}.${String(element.getAttribute('class') ?? '')}` : 'nothing'
  }, quad)
  await page.mouse.click(frameBox.x + quad.x, frameBox.y + quad.y)
  await until(async () => (await details()).selection.vertices?.length > 0 && (await kit.locator('.scene-uv-face[data-selected]').count()) > 0)
  const islandDetail = async () => `hit ${hitBefore}; vertices ${(await details()).selection.vertices?.length}; selected faces ${await kit.locator('.scene-uv-face[data-selected]').count()}`
  if (process.env.KITION_SCENE_UV_SNAPSHOT && which === 'chromium') {
    await page.waitForTimeout(400)
    await page.screenshot({ path: process.env.KITION_SCENE_UV_SNAPSHOT })
  }
  step('a click in the UV editor selects an island', (await details()).selection.vertices?.length > 0 && (await kit.locator('.scene-uv-face[data-selected]').count()) > 0, await islandDetail())
  await page.mouse.move(frameBox.x + quad.x, frameBox.y + quad.y)
  await page.mouse.down()
  await page.mouse.move(frameBox.x + quad.x + 40, frameBox.y + quad.y, { steps: 6 })
  await page.mouse.up()
  await until(async () => (await details()).undo === 'Move UVs')
  step('a drag in the UV editor moves the island', (await details()).undo === 'Move UVs', (await details()).undo)
  await page.keyboard.press('ControlOrMeta+z')
  await until(async () => (await details()).undo !== 'Move UVs')
  await kit.getByTestId('scene-uv-close').click()
  await until(async () => (await kit.getByTestId('scene-uv-editor').count()) === 0)
  // The knife across the middle of the view, confirmed with Enter.
  await page.evaluate(() => window.devHost.connection().ai.run([{ command: 'select.all', params: {} }, { command: 'view.frameSelected', params: {} }, { command: 'select.all', params: { action: 'deselect' } }], false))
  const facesBefore = (await edited())?.mesh?.faces
  const area = await kit.getByTestId('scene-viewport').boundingBox()
  await kit.getByTestId('scene-viewport').hover()
  await page.keyboard.press('k')
  await until(async () => (await details()).tool?.id === 'knife')
  // Off the middle, which runs along the sphere's equator edges (nothing to cut there). The
  // pause keeps the two clicks from reading as a double click, which would end the line.
  await page.mouse.click(area.x + area.width * 0.2, area.y + area.height * 0.42)
  await page.waitForTimeout(400)
  await page.mouse.click(area.x + area.width * 0.8, area.y + area.height * 0.42)
  await page.waitForTimeout(400)
  await page.keyboard.press('Enter')
  await until(async () => (await edited())?.mesh?.faces > facesBefore)
  step('the knife cuts across the mesh', (await edited())?.mesh?.faces > facesBefore, `${facesBefore} → ${(await edited())?.mesh?.faces}`)
  await page.keyboard.press('ControlOrMeta+z')
  await until(async () => (await edited())?.mesh?.faces === facesBefore)
  step('undo puts the cut back', (await edited())?.mesh?.faces === facesBefore)
  await page.keyboard.press('w')
  await until(async () => (await details()).tool?.id === 'select')
  await page.keyboard.press('Tab')
  await until(async () => (await details()).mode.mode === 'object')
  step('Tab leaves edit mode', (await details()).mode.mode === 'object')

  await page.keyboard.press('ControlOrMeta+z')
  await until(async () => !(await object('Sphere'))?.modifiers?.length)
  step('Cmd/Ctrl+Z undoes the bevel modifier', !(await object('Sphere'))?.modifiers?.length)
  await page.keyboard.press('ControlOrMeta+z')
  await until(async () => (await object('Sphere'))?.location[0] === 0)
  step('Cmd/Ctrl+Z undoes the move', (await object('Sphere'))?.location[0] === 0)

  // Animation: key the cube on frame 1, move it on frame 20 and key it, scrub between.
  const animation = async () => (await details()).animation
  await kit.locator('[data-testid="scene-outliner-item"][data-name="Cube"]').click()
  await until(async () => (await details()).selection.active === (await object('Cube')).id)
  await page.keyboard.press('i')
  await until(async () => (await animation()).selectedKeyFrames.includes(1))
  step('I keys the selection on frame 1', (await animation()).selectedKeyFrames.includes(1) && (await kit.locator('.scene-timeline-key').count()) === 1)
  const frameField = kit.getByTestId('scene-timeline-frame').locator('input')
  await frameField.fill('20')
  await frameField.press('Enter')
  await until(async () => (await animation()).frame === 20)
  step('the frame field goes to frame 20', (await animation()).frame === 20)
  const cubeX = kit.getByTestId('scene-location-x').locator('input')
  await cubeX.fill('4')
  await cubeX.press('Enter')
  await until(async () => (await object('Cube'))?.location[0] === 4)
  await kit.getByTestId('scene-timeline-insert').click()
  await until(async () => (await animation()).selectedKeyFrames.includes(20))
  step('the key button keys frame 20', JSON.stringify((await animation()).selectedKeyFrames) === '[1,20]')
  step('keyed fields are tinted', (await kit.getByTestId('scene-location-keyed').count()) === 1)
  await frameField.fill('10')
  await frameField.press('Enter')
  await until(async () => (await animation()).frame === 10)
  const eased = (await object('Cube'))?.location[0]
  step('frame 10 eases between the keys', Math.abs(eased - 1.8423) < 0.01, String(eased))
  step('an unkeyed frame tints the field as animated', (await kit.getByTestId('scene-location-animated').count()) === 1)
  if (process.env.KITION_SCENE_TIMELINE_SNAPSHOT && which === 'chromium') {
    await page.waitForTimeout(400)
    await page.screenshot({ path: process.env.KITION_SCENE_TIMELINE_SNAPSHOT })
  }
  const ruler = await kit.getByTestId('scene-timeline-ruler').boundingBox()
  await page.mouse.click(ruler.x + 4, ruler.y + ruler.height / 2)
  await until(async () => (await animation()).frame === 1)
  step('a click on the ruler scrubs to the start', (await animation()).frame === 1 && (await object('Cube'))?.location[0] === 0, `frame ${(await animation()).frame}, x ${(await object('Cube'))?.location[0]}`)
  await kit.getByTestId('scene-timeline-play').click()
  await until(async () => (await animation()).frame > 3, 60)
  const played = (await animation()).frame
  await kit.getByTestId('scene-timeline-play').click()
  step('play advances the frames', played > 3, String(played))
  await page.keyboard.press('ArrowRight')
  await page.waitForTimeout(150)
  // An image in the cube's material: the panel shows its thumbnail and the material view paints it.
  const RED_PNG = 'iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAEUlEQVR4nGP4z8DwH4QZYAwAR8oH+WdZbrcAAAAASUVORK5CYII='
  await page.evaluate((base64) => window.devHost.connection().ai.run([{ command: 'select.objects', params: { objects: 'Cube' } }, { command: 'material.setTexture', params: { material: 'Material', base64, name: 'red.png' } }], false), RED_PNG)
  await until(async () => (await kit.locator('[data-testid="scene-texture-baseColor"] img').count()) === 1)
  step('the material panel shows the image', (await kit.locator('[data-testid="scene-texture-baseColor"] img').count()) === 1)
  await page.evaluate(() => window.devHost.connection().ai.run([{ command: 'view.shading', params: { mode: 'material' } }, { command: 'select.objects', params: { objects: 'Cube' } }, { command: 'view.frameSelected', params: {} }], false))
  const reddened = await until(async () => {
    const { centre } = await sample()
    return centre[0] > centre[1] + 40 && centre[0] > centre[2] + 40
  })
  step('the material view paints the image', reddened, JSON.stringify((await sample()).centre))
  if (process.env.KITION_SCENE_SNAPSHOT && which === 'chromium') {
    await kit.getByTestId('scene-texture-baseColor').scrollIntoViewIfNeeded()
    await page.screenshot({ path: process.env.KITION_SCENE_SNAPSHOT.replace(/\.png$/, '-textured.png') })
  }

  // Path tracing in the render dialog: a small render, a few samples, stepped in the worker.
  await page.evaluate(() => window.devHost.connection().ai.run([{ command: 'render.set', params: { width: 160, height: 90 } }]))
  await kit.getByTestId('scene-render').click()
  await until(async () => (await kit.getByTestId('scene-render-image').count()) === 1, 150)
  step('the render dialog opens with a fast render', (await kit.getByTestId('scene-render-image').count()) === 1)
  await kit.getByTestId('scene-render-engine-path').click()
  if ((await kit.getByTestId('scene-render-cancel').count()) === 1) await kit.getByTestId('scene-render-cancel').click()
  const samplesField = kit.getByTestId('scene-render-samples').locator('input')
  await samplesField.fill('4')
  await samplesField.press('Enter')
  await kit.getByTestId('scene-render-start').click()
  const traced = await until(async () => (await kit.getByTestId('scene-render-progress').getAttribute('data-state')) === 'done', 300)
  step('path tracing renders its samples and shows the picture', traced && /4 \/ 4/.test(await kit.getByTestId('scene-render-progress').textContent()) && (await kit.getByTestId('scene-render-image').count()) === 1, await kit.getByTestId('scene-render-progress').textContent())
  // Split across workers or not, the picture is the same, byte for byte.
  const picture = () =>
    frame.evaluate(async () => {
      const image = document.querySelector('[data-testid="scene-render-image"]')
      const bytes = new Uint8Array(await (await fetch(image.src)).arrayBuffer())
      let digest = 0
      for (const byte of bytes) digest = (digest * 31 + byte) >>> 0
      return `${bytes.length}:${digest}`
    })
  const progress = kit.getByTestId('scene-render-progress')
  const autoWorkers = Number(await progress.getAttribute('data-workers'))
  const cores = await frame.evaluate(() => navigator.hardwareConcurrency || 1)
  step('path tracing splits across workers by the cores', autoWorkers === Math.max(1, Math.min(8, cores - 1)), `${autoWorkers} workers on ${cores} cores`)
  const pictures = [[autoWorkers, await picture()]]
  for (const count of [1, autoWorkers === 3 ? 2 : 3]) {
    const workersField = kit.getByTestId('scene-render-workers').locator('input')
    await workersField.fill(String(count))
    await workersField.press('Enter')
    await kit.getByTestId('scene-render-start').click()
    const done = await until(async () => (await progress.getAttribute('data-state')) === 'done' && Number(await progress.getAttribute('data-workers')) === count, 300)
    pictures.push([count, done ? await picture() : 'not done'])
  }
  step('the picture is the same from one, three, and every worker', pictures.every(([, digest]) => digest === pictures[0][1]), JSON.stringify(pictures))
  if (process.env.KITION_SCENE_SNAPSHOT && which === 'chromium') await page.screenshot({ path: process.env.KITION_SCENE_SNAPSHOT.replace(/\.png$/, '-trace.png') })
  await page.keyboard.press('Escape')
  await until(async () => (await kit.getByTestId('scene-render-dialog').count()) === 0)
  // Glass on the cube's material, a world image, and the camera's depth of field from the panels.
  await kit.locator('[data-testid="scene-outliner-item"][data-name="Cube"]').click()
  await until(async () => (await details()).selection.active === (await object('Cube')).id)
  const transmissionField = kit.getByTestId('scene-transmission').locator('input')
  await transmissionField.fill('1')
  await transmissionField.press('Enter')
  const material = async () => {
    const id = (await object('Cube')).material
    return (await details()).materials.find((item) => item.id === id)
  }
  await until(async () => (await material())?.transmission === 1)
  step('the material panel makes the cube glass', (await material())?.transmission === 1)
  // A 4 × 2 Radiance image: flat RGBE pixels, sky blue above, dark below.
  const sky = await page.evaluate(() => {
    const header = new TextEncoder().encode('#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 2 +X 4\n')
    const pixels = [...Array(4).fill([64, 128, 255, 129]), ...Array(4).fill([32, 32, 32, 126])].flat()
    const bytes = new Uint8Array([...header, ...pixels])
    let binary = ''
    for (const byte of bytes) binary += String.fromCharCode(byte)
    return btoa(binary)
  })
  await page.evaluate((base64) => window.devHost.connection().ai.run([{ command: 'world.setImage', params: { base64, name: 'Sky.hdr' } }]), sky)
  await until(async () => (await details()).world?.image != null)
  const settings = kit.getByTestId('scene-settings')
  if ((await settings.getAttribute('data-open')) !== 'true') await settings.locator('.kui-panel-section-toggle').first().click()
  await until(async () => (await kit.locator('[data-testid="scene-world-image"] img').count()) === 1)
  step('a world image shows in the Scene section', (await details()).world?.image != null && (await kit.locator('[data-testid="scene-world-image"] img').count()) === 1)
  await kit.locator('[data-testid="scene-outliner-item"][data-name="Camera"]').click()
  await kit.getByTestId('scene-dof').click()
  const cameraOf = async () => (await object('Camera'))?.camera
  await until(async () => (await cameraOf())?.depthOfField === true)
  step('the camera panel turns depth of field on', (await cameraOf())?.depthOfField === true)
  // Rigging: an armature from the Add menu, a bone extruded in its edit mode, the sphere bound
  // with automatic weights, a bone posed and keyed, and its curve in the graph editor.
  await page.evaluate(() => window.devHost.connection().ai.run([{ command: 'view.shading', params: { mode: 'solid' } }, { command: 'frame.set', params: { frame: 1 } }]))
  await kit.getByTestId('scene-menu-Add').click()
  await kit.getByTestId('scene-cmd-add.armature').click()
  await until(async () => (await kit.locator('[data-testid="scene-outliner-item"][data-name="Armature"]').count()) === 1)
  step('Add ▸ Armature adds an armature', (await object('Armature'))?.armature?.bones?.length === 1)
  // The editor knows the armature is active once its bones show.
  await kit.locator('[data-testid="scene-bone-item"][data-name="Bone"]').waitFor()
  await kit.locator('.kui-app').focus()
  await page.keyboard.press('Tab')
  await until(async () => (await details()).mode.mode === 'armatureEdit')
  step('Tab edits the armature', (await details()).mode.mode === 'armatureEdit' && /Edit/.test(await kit.getByTestId('scene-status').textContent()))
  await kit.locator('[data-testid="scene-bone-item"][data-name="Bone"]').click()
  await kit.getByTestId('scene-bone-extrude').click()
  await until(async () => (await object('Armature'))?.armature?.bones?.length === 2)
  step('Extrude grows a connected bone', (await object('Armature'))?.armature?.bones?.[1]?.connected === true)
  await page.evaluate(() =>
    window.devHost.connection().ai.run([
      { command: 'mode.armature', params: {} },
      // The sphere's middle on the joint, so bone heat shares it between the two bones.
      { command: 'object.transform', params: { objects: 'Sphere', location: [0, 0, 1] } },
      { command: 'select.objects', params: { objects: ['Sphere', 'Armature'] } },
      { command: 'object.parentArmature', params: { weights: 'automatic' } },
    ]),
  )
  await until(async () => (await object('Sphere'))?.vertexGroups?.length === 2)
  step('the sphere is bound with automatic weights', (await object('Sphere'))?.vertexGroups?.length === 2)
  await kit.getByRole('radio', { name: /Pose/ }).first().click().catch(async () => {
    await page.evaluate(() => window.devHost.connection().ai.run([{ command: 'mode.pose', params: { object: 'Armature', on: true } }]))
  })
  await until(async () => (await details()).mode.mode === 'pose')
  await kit.locator('[data-testid="scene-bone-item"][data-name="Bone.001"]').click()
  await kit.getByTestId('scene-pose-rotation-x').waitFor()
  await until(async () => (await kit.locator('[data-testid="scene-bone-item"][data-name="Bone.001"]').getAttribute('aria-selected')) === 'true')
  await kit.locator('.kui-app').focus()
  await page.keyboard.press('i')
  await until(async () => (await details()).animation.channels.some((channel) => channel.path === 'Bone.001'))
  step('I keys the bone in pose mode', (await details()).animation.channels.some((channel) => channel.path === 'Bone.001'))
  const unposed = (await object('Sphere')).dimensions
  const poseX = kit.getByTestId('scene-pose-rotation-x').locator('input')
  await poseX.fill('60')
  await poseX.press('Enter')
  await until(async () => JSON.stringify((await object('Sphere')).dimensions) !== JSON.stringify(unposed))
  step('posing the bone bends the bound sphere', JSON.stringify((await object('Sphere')).dimensions) !== JSON.stringify(unposed))
  await kit.getByTestId('scene-graph-toggle').click()
  await kit.locator('[data-testid="scene-bone-item"][data-name="Bone.001"]').scrollIntoViewIfNeeded()
  await page.evaluate(() => window.devHost.connection().ai.run([{ command: 'mode.pose', params: { on: false } }, { command: 'select.objects', params: { objects: 'Armature' } }]))
  await until(async () => (await kit.getByTestId('scene-graph-key').count()) > 0)
  step('the graph editor shows the bone curves', (await kit.getByTestId('scene-graph-key').count()) > 0)
  if (process.env.KITION_SCENE_SNAPSHOT && which === 'chromium') {
    await page.screenshot({ path: process.env.KITION_SCENE_SNAPSHOT.replace(/\.png$/, '-rig.png') })
  }

  // Constraints: the camera aims at the sphere from the Constraints panel, and a posed bone gets IK.
  await page.evaluate(() => window.devHost.connection().ai.run([{ command: 'select.objects', params: { objects: 'Camera' } }]))
  await kit.getByTestId('scene-constraints').waitFor()
  await kit.getByTestId('scene-constraints').getByTestId('scene-add-constraint').click()
  await kit.getByTestId('scene-add-constraint-trackTo').click()
  await until(async () => (await object('Camera'))?.constraints?.length === 1)
  step('the Constraints panel adds Track To', (await object('Camera'))?.constraints?.[0]?.type === 'trackTo')
  await kit.getByTestId('scene-constraint-0-target').selectOption(String((await object('Sphere')).id))
  await until(async () => (await object('Camera'))?.constraints?.[0]?.target?.name === 'Sphere')
  step('the target field aims the camera at the sphere', (await object('Camera'))?.constraints?.[0]?.target?.name === 'Sphere')
  const influence = kit.getByTestId('scene-constraint-0-influence').locator('input')
  await influence.fill('0.5')
  await influence.press('Enter')
  await until(async () => (await object('Camera'))?.constraints?.[0]?.influence === 0.5)
  step('the influence field blends the constraint', (await object('Camera'))?.constraints?.[0]?.influence === 0.5)
  await page.evaluate(() => window.devHost.connection().ai.run([{ command: 'mode.pose', params: { object: 'Armature', on: true } }, { command: 'bone.select', params: { bones: ['Bone.001'] } }]))
  await kit.getByTestId('scene-bone-constraints').waitFor()
  await kit.getByTestId('scene-bone-constraints').getByTestId('scene-add-constraint').click()
  await kit.getByTestId('scene-add-constraint-ik').click()
  const ik = async () => (await object('Armature'))?.boneConstraints?.['Bone.001']?.[0]
  await until(async () => (await ik())?.type === 'ik')
  await kit.getByTestId('scene-bone-constraints').getByTestId('scene-constraint-0-target').selectOption(String((await object('Cube')).id))
  await until(async () => (await ik())?.target?.name === 'Cube')
  step('a pose bone gets IK aimed at a target', (await ik())?.type === 'ik' && (await ik())?.target?.name === 'Cube', JSON.stringify((await object('Armature'))?.boneConstraints))
  await kit.getByTestId('scene-bone-constraints').getByTestId('scene-constraint-0-remove').click()
  await until(async () => (await ik()) === undefined)
  step('the remove button takes the IK away', (await ik()) === undefined)
  // Weight paint: the status bar's switch, the brush, a stroke on the sphere, and Normalize All.
  await page.evaluate(() => window.devHost.connection().ai.run([{ command: 'mode.pose', params: { on: false } }, { command: 'select.objects', params: { objects: 'Sphere' } }, { command: 'view.frameSelected', params: {} }]))
  await kit.getByTestId('scene-weight-paint').click()
  await until(async () => (await details()).mode.mode === 'weightPaint')
  step('the status bar switches to weight paint', /Weight Paint/.test(await kit.getByTestId('scene-status').first().textContent()) && (await kit.getByTestId('scene-weight-toolbar').count()) === 1)
  await kit.getByTestId('scene-brush-mode-subtract').click()
  await until(async () => (await details()).mode.paint?.brush?.mode === 'subtract')
  step('the toolbar sets the brush', (await details()).mode.paint.brush.mode === 'subtract')
  const paintBox = await kit.getByTestId('scene-viewport').boundingBox()
  const [px, py] = [paintBox.x + paintBox.width / 2, paintBox.y + paintBox.height / 2]
  const strokeHistory = (await details()).undo
  await page.mouse.move(px - 30, py)
  await page.mouse.down()
  for (let i = 1; i <= 6; i += 1) await page.mouse.move(px - 30 + i * 10, py + i * 2)
  await page.mouse.up()
  await until(async () => (await details()).undo === 'Paint Weights')
  step('a stroke paints the weights as one step', (await details()).undo === 'Paint Weights' && strokeHistory !== 'Paint Weights', (await details()).undo)
  await kit.getByTestId('scene-normalize-all').click()
  await until(async () => (await details()).undo === 'Normalize All')
  step('Normalize All is one step', (await details()).undo === 'Normalize All')
  // The Linear Gradient tool from the toolbar, dragged across the sphere, and X mirror.
  await kit.getByTestId('scene-weight-tool-linearGradient').click()
  await until(async () => (await details()).mode.paint?.tool === 'linearGradient')
  step('the toolbar picks the linear gradient', (await details()).mode.paint.tool === 'linearGradient')
  await page.mouse.move(px - 60, py)
  await page.mouse.down()
  for (let i = 1; i <= 6; i += 1) await page.mouse.move(px - 60 + i * 20, py)
  await page.mouse.up()
  await until(async () => (await details()).undo === 'Gradient')
  step('a gradient dragged in the viewport is one step', (await details()).undo === 'Gradient', (await details()).undo)
  await kit.getByTestId('scene-weight-x-mirror').click()
  await until(async () => (await details()).mode.paint?.mirror?.x === true)
  step('X mirror turns on from the toolbar', (await details()).mode.paint.mirror.x === true && (await kit.getByTestId('scene-weight-topology-mirror').count()) === 1)
  if (process.env.KITION_SCENE_SNAPSHOT && which === 'chromium') await page.screenshot({ path: process.env.KITION_SCENE_SNAPSHOT.replace(/\.png$/, '-weights.png') })
  await kit.getByTestId('scene-weight-paint').click()
  await until(async () => (await details()).mode.mode === 'object')
  step('the switch leaves weight paint', (await details()).mode.mode === 'object')

  // Curves and text: a Bezier curve from Add ▸ Curve, bevelled in its panel, a point extruded
  // with E in its edit mode, a text typed into in the viewport, and the text made a mesh.
  await kit.getByTestId('scene-menu-Add').click()
  await kit.locator('[role="menuitem"][data-kit-ui-id="menu:Add/Curve"]').hover()
  await kit.getByTestId('scene-cmd-add.curveBezier').click()
  await until(async () => (await kit.locator('[data-testid="scene-outliner-item"][data-name="BezierCurve"]').count()) === 1)
  step('Add ▸ Curve ▸ Bezier adds a curve', (await object('BezierCurve'))?.type === 'curve')
  const bevel = kit.getByTestId('scene-curve-bevel').locator('input')
  await bevel.fill('0.1')
  await bevel.press('Enter')
  await until(async () => (await object('BezierCurve'))?.curve?.bevelDepth === 0.1)
  step('the Curve panel bevels the curve', (await object('BezierCurve'))?.curve?.evaluated?.faces === 144, JSON.stringify((await object('BezierCurve'))?.curve?.evaluated))
  await kit.locator('.kui-app').focus()
  await page.keyboard.press('Tab')
  await until(async () => (await details()).mode.mode === 'curveEdit')
  step('Tab edits the curve', (await details()).mode.mode === 'curveEdit' && /Edit/.test(await kit.getByTestId('scene-status').textContent()))
  await page.evaluate(() => window.devHost.connection().ai.run([{ command: 'curve.select', params: { points: [1] } }]))
  await until(async () => /Points: 1 selected/.test(await kit.getByTestId('scene-status').textContent()))
  await kit.locator('.kui-app').focus()
  await page.keyboard.press('e')
  await until(async () => (await object('BezierCurve'))?.curve?.splines?.[0]?.points?.length === 3)
  step('E extrudes the end point', (await object('BezierCurve'))?.curve?.splines?.[0]?.points?.length === 3 && (await details()).undo === 'Extrude Points', (await details()).undo)
  await page.keyboard.press('Tab')
  await until(async () => (await details()).mode.mode === 'object')
  step('Tab leaves the curve', (await details()).mode.mode === 'object')
  await kit.getByTestId('scene-menu-Add').click()
  await kit.getByTestId('scene-cmd-add.text').click()
  await until(async () => (await object('Text'))?.type === 'text')
  step('Add ▸ Text adds a text', (await object('Text'))?.text?.body === 'Text')
  await kit.getByTestId('scene-text').waitFor()
  await kit.locator('.kui-app').focus()
  await page.keyboard.press('Tab')
  await until(async () => (await details()).mode.mode === 'textEdit')
  step('Tab types into the text', (await details()).mode.mode === 'textEdit')
  await page.waitForTimeout(200)
  await page.keyboard.type('ed')
  await until(async () => (await object('Text'))?.text?.body === 'Texted')
  step('typed letters go into the text', (await object('Text'))?.text?.body === 'Texted', (await object('Text'))?.text?.body)
  await page.keyboard.press('Backspace')
  await until(async () => (await object('Text'))?.text?.body === 'Texte')
  step('Backspace deletes before the caret', (await object('Text'))?.text?.body === 'Texte')
  await page.keyboard.press('Escape')
  await until(async () => (await details()).mode.mode === 'object')
  step('Esc leaves the text', (await details()).mode.mode === 'object' && (await object('Text'))?.text?.body === 'Texte')
  if (process.env.KITION_SCENE_SNAPSHOT && which === 'chromium') await page.screenshot({ path: process.env.KITION_SCENE_SNAPSHOT.replace(/\.png$/, '-curves.png') })
  await kit.getByTestId('scene-convert-mesh').click()
  await until(async () => (await object('Text'))?.type === 'mesh')
  step('Convert to Mesh makes the text a mesh', (await object('Text'))?.type === 'mesh' && (await object('Text'))?.mesh?.faces > 0)

  // Sculpt: the status bar's switch, a pointer stroke raising a grid (one undo step), undo, and
  // the mask protecting it.
  await page.evaluate(() =>
    window.devHost.connection().ai.run([
      { command: 'add.grid', params: { name: 'Clay', xSubdivisions: 60, ySubdivisions: 60, size: 2, location: [12, 12, 0] } },
      { command: 'view.top', params: {} },
      { command: 'view.frameSelected', params: {} },
    ]),
  )
  await until(async () => (await object('Clay'))?.type === 'mesh')
  await kit.getByTestId('scene-sculpt').click()
  await until(async () => (await details()).mode.mode === 'sculpt')
  step('the status bar switches to sculpt', /Sculpt/.test(await kit.getByTestId('scene-status').first().textContent()) && (await kit.getByTestId('scene-sculpt-toolbar').count()) === 1)
  const clayHeight = async () => (await object('Clay'))?.dimensions?.[2] ?? -1
  const sculptBox = await kit.getByTestId('scene-viewport').boundingBox()
  const [sx, sy] = [sculptBox.x + sculptBox.width / 2, sculptBox.y + sculptBox.height / 2]
  const sculptStroke = async () => {
    await page.mouse.move(sx - 60, sy)
    await page.mouse.down()
    for (let i = 1; i <= 12; i += 1) await page.mouse.move(sx - 60 + i * 10, sy + i)
    await page.mouse.up()
  }
  await sculptStroke()
  await until(async () => (await details()).undo === 'Sculpt')
  step('a stroke with Draw raises the grid under the pointer as one step', (await clayHeight()) > 0.001 && (await details()).undo === 'Sculpt', `${await clayHeight()} ${(await details()).undo}`)
  await page.keyboard.press('ControlOrMeta+z')
  await until(async () => (await clayHeight()) === 0)
  step('Cmd/Ctrl+Z takes the stroke back', (await clayHeight()) === 0)
  await page.evaluate(() => window.devHost.connection().ai.run([{ command: 'sculpt.maskFill', params: {} }]))
  await until(async () => /masked/.test(await kit.getByTestId('scene-counts').textContent()) && (await details()).mode.sculpt?.masked > 0)
  await sculptStroke()
  await until(async () => (await details()).mode.sculpt?.stroking === false)
  step('the mask protects the grid', (await clayHeight()) === 0 && (await details()).mode.sculpt.masked > 3000)
  if (process.env.KITION_SCENE_SNAPSHOT && which === 'chromium') await page.screenshot({ path: process.env.KITION_SCENE_SNAPSHOT.replace(/\.png$/, '-sculpt.png') })
  await kit.getByTestId('scene-sculpt-mask-clear').click()
  await until(async () => (await details()).mode.sculpt?.masked === 0)
  step('Clear Mask clears it', (await details()).mode.sculpt.masked === 0)

  // The box mask tool masks what a dragged box encloses; Face Set from Masked makes a face set
  // of it; H over it hides it, and Show All shows it again.
  const sculptInfo = async () => (await details()).mode.sculpt ?? {}
  await kit.getByTestId('scene-sculpt-tool').selectOption('boxMask')
  await until(async () => (await sculptInfo()).tool === 'boxMask')
  await page.mouse.move(sculptBox.x + 4, sculptBox.y + 4)
  await page.mouse.down()
  await page.mouse.move(sx - 40, sy, { steps: 4 })
  await page.mouse.move(sx, sculptBox.y + sculptBox.height - 4, { steps: 4 })
  await page.mouse.up()
  await until(async () => (await sculptInfo()).masked > 0)
  const boxMasked = (await sculptInfo()).masked
  step('the box mask tool masks what the box encloses', boxMasked > 300 && boxMasked < 3400, `${boxMasked} masked`)
  await kit.getByTestId('scene-sculpt-face-set-mask').click()
  await until(async () => (await sculptInfo()).faceSets?.length === 1)
  step('Face Set from Masked makes one face set', (await sculptInfo()).faceSets?.length === 1 && /1/.test(await kit.getByTestId('scene-sculpt-face-set-count').textContent()))
  if (process.env.KITION_SCENE_SNAPSHOT && which === 'chromium') await page.screenshot({ path: process.env.KITION_SCENE_SNAPSHOT.replace(/\.png$/, '-face-sets.png') })
  await kit.getByTestId('scene-sculpt-mask-clear').click()
  await until(async () => (await sculptInfo()).masked === 0)
  await page.mouse.move(sx - 120, sy)
  await page.keyboard.press('h')
  await until(async () => (await sculptInfo()).hiddenFaces > 0)
  step('H hides the face set under the pointer', (await sculptInfo()).hiddenFaces > 100, `${(await sculptInfo()).hiddenFaces} hidden`)
  await kit.getByTestId('scene-sculpt-show-all').click()
  await until(async () => (await sculptInfo()).hiddenFaces === 0)
  step('Show All shows every face again', (await sculptInfo()).hiddenFaces === 0)
  await kit.getByTestId('scene-sculpt-face-sets-clear').click()
  await until(async () => (await sculptInfo()).faceSets?.length === 0)

  // Dynamic topology: on, the grid becomes triangles and a stroke adds faces where it goes.
  await kit.getByTestId('scene-sculpt-tool').selectOption('brush')
  await until(async () => (await sculptInfo()).tool === 'brush')
  await kit.getByTestId('scene-sculpt-dyntopo').click()
  await until(async () => (await sculptInfo()).dyntopo?.enabled === true && (await sculptInfo()).triangles === true)
  const trianglesBefore = (await object('Clay'))?.mesh?.faces ?? 0
  await sculptStroke()
  await until(async () => ((await object('Clay'))?.mesh?.faces ?? 0) > trianglesBefore)
  const trianglesAfter = (await object('Clay'))?.mesh?.faces ?? 0
  step('with dynamic topology a stroke adds detail where it goes', trianglesBefore === 7200 && trianglesAfter > trianglesBefore + 20, `${trianglesBefore} → ${trianglesAfter} faces`)
  await kit.getByTestId('scene-sculpt-dyntopo').click()
  await until(async () => (await sculptInfo()).dyntopo?.enabled === false)
  await kit.getByTestId('scene-sculpt').click()
  await until(async () => (await details()).mode.mode === 'object')
  step('the switch leaves sculpt', (await details()).mode.mode === 'object')

  // Multiresolution: the Multires modifier from the panel, two levels by its Subdivide button,
  // and a stroke in sculpt mode shaping level 2 while the base keeps its eight vertices.
  await page.evaluate(() =>
    window.devHost.connection().ai.run([
      { command: 'add.cube', params: { name: 'Lump', location: [-12, 12, 0] } },
      { command: 'view.frameSelected', params: {} },
    ]),
  )
  await until(async () => (await object('Lump'))?.type === 'mesh')
  await kit.getByTestId('scene-add-modifier').click()
  await kit.getByTestId('scene-add-modifier-multires').click()
  await until(async () => (await object('Lump'))?.modifiers?.[0]?.type === 'multires')
  await kit.getByTestId('scene-multires-subdivide').click()
  await until(async () => (await object('Lump'))?.mesh?.multiresLevels === 1)
  await kit.getByTestId('scene-multires-subdivide').click()
  await until(async () => (await object('Lump'))?.mesh?.multiresLevels === 2)
  const lump = await object('Lump')
  step('Subdivide adds multiresolution levels', lump?.mesh?.multiresLevels === 2 && lump?.mesh?.evaluated?.vertices === 98 && /2/.test(await kit.getByTestId('scene-modifier-0-total').textContent()), JSON.stringify(lump?.mesh))
  await kit.getByTestId('scene-sculpt').click()
  await until(async () => (await sculptInfo()).level === 2)
  step('sculpt mode works on the sculpt level', /2/.test(await kit.getByTestId('scene-sculpt-level').textContent()))
  const lumpBefore = JSON.stringify((await object('Lump')).dimensions)
  await sculptStroke()
  await until(async () => JSON.stringify((await object('Lump'))?.dimensions) !== lumpBefore && (await details()).undo === 'Sculpt')
  if (process.env.KITION_SCENE_SNAPSHOT && which === 'chromium') await page.screenshot({ path: process.env.KITION_SCENE_SNAPSHOT.replace(/\.png$/, '-multires.png') })
  step('a stroke shapes the level and the base keeps its vertices', JSON.stringify((await object('Lump')).dimensions) !== lumpBefore && (await object('Lump')).mesh.vertices === 8, `${lumpBefore} → ${JSON.stringify((await object('Lump')).dimensions)}`)
  await kit.getByTestId('scene-sculpt').click()
  await until(async () => (await details()).mode.mode === 'object')

  // Physics: a floor and a box made rigid bodies in the Physics panel, the scene's frame range
  // shortened, a bake stepped in the engine's worker with its progress, the timeline showing the
  // fall, and the bake freed.
  const physics = async () => (await details()).physics
  const openSection = async (testId) => {
    const section = kit.getByTestId(testId)
    await section.waitFor()
    if ((await section.getAttribute('data-open')) !== 'true') await section.locator('.kui-panel-section-toggle').first().click()
  }
  await page.evaluate(() =>
    window.devHost.connection().ai.run([
      { command: 'add.plane', params: { name: 'Floor', size: 6, location: [-12, -12, 0] } },
      { command: 'add.cube', params: { name: 'Faller', size: 1, location: [-12, -12, 3] } },
      { command: 'select.objects', params: { objects: 'Floor' } },
    ]),
  )
  await openSection('scene-physics')
  await kit.getByTestId('scene-physics-add-rigid').click()
  await until(async () => (await physics())?.objects?.length === 1)
  step('the Physics panel adds a rigid body', (await physics())?.objects?.[0]?.rigidBody?.kind === 'active')
  await kit.getByTestId('scene-rigid-type').selectOption('passive')
  await until(async () => (await physics())?.objects?.[0]?.rigidBody?.kind === 'passive')
  step('the type field makes the floor passive', (await physics())?.objects?.[0]?.rigidBody?.kind === 'passive')
  await page.evaluate(() => window.devHost.connection().ai.run([{ command: 'select.objects', params: { objects: 'Faller' } }]))
  await openSection('scene-physics')
  await kit.getByTestId('scene-physics-add-rigid').click()
  await until(async () => (await physics())?.objects?.length === 2)
  step('the box becomes an active rigid body', (await physics())?.objects?.[1]?.rigidBody?.kind === 'active')
  await openSection('scene-physics-world')
  const physicsEnd = kit.getByTestId('scene-physics-end').locator('input')
  await physicsEnd.fill('48')
  await physicsEnd.press('Enter')
  await until(async () => (await physics())?.world?.frameEnd === 48)
  step('the Scene physics section sets the frame range', (await physics())?.world?.frameEnd === 48)
  await kit.getByTestId('scene-physics-bake').click()
  const sawBaking = await until(async () => (await physics())?.state === 'baking' || (await physics())?.state === 'baked', 50)
  await until(async () => (await physics())?.state === 'baked', 300)
  step('Bake steps the simulation to the end with its progress', sawBaking && (await physics())?.state === 'baked' && /48/.test(await kit.getByTestId('scene-physics-progress').textContent()), await kit.getByTestId('scene-physics-progress').textContent())
  await page.evaluate(() => window.devHost.connection().ai.run([{ command: 'frame.set', params: { frame: 48 } }]))
  await until(async () => (await object('Faller'))?.worldLocation?.[2] < 1)
  const landed = (await object('Faller'))?.worldLocation?.[2]
  step('the timeline shows the box resting on the floor', Math.abs(landed - 0.5) < 0.05, String(landed))
  await kit.getByTestId('scene-physics-free').click()
  await until(async () => (await physics())?.state === 'none')
  step('Free Bake puts the box back where it was', (await physics())?.state === 'none' && Math.abs((await object('Faller'))?.worldLocation?.[2] - 3) < 1e-3)
  // Deactivation, a force field, and a hinge from the Physics panel: the box starts asleep and
  // hangs; a hinge on an empty joins it to the floor; a wind on another empty.
  const physicsOf = async (name) => {
    const id = (await object(name))?.id
    return (await physics())?.objects?.find((item) => item.id === id)
  }
  await page.evaluate(() => window.devHost.connection().ai.run([{ command: 'select.objects', params: { objects: 'Faller' } }]))
  await openSection('scene-physics')
  await kit.getByTestId('scene-rigid-deactivation').click()
  await until(async () => (await physicsOf('Faller'))?.rigidBody?.deactivation === true)
  await kit.getByTestId('scene-rigid-asleep').click()
  await until(async () => (await physicsOf('Faller'))?.rigidBody?.startDeactivated === true)
  step('the rigid body card sets deactivation and starting asleep', (await physicsOf('Faller'))?.rigidBody?.startDeactivated === true)
  await page.evaluate(() =>
    window.devHost.connection().ai.run([
      { command: 'add.empty', params: { name: 'Gust', location: [-14, -12, 1] } },
      { command: 'add.empty', params: { name: 'Joint', location: [-12, -12, 4], rotation: [90, 0, 0] } },
      { command: 'select.objects', params: { objects: 'Gust' } },
    ]),
  )
  await openSection('scene-physics')
  await kit.getByTestId('scene-physics-add-field').click()
  await until(async () => (await physicsOf('Gust'))?.field?.kind === 'force')
  await kit.getByTestId('scene-field-type').selectOption('wind')
  await until(async () => (await physicsOf('Gust'))?.field?.kind === 'wind')
  const strength = kit.getByTestId('scene-field-strength').locator('input')
  await strength.fill('3')
  await strength.press('Enter')
  await until(async () => (await physicsOf('Gust'))?.field?.strength === 3)
  step('the Physics panel adds a force field to an empty and sets it', (await physicsOf('Gust'))?.field?.kind === 'wind' && (await physicsOf('Gust'))?.field?.strength === 3, JSON.stringify((await physicsOf('Gust'))?.field))
  await page.evaluate(() => window.devHost.connection().ai.run([{ command: 'select.objects', params: { objects: 'Joint' } }]))
  await openSection('scene-physics')
  await kit.getByTestId('scene-physics-add-constraint').click()
  await until(async () => (await physicsOf('Joint'))?.rigidConstraint != null)
  await kit.getByTestId('scene-constraint-type').selectOption('hinge')
  await until(async () => (await physicsOf('Joint'))?.rigidConstraint?.kind === 'hinge')
  const floorId = String((await object('Floor')).id)
  const fallerId = String((await object('Faller')).id)
  await kit.getByTestId('scene-constraint-object1').selectOption(floorId)
  await until(async () => String((await physicsOf('Joint'))?.rigidConstraint?.object1) === floorId)
  await kit.getByTestId('scene-constraint-object2').selectOption(fallerId)
  await until(async () => String((await physicsOf('Joint'))?.rigidConstraint?.object2) === fallerId)
  await kit.getByTestId('scene-constraint-angle-on').click()
  await until(async () => (await physicsOf('Joint'))?.rigidConstraint?.angleLimit != null)
  const hinge = (await physicsOf('Joint'))?.rigidConstraint
  step('the constraint card joins the floor and the box with a limited hinge', hinge?.kind === 'hinge' && String(hinge?.object1) === floorId && String(hinge?.object2) === fallerId && Math.abs(hinge?.angleLimit?.[1] - Math.PI / 4) < 1e-4, JSON.stringify(hinge))
  await openSection('scene-physics-world')
  await kit.getByTestId('scene-physics-bake').click()
  await until(async () => (await physics())?.state === 'baked', 300)
  await page.evaluate(() => window.devHost.connection().ai.run([{ command: 'frame.set', params: { frame: 48 } }]))
  await page.waitForTimeout(200)
  const hung = (await object('Faller'))?.worldLocation?.[2]
  step('baked, the box asleep on its hinge stays up', (await physics())?.state === 'baked' && Math.abs(hung - 3) < 1e-3, String(hung))
  await kit.getByTestId('scene-constraint-remove').click()
  await until(async () => (await physicsOf('Joint')) == null)
  step('the constraint card removes the constraint', (await physicsOf('Joint')) == null)
  await kit.getByTestId('scene-physics-free').click()
  await until(async () => (await physics())?.state === 'none')
  await page.evaluate(() => window.devHost.connection().ai.run([{ command: 'frame.set', params: { frame: 1 } }]))
  // Particles: hair added from the Particles panel and drawn, groomed in Particle Edit with a
  // pointer stroke (one undo step), and Particle Edit left from its tool header.
  await page.evaluate(() =>
    window.devHost.connection().ai.run([
      { command: 'add.uvSphere', params: { name: 'Furball', location: [-12, 12, 0] } },
      { command: 'view.front', params: {} },
      { command: 'view.frameSelected', params: {} },
    ]),
  )
  await until(async () => (await object('Furball'))?.type === 'mesh')
  await page.waitForTimeout(300)
  const bare = (await sample()).digest
  await kit.getByTestId('scene-add-particles').click()
  await kit.getByTestId('scene-add-particles-hair').click()
  const hairOf = async () => (await details()).particles?.find((system) => system.kind === 'hair')
  await until(async () => (await hairOf())?.count === 1000)
  step('the Particles panel adds hair to the sphere', (await hairOf())?.count === 1000 && (await kit.getByTestId('scene-particles-hairLength').count()) === 1)
  await until(async () => (await sample()).digest !== bare)
  step('the viewport draws the hair', (await sample()).digest !== bare)
  await kit.getByTestId('scene-particle-edit').click()
  await until(async () => (await details()).mode.mode === 'particleEdit')
  step('Particle Edit shows its tool header', (await details()).mode.mode === 'particleEdit' && (await kit.getByTestId('scene-particle-toolbar').count()) === 1 && /Particle Edit/.test(await kit.getByTestId('scene-status').first().textContent()))
  const hairBox = await kit.getByTestId('scene-viewport').boundingBox()
  const [hx, hy] = [hairBox.x + hairBox.width / 2, hairBox.y + hairBox.height / 2]
  await page.mouse.move(hx - 80, hy)
  await page.mouse.down()
  for (let i = 1; i <= 16; i += 1) await page.mouse.move(hx - 80 + i * 10, hy - i * 2)
  await page.mouse.up()
  await until(async () => (await details()).undo === 'Groom' && (await hairOf())?.groomed === true)
  step('a comb stroke grooms the hair as one step', (await details()).undo === 'Groom' && (await hairOf())?.groomed === true, `${(await details()).undo}`)
  if (process.env.KITION_SCENE_SNAPSHOT && which === 'chromium') await page.screenshot({ path: process.env.KITION_SCENE_SNAPSHOT.replace(/\.png$/, '-hair.png') })
  await page.keyboard.press('ControlOrMeta+z')
  await until(async () => (await hairOf())?.groomed === false)
  step('Cmd/Ctrl+Z takes the groom back', (await hairOf())?.groomed === false)
  await kit.getByTestId('scene-groom-exit').click()
  await until(async () => (await details()).mode.mode === 'object')
  step('Done leaves Particle Edit', (await details()).mode.mode === 'object')
  // Geometry nodes: the node editor makes a group on a cube, adds a Grid from the searched menu,
  // links it to the output by a drag, sets a value, moves the node, and deletes it (undo brings
  // it back).
  await page.evaluate(() =>
    window.devHost.connection().ai.run([
      { command: 'add.cube', params: { name: 'Nodes', location: [-12, -12, 0] } },
      { command: 'select.objects', params: { objects: ['Nodes'] } },
    ]),
  )
  await until(async () => (await object('Nodes'))?.type === 'mesh')
  await kit.getByTestId('scene-nodes-toggle').click()
  await kit.getByTestId('scene-nodes').waitFor()
  await kit.getByTestId('scene-nodes-new').click()
  await until(async () => (await details()).nodeGroups?.length === 1)
  const nodeGroup = async () => (await details()).nodeGroups?.[0]
  const nodesMesh = async () => (await object('Nodes'))?.mesh?.evaluated
  step('the node editor makes a group on the active cube', (await object('Nodes'))?.modifiers?.[0]?.type === 'geometryNodes' && (await nodeGroup())?.nodes?.length === 2 && (await kit.locator('.scene-node').count()) === 2, JSON.stringify([(await object('Nodes'))?.modifiers, (await nodeGroup())?.nodes?.length, await kit.locator('.scene-node').count(), (await details()).selection]))
  await kit.getByTestId('scene-nodes-add').click()
  await kit.getByTestId('scene-nodes-search').fill('grid')
  await until(async () => (await kit.getByTestId('scene-nodes-add-meshGrid').count()) === 1)
  await kit.getByTestId('scene-nodes-search').press('Enter')
  await until(async () => (await nodeGroup())?.nodes?.some((node) => node.kind === 'meshGrid'))
  const gridNode = (await nodeGroup())?.nodes?.find((node) => node.kind === 'meshGrid')
  const outputNode = (await nodeGroup())?.nodes?.find((node) => node.kind === 'groupOutput')
  step('the add menu searches and adds a Grid', gridNode != null && (await kit.locator('.scene-node[data-kind="meshGrid"]').count()) === 1)
  await kit.getByTestId('scene-nodes-frame').click()
  await page.waitForTimeout(100)
  const socketCentre = async (testId) => {
    const found = await kit.getByTestId(testId).boundingBox()
    return [found.x + found.width / 2, found.y + found.height / 2]
  }
  const [fromX, fromY] = await socketCentre(`scene-socket-${gridNode.id}-out-Mesh`)
  const [toX, toY] = await socketCentre(`scene-socket-${outputNode.id}-in-Geometry`)
  await page.mouse.move(fromX, fromY)
  await page.mouse.down()
  for (let i = 1; i <= 10; i += 1) await page.mouse.move(fromX + ((toX - fromX) * i) / 10, fromY + ((toY - fromY) * i) / 10)
  await page.mouse.up()
  await until(async () => (await nodesMesh())?.vertices === 9)
  step('a drag links the Grid to the output', (await nodeGroup())?.links?.some((link) => link.from === gridNode.id && link.to === outputNode.id) && (await nodesMesh())?.vertices === 9, JSON.stringify(await nodesMesh()))
  const verticesX = kit.getByTestId(new RegExp(`^scene-node-${gridNode.id}-Vertices X-`))
  await verticesX.fill('5')
  await verticesX.press('Enter')
  await until(async () => (await nodesMesh())?.vertices === 15)
  step('a node value changes the result', (await nodesMesh())?.vertices === 15)
  const header = await kit.locator(`[data-testid="scene-node-${gridNode.id}"] .scene-node-header`).boundingBox()
  const placed = (await nodeGroup()).nodes.find((node) => node.id === gridNode.id).location
  await page.mouse.move(header.x + 30, header.y + 10)
  await page.mouse.down()
  for (let i = 1; i <= 6; i += 1) await page.mouse.move(header.x + 30 + i * 10, header.y + 10 + i * 4)
  await page.mouse.up()
  await until(async () => (await nodeGroup()).nodes.find((node) => node.id === gridNode.id).location[0] !== placed[0])
  step('dragging a title moves the node as one step', (await nodeGroup()).nodes.find((node) => node.id === gridNode.id).location[0] !== placed[0] && (await details()).undo === 'Move Nodes', (await details()).undo)
  await kit.getByTestId('scene-nodes-canvas').focus()
  await page.keyboard.press('Delete')
  await until(async () => (await nodeGroup())?.nodes?.length === 2)
  step('Delete removes the selected node and its links', (await nodeGroup())?.nodes?.length === 2 && (await nodesMesh())?.vertices === 0)
  await page.keyboard.press('ControlOrMeta+z')
  await until(async () => (await nodeGroup())?.nodes?.length === 3)
  step('undo brings the node back', (await nodeGroup())?.nodes?.length === 3 && (await nodesMesh())?.vertices === 15)
  if (process.env.KITION_SCENE_SNAPSHOT && which === 'chromium') await page.screenshot({ path: process.env.KITION_SCENE_SNAPSHOT.replace(/\.png$/, '-nodes.png') })
  // Nested groups and zones: a second group ("Inner", with a Height input and a Lift output) is
  // used by a Group node in the cube's group, entered from the node and left by the path; a
  // repeat zone comes from the add menu as a pair in a frame, and an item added on its output
  // shows on both of its nodes.
  const outerId = (await nodeGroup()).id
  await page.evaluate(() => window.devHost.connection().ai.run([{ command: 'nodes.newGroup', params: { name: 'Inner' } }]))
  await until(async () => (await details()).nodeGroups?.length === 2)
  const innerGroup = (await details()).nodeGroups.find((group) => group.name === 'Inner')
  await page.evaluate(
    ([inner, outer]) =>
      window.devHost.connection().ai.run([
        { command: 'nodes.addInput', params: { group: inner, name: 'Height', type: 'float', default: 1 } },
        { command: 'nodes.addOutput', params: { group: inner, name: 'Lift', type: 'float' } },
        { command: 'nodes.addNode', params: { group: outer, kind: 'group', location: [0, 260], values: { group: inner } } },
      ]),
    [innerGroup.id, outerId],
  )
  await until(async () => (await nodeGroup()).nodes.some((node) => node.kind === 'group'))
  const groupNode = (await nodeGroup()).nodes.find((node) => node.kind === 'group')
  await until(async () => (await kit.getByTestId(`scene-socket-${groupNode.id}-out-Lift`).count()) === 1)
  await kit.getByTestId('scene-nodes-frame').click()
  step(
    'a Group node shows its group and that group\'s interface',
    (await kit.getByTestId(`scene-socket-${groupNode.id}-in-Height`).count()) === 1 &&
      (await kit.getByTestId(`scene-socket-${groupNode.id}-out-Lift`).count()) === 1 &&
      /Inner/.test(await kit.locator(`[data-testid="scene-node-${groupNode.id}"] .scene-node-header`).textContent()),
  )
  await kit.getByTestId(`scene-node-${groupNode.id}-enter`).click()
  const innerOutput = innerGroup.nodes.find((node) => node.kind === 'groupOutput')
  await until(async () => (await kit.getByTestId(`scene-node-${innerOutput.id}`).count()) === 1)
  step(
    'entering the Group node shows its group and the path back',
    (await kit.getByTestId(`scene-node-${innerOutput.id}`).count()) === 1 &&
      (await kit.getByTestId('scene-nodes-path').count()) === 1 &&
      (await kit.getByTestId(`scene-socket-${innerOutput.id}-in-Lift`).count()) === 1,
  )
  await kit.getByTestId('scene-nodes-crumb-0').click()
  await until(async () => (await kit.getByTestId(`scene-node-${groupNode.id}`).count()) === 1)
  step('the path leads back to the cube\'s group', (await kit.getByTestId(`scene-node-${groupNode.id}`).count()) === 1 && (await kit.getByTestId('scene-nodes-path').count()) === 0)
  await kit.getByTestId('scene-nodes-add').click()
  await kit.getByTestId('scene-nodes-search').fill('repeat')
  await until(async () => (await kit.getByTestId('scene-nodes-add-repeatInput').count()) === 1)
  step('the add menu offers a repeat zone, not its output alone', (await kit.getByTestId('scene-nodes-add-repeatInput').count()) === 1 && (await kit.getByTestId('scene-nodes-add-repeatOutput').count()) === 0)
  await kit.getByTestId('scene-nodes-search').press('Enter')
  await until(async () => (await nodeGroup()).nodes.some((node) => node.kind === 'repeatOutput'))
  const repeatIn = (await nodeGroup()).nodes.find((node) => node.kind === 'repeatInput')
  const repeatOut = (await nodeGroup()).nodes.find((node) => node.kind === 'repeatOutput')
  await until(async () => (await kit.getByTestId(`scene-nodes-zone-${repeatIn?.id}`).count()) === 1)
  step('a repeat zone comes as a pair in a frame', repeatIn?.zone?.pair === repeatOut?.id && (await kit.getByTestId(`scene-nodes-zone-${repeatIn?.id}`).count()) === 1)
  await kit.getByTestId('scene-nodes-frame').click()
  await kit.getByTestId(`scene-node-${repeatOut.id}-add-item`).click()
  await until(async () => (await nodeGroup()).nodes.find((node) => node.id === repeatOut.id)?.zone?.items?.length === 2)
  step(
    'an item added on the zone\'s output shows on both of its nodes',
    (await kit.getByTestId(`scene-socket-${repeatIn.id}-out-Float`).count()) === 1 && (await kit.getByTestId(`scene-socket-${repeatOut.id}-in-Float`).count()) === 1,
  )
  // Compositing: the node editor makes a compositing group renders pass through, an Exposure
  // node is added and linked between Render Layers and the output by drags, its value brightens
  // the backdrop and the render dialog's picture (which can turn compositing off), and RGB
  // Curves take a point added on their square.
  await kit.getByTestId('scene-nodes-new-compositor').click()
  await until(async () => (await details()).render?.compositor != null)
  const compositor = async () => {
    const now = await details()
    return now.nodeGroups?.find((group) => group.id === now.render?.compositor)
  }
  step(
    'the node editor makes a compositing group renders pass through',
    (await compositor())?.tree === 'compositor' && (await kit.locator('.scene-node[data-kind="renderLayers"]').count()) === 1 && (await kit.locator('.scene-node[data-kind="compositorOutput"]').count()) === 1,
    JSON.stringify((await details()).render),
  )
  await kit.getByTestId('scene-nodes-add').click()
  await kit.getByTestId('scene-nodes-search').fill('expos')
  await until(async () => (await kit.getByTestId('scene-nodes-add-exposure').count()) === 1)
  step('the add menu lists compositing nodes only', (await kit.getByTestId('scene-nodes-add-meshGrid').count()) === 0 && (await kit.getByTestId('scene-nodes-add-exposure').count()) === 1)
  await kit.getByTestId('scene-nodes-search').press('Enter')
  await until(async () => (await compositor())?.nodes?.some((node) => node.kind === 'exposure'))
  const exposureNode = (await compositor()).nodes.find((node) => node.kind === 'exposure')
  const layersNode = (await compositor()).nodes.find((node) => node.kind === 'renderLayers')
  const compositeNode = (await compositor()).nodes.find((node) => node.kind === 'compositorOutput')
  // Move the Exposure node clear of the others, then link by drags.
  const exposureHeader = await kit.locator(`[data-testid="scene-node-${exposureNode.id}"] .scene-node-header`).boundingBox()
  await page.mouse.move(exposureHeader.x + 20, exposureHeader.y + 8)
  await page.mouse.down()
  for (let i = 1; i <= 6; i += 1) await page.mouse.move(exposureHeader.x + 20, exposureHeader.y + 8 + i * 25)
  await page.mouse.up()
  await kit.getByTestId('scene-nodes-frame').click()
  await page.waitForTimeout(100)
  const dragLink = async (from, to) => {
    const [ax, ay] = await socketCentre(from)
    const [bx, by] = await socketCentre(to)
    await page.mouse.move(ax, ay)
    await page.mouse.down()
    for (let i = 1; i <= 10; i += 1) await page.mouse.move(ax + ((bx - ax) * i) / 10, ay + ((by - ay) * i) / 10)
    await page.mouse.up()
  }
  await dragLink(`scene-socket-${layersNode.id}-out-Image`, `scene-socket-${exposureNode.id}-in-Image`)
  await dragLink(`scene-socket-${exposureNode.id}-out-Image`, `scene-socket-${compositeNode.id}-in-Image`)
  const linkedThrough = await until(async () => {
    const links = (await compositor())?.links ?? []
    return links.some((link) => link.from === layersNode.id && link.to === exposureNode.id) && links.some((link) => link.from === exposureNode.id && link.to === compositeNode.id)
  })
  step('drags link Render Layers through Exposure to the output', linkedThrough, JSON.stringify((await compositor())?.links))
  const exposureField = kit.getByTestId(new RegExp(`^scene-node-${exposureNode.id}-Exposure-`))
  await exposureField.fill('2')
  await exposureField.press('Enter')
  await until(async () => (await compositor())?.nodes?.find((node) => node.id === exposureNode.id)?.values?.Exposure === 2)
  const backdropReady = await until(async () => (await kit.getByTestId('scene-compositor-backdrop').getAttribute('data-ready')) === 'true', 150)
  step('the backdrop shows the composited render', backdropReady && (await compositor())?.nodes?.find((node) => node.id === exposureNode.id)?.values?.Exposure === 2)
  await kit.getByTestId('scene-nodes-add').click()
  await kit.getByTestId('scene-nodes-search').fill('curves')
  await until(async () => (await kit.getByTestId('scene-nodes-add-rgbCurves').count()) === 1)
  await kit.getByTestId('scene-nodes-search').press('Enter')
  await until(async () => (await compositor())?.nodes?.some((node) => node.kind === 'rgbCurves'))
  const curvesNode = (await compositor()).nodes.find((node) => node.kind === 'rgbCurves')
  await kit.getByTestId('scene-nodes-frame').click()
  await page.waitForTimeout(100)
  const square = await kit.getByTestId(`scene-node-${curvesNode.id}-curves`).boundingBox()
  await page.mouse.dblclick(square.x + square.width * 0.4, square.y + square.height * 0.3)
  const curved = await until(async () => (await compositor())?.nodes?.find((node) => node.id === curvesNode.id)?.values?.curves?.c?.length === 3)
  step('a double-click on RGB Curves adds a point', curved, JSON.stringify((await compositor())?.nodes?.find((node) => node.id === curvesNode.id)?.values))
  if (process.env.KITION_SCENE_SNAPSHOT && which === 'chromium') await page.screenshot({ path: process.env.KITION_SCENE_SNAPSHOT.replace(/\.png$/, '-compositor.png') })
  // The render dialog's picture is composited, and its Compositing switch turns that off.
  const renderBrightness = () =>
    frame.evaluate(async () => {
      const image = document.querySelector('[data-testid="scene-render-image"]')
      if (!image) return -1
      await image.decode().catch(() => {})
      const canvas = document.createElement('canvas')
      canvas.width = image.naturalWidth
      canvas.height = image.naturalHeight
      const context = canvas.getContext('2d')
      context.drawImage(image, 0, 0)
      const data = context.getImageData(0, 0, canvas.width, canvas.height).data
      let sum = 0
      for (let index = 0; index < data.length; index += 4) sum += data[index] + data[index + 1] + data[index + 2]
      return sum / (data.length / 4) / 3
    })
  await kit.getByTestId('scene-render').click()
  await until(async () => (await kit.getByTestId('scene-render-image').count()) === 1, 150)
  await page.waitForTimeout(200)
  const brighter = await renderBrightness()
  await kit.getByTestId('scene-render-compositing').click()
  await until(async () => (await details()).render?.compositing === false)
  await until(async () => Math.abs((await renderBrightness()) - brighter) > 5, 100)
  const plain = await renderBrightness()
  step('the render dialog renders composited and turns compositing off', brighter > plain + 5 && (await details()).render?.compositing === false, `${brighter.toFixed(1)} against ${plain.toFixed(1)}`)
  await kit.getByTestId('scene-render-compositing').click()
  await until(async () => (await details()).render?.compositing === true)
  await page.keyboard.press('Escape')
  await until(async () => (await kit.getByTestId('scene-render-dialog').count()) === 0)
  await kit.getByTestId('scene-nodes-toggle').click()
  // Lattices and the deforming modifiers: a cube bent by a lattice (added from the Add menu,
  // its points dragged in lattice edit mode), shrinkwrapped onto, and decimated by its panel.
  await page.evaluate(() =>
    window.devHost.connection().ai.run([
      { command: 'add.cube', params: { name: 'Caged', location: [12, -12, 0] } },
      { command: 'select.objects', params: { objects: ['Caged'] } },
    ]),
  )
  await until(async () => (await object('Caged'))?.type === 'mesh')
  await kit.getByTestId('scene-menu-Add').click()
  await kit.getByTestId('scene-cmd-add.lattice').click()
  await until(async () => (await object('Lattice'))?.type === 'lattice')
  step('the Add menu adds a lattice', (await object('Lattice'))?.lattice?.pointCount === 8 && (await kit.getByTestId('scene-lattice').count()) === 1)
  await kit.getByTestId('scene-lattice-u').locator('input').fill('3')
  await kit.getByTestId('scene-lattice-u').locator('input').press('Enter')
  await until(async () => (await object('Lattice'))?.lattice?.pointCount === 12)
  step('the Lattice panel sets the points along U', (await object('Lattice'))?.lattice?.resolution?.join() === '3,2,2')
  await page.evaluate(() =>
    window.devHost.connection().ai.run([
      { command: 'object.transform', params: { objects: ['Lattice'], location: [12, -12, 0], scale: [2.5, 2.5, 2.5] } },
      { command: 'select.objects', params: { objects: ['Caged'] } },
    ]),
  )
  await kit.getByTestId('scene-add-modifier').click()
  await kit.getByTestId('scene-add-modifier-lattice').click()
  await until(async () => (await object('Caged'))?.modifiers?.[0]?.type === 'lattice')
  await kit.getByTestId('scene-modifier-0-operand').selectOption(String((await object('Lattice')).id))
  await until(async () => (await object('Caged'))?.modifiers?.[0]?.operand === (await object('Lattice')).id)
  step('the modifier panel adds a lattice modifier and picks the lattice', (await object('Caged'))?.modifiers?.[0]?.operand === (await object('Lattice')).id)
  await page.evaluate(() => window.devHost.connection().ai.run([{ command: 'select.objects', params: { objects: ['Lattice'] } }, { command: 'view.frameSelected', params: {} }]))
  await kit.getByTestId('scene-viewport').hover()
  await page.keyboard.press('Tab')
  await until(async () => (await details()).mode.mode === 'latticeEdit' && /Lattice Edit/.test(await kit.getByTestId('scene-status').first().textContent()))
  step('Tab enters lattice edit mode', (await details()).mode.mode === 'latticeEdit' && /Lattice Edit/.test(await kit.getByTestId('scene-status').first().textContent()), await kit.getByTestId('scene-status').first().textContent())
  await page.keyboard.press('a')
  await until(async () => (await details()).mode.points?.length === 12)
  step('A selects every lattice point', (await details()).mode.points?.length === 12, JSON.stringify((await details()).mode))
  const cagedBefore = (await object('Caged')).dimensions
  await page.evaluate(() => window.devHost.connection().ai.run([{ command: 'lattice.select', params: { points: [2, 5, 8, 11] } }]))
  await until(async () => (await details()).mode.points?.length === 4)
  await kit.getByTestId('scene-viewport').hover()
  await page.keyboard.press('g')
  await until(async () => (await details()).tool?.id === 'move')
  const latticeBox = await kit.getByTestId('scene-viewport').boundingBox()
  const [lx, ly] = [latticeBox.x + latticeBox.width / 2, latticeBox.y + latticeBox.height / 2]
  await page.mouse.move(lx, ly)
  await page.mouse.down()
  for (let i = 1; i <= 8; i += 1) await page.mouse.move(lx + i * 12, ly)
  await page.mouse.up()
  await until(async () => (await details()).undo === 'Transform Points')
  const cagedAfter = (await object('Caged')).dimensions
  step('dragging lattice points bends the caged cube in one step', (await object('Lattice'))?.lattice?.moved?.length === 4 && cagedAfter[0] > cagedBefore[0] + 0.05 && (await details()).undo === 'Transform Points', JSON.stringify([cagedBefore, cagedAfter, (await object('Lattice')).lattice, (await object('Lattice')).scale, (await details()).view]))
  await page.keyboard.press('Tab')
  await until(async () => (await details()).mode.mode === 'object')
  step('Tab leaves lattice edit mode', (await details()).mode.mode === 'object')
  await page.keyboard.press('w')
  await page.evaluate(() =>
    window.devHost.connection().ai.run([
      { command: 'add.uvSphere', params: { name: 'Wrapped', location: [12, -12, 0], radius: 2 } },
      { command: 'select.objects', params: { objects: ['Wrapped'] } },
    ]),
  )
  await until(async () => (await object('Wrapped'))?.type === 'mesh')
  await kit.getByTestId('scene-add-modifier').click()
  await kit.getByTestId('scene-add-modifier-shrinkwrap').click()
  await until(async () => (await object('Wrapped'))?.modifiers?.[0]?.type === 'shrinkwrap')
  await kit.getByTestId('scene-modifier-0-operand').selectOption(String((await object('Caged')).id))
  await until(async () => (await object('Wrapped'))?.dimensions?.[2] < 3)
  step('a shrinkwrap pulls the sphere onto the caged cube', (await object('Wrapped'))?.dimensions?.[2] < 3, JSON.stringify((await object('Wrapped'))?.dimensions))
  await kit.getByTestId('scene-modifier-0-wrapMethod').selectOption('project')
  await until(async () => (await object('Wrapped'))?.modifiers?.[0]?.wrapMethod === 'project')
  step('the shrinkwrap panel switches to projecting', (await kit.getByText('Against normal').count()) === 1)
  await kit.getByTestId('scene-add-modifier').click()
  await kit.getByTestId('scene-add-modifier-decimate').click()
  await until(async () => (await object('Wrapped'))?.modifiers?.[1]?.type === 'decimate')
  await kit.getByTestId('scene-modifier-1-mode').selectOption('unsubdivide')
  await until(async () => (await kit.getByTestId('scene-modifier-1-iterations').count()) === 1)
  await kit.getByTestId('scene-modifier-1-iterations').locator('input').fill('1')
  await kit.getByTestId('scene-modifier-1-iterations').locator('input').press('Enter')
  await until(async () => (await object('Wrapped'))?.mesh?.evaluated?.vertices === 242)
  step('decimate un-subdivides from its panel', (await object('Wrapped'))?.mesh?.evaluated?.vertices === 242, JSON.stringify((await object('Wrapped'))?.mesh))
  if (process.env.KITION_SCENE_SNAPSHOT && which === 'chromium') await page.screenshot({ path: process.env.KITION_SCENE_SNAPSHOT.replace(/\.png$/, '-lattice.png') })

  // Drivers, keying sets, actions, and Stretch To from the panels.
  await page.evaluate(() => window.devHost.connection().ai.run([{ command: 'add.empty', params: { name: 'Knob', location: [2, 0, 0] } }, { command: 'select.objects', params: { objects: 'Caged' } }]))
  await kit.getByTestId('scene-drivers').waitFor()
  await kit.getByTestId('scene-add-driver').click()
  await kit.getByTestId('scene-add-driver-location-2').click()
  const driver = async () => (await details()).animation.drivers[0]
  await until(async () => (await details()).animation.drivers.length === 1)
  step('the Drivers panel adds a driver', (await details()).animation.drivers[0]?.property === 'location')
  await kit.getByTestId('scene-driver-0-variable-0-type').selectOption('transform')
  await until(async () => (await details()).animation.drivers[0]?.variables?.[0]?.type === 'transform')
  await kit.getByTestId('scene-driver-0-variable-0-object').selectOption(String((await object('Knob')).id))
  await until(async () => (await details()).animation.drivers[0]?.variables?.[0]?.target?.name === 'Knob')
  const expression = kit.getByTestId('scene-driver-0-expression')
  await expression.fill('var * 2')
  await expression.press('Enter')
  await until(async () => Math.abs(((await object('Caged'))?.location?.[2] ?? 0) - 4) < 1e-3)
  step('a driven height follows the knob', Math.abs(((await object('Caged'))?.location?.[2] ?? 0) - 4) < 1e-3, JSON.stringify((await driver()) ?? null))
  await kit.getByTestId('scene-driver-0-remove').click()
  await until(async () => (await details()).animation.drivers.length === 0)
  await kit.getByTestId('scene-keying-set').selectOption('locRot')
  await until(async () => (await details()).animation.keyingSets.active === 'locRot')
  await kit.getByTestId('scene-timeline-insert').click()
  await until(async () => (await object('Caged'))?.animated?.includes('rotation'))
  step('the timeline keys what the keying set names', JSON.stringify((await object('Caged'))?.animated) === '["location","rotation"]', JSON.stringify((await object('Caged'))?.animated))
  await kit.getByTestId('scene-stash-action').click()
  await until(async () => (await details()).animation.actions.length === 1)
  step('the Actions panel stashes the keys in an action', (await details()).animation.actions[0]?.channels === 6)
  // The NLA editor: the keys pushed down into a strip, the strip slowed from its settings and
  // tweaked, a Cycles modifier added to a tweaked curve from the graph editor (open since the
  // rigging steps), and tweaking stopped.
  await kit.getByTestId('scene-nla-toggle').click()
  await kit.getByTestId('scene-nla-push-down').click()
  const nlaOf = async () => (await object('Caged'))?.nla
  await until(async () => (await nlaOf())?.tracks?.[0]?.strips?.length === 1)
  step('Push Down puts the keys on an NLA strip', (await nlaOf())?.tracks?.[0]?.strips?.length === 1 && !(await object('Caged'))?.animated, JSON.stringify(await nlaOf()))
  await kit.getByTestId('scene-nla-strip-0-0').click()
  const stripScale = kit.getByTestId('scene-nla-scale').locator('input')
  await stripScale.fill('2')
  await stripScale.press('Enter')
  await until(async () => (await nlaOf())?.tracks?.[0]?.strips?.[0]?.scale === 2)
  step('the strip settings slow the strip down', (await nlaOf())?.tracks?.[0]?.strips?.[0]?.scale === 2)
  await kit.getByTestId('scene-nla-tweak').click()
  await until(async () => (await nlaOf())?.tweak != null)
  step('Tweak makes the strip action the keys', (await nlaOf())?.tweak?.track === 0 && (await object('Caged'))?.animated?.length === 2)
  await until(async () => (await kit.getByTestId('scene-graph-key').count()) > 0)
  await kit.getByTestId('scene-graph-key').first().click()
  await kit.getByTestId('scene-fmod-add').click()
  await kit.getByTestId('scene-fmod-add-cycles').click()
  const caged = (await object('Caged')).id
  const cycled = async () => (await details()).animation.channels.some((channel) => channel.object === caged && channel.modifiers?.[0]?.type === 'cycles')
  await until(cycled)
  step('the graph editor adds a Cycles modifier', await cycled())
  await kit.getByTestId('scene-fmod-add').click()
  await kit.getByTestId('scene-fmod-add-limits').click()
  await until(async () => (await kit.getByTestId('scene-fmod-use-maxY').count()) === 1)
  await kit.getByTestId('scene-fmod-use-maxY').click()
  const maxValue = kit.getByTestId('scene-fmod-maxY').locator('input')
  await maxValue.fill('12')
  await maxValue.press('Enter')
  const limited = async () => (await details()).animation.channels.some((channel) => channel.object === caged && channel.modifiers?.some((modifier) => modifier.type === 'limits' && modifier.maxY === 12))
  await until(limited)
  step('the graph editor edits an independent Limits value bound', await limited())
  await kit.getByTestId('scene-nla-tweak-exit').click()
  await until(async () => (await nlaOf())?.tweak == null)
  step('Stop tweaking keeps the strip', (await nlaOf())?.tweak == null && (await nlaOf())?.tracks?.[0]?.strips?.length === 1)
  // Click inside the strip body: its label can extend over the next strip.
  const firstStrip = (await nlaOf()).tracks[0].strips[0]
  await page.evaluate(async ({ action, start }) => window.devHost.connection().ai.run([{ command: 'nla.addStrip', params: { object: 'Caged', action, track: 0, start } }], false), { action: firstStrip.action, start: firstStrip.end + 10 })
  await until(async () => (await nlaOf())?.tracks?.[0]?.strips?.length === 2)
  await kit.getByTestId('scene-nla-strip-0-0').click({ position: { x: 5, y: 10 } })
  await kit.getByTestId('scene-nla-transition').click()
  await until(async () => (await nlaOf())?.tracks?.[0]?.strips?.[1]?.kind === 'transition')
  step('the NLA editor adds a transition across the gap', (await nlaOf())?.tracks?.[0]?.strips?.[1]?.kind === 'transition')
  await kit.getByTestId('scene-nla-group').click()
  await until(async () => (await nlaOf())?.tracks?.[0]?.strips?.[0]?.kind === 'meta')
  step('the NLA editor groups the actions and their transition', (await nlaOf())?.tracks?.[0]?.strips?.[0]?.children?.length === 3)
  await kit.getByTestId('scene-nla-strip-0-0').click({ position: { x: 5, y: 10 } })
  const groupEnd = kit.getByTestId('scene-nla-end').locator('input')
  const originalEnd = (await nlaOf()).tracks[0].strips[0].end
  await groupEnd.fill(String(originalEnd + 10))
  await groupEnd.press('Enter')
  await until(async () => (await nlaOf())?.tracks?.[0]?.strips?.[0]?.end === originalEnd + 10)
  step('stretching a meta changes its children together', (await nlaOf())?.tracks?.[0]?.strips?.[0]?.children?.[2]?.end === originalEnd + 10)
  await kit.getByTestId('scene-nla-dissolve').click()
  await until(async () => (await nlaOf())?.tracks?.[0]?.strips?.length === 3)
  step('ungrouping preserves the transition', (await nlaOf())?.tracks?.[0]?.strips?.[1]?.kind === 'transition')
  if (process.env.KITION_SCENE_SNAPSHOT && which === 'chromium') await page.screenshot({ path: process.env.KITION_SCENE_SNAPSHOT.replace(/\.png$/, '-nla.png') })
  await kit.getByTestId('scene-nla-toggle').click()
  await kit.getByTestId('scene-constraints').getByTestId('scene-add-constraint').click()
  await kit.getByTestId('scene-add-constraint-stretchTo').click()
  await until(async () => (await object('Caged'))?.constraints?.[0]?.type === 'stretchTo')
  await kit.getByTestId('scene-constraint-0-target').selectOption(String((await object('Knob')).id))
  await until(async () => ((await object('Caged'))?.constraints?.[0]?.restLength ?? 0) > 0)
  step('Stretch To takes its rest length when aimed', ((await object('Caged'))?.constraints?.[0]?.restLength ?? 0) > 0, JSON.stringify((await object('Caged'))?.constraints))
  if (gpu) {
    // The GPU viewport draws the states it once left to the software one, each against the
    // software picture: shadows, a world image, a camera's view, sculpt mode, weight paint,
    // particles, and edit modes' points and bones. Everything so far is hidden first.
    const { width: W, height: H } = (await details()).view
    const sky = await frame.evaluate(() => {
      const canvas = document.createElement('canvas')
      canvas.width = 64
      canvas.height = 32
      const context = canvas.getContext('2d')
      const gradient = context.createLinearGradient(0, 0, 0, 32)
      gradient.addColorStop(0, '#2a5ba5')
      gradient.addColorStop(0.5, '#c8d4e8')
      gradient.addColorStop(1, '#3a3028')
      context.fillStyle = gradient
      context.fillRect(0, 0, 64, 32)
      context.fillStyle = '#ffe2a0'
      context.fillRect(40, 6, 6, 4)
      return canvas.toDataURL('image/png').split(',')[1]
    })
    await gpuDraws('a point light’s shadows', [
      { command: 'select.all', params: {} },
      { command: 'object.hide', params: {} },
      { command: 'add.plane', params: { name: 'Stage', size: 12, location: [40, 0, 0] } },
      { command: 'add.cube', params: { name: 'Block', location: [40, 0, 1] } },
      { command: 'add.uvSphere', params: { name: 'Ball', location: [42.5, 1, 0.8], radius: 0.8 } },
      { command: 'add.light', params: { name: 'Bulb', kind: 'point', power: 300, location: [37, -3, 5] } },
      { command: 'select.objects', params: { objects: ['Stage'] } },
      { command: 'view.frameSelected', params: {} },
      { command: 'view.orbit', params: { tilt: -15 } },
      { command: 'view.shading', params: { mode: 'material' } },
    ])
    await gpuDraws('a spot light’s and a sun’s shadows', [
      { command: 'add.light', params: { name: 'Spot', kind: 'spot', power: 500, location: [44, -4, 6], rotation: [35, 0, 45] } },
      { command: 'add.light', params: { name: 'Sun', kind: 'sun', power: 1, location: [40, 0, 8], rotation: [30, 15, -40] } },
    ])
    await gpuDraws('a world image', [
      { command: 'object.hide', params: { objects: ['Bulb', 'Spot', 'Sun'] } },
      { command: 'world.setImage', params: { base64: sky, name: 'Sky.png', strength: 0.8 } },
    ])
    await gpuDraws('the view through a camera', [
      { command: 'add.camera', params: { name: 'Lens', location: [46, -8, 4], rotation: [70, 0, 35] } },
      { command: 'object.setCamera', params: { object: 'Lens' } },
      { command: 'view.camera', params: {} },
    ])
    await gpuDraws('sculpt mode’s mask, face sets, and hidden faces', [
      { command: 'view.camera', params: {} },
      { command: 'world.setImage', params: { clear: true } },
      { command: 'view.shading', params: { mode: 'solid' } },
      { command: 'select.objects', params: { objects: ['Ball'] } },
      { command: 'view.frameSelected', params: {} },
      { command: 'mode.sculpt', params: { on: true } },
      { command: 'sculpt.gesture', params: { action: 'mask', points: [[0, 0], [W * 0.45, H]] } },
      { command: 'sculpt.gesture', params: { action: 'faceSet', points: [[W * 0.55, 0], [W, H]] } },
      { command: 'sculpt.gesture', params: { action: 'hide', points: [[0, 0], [W, H * 0.4]] } },
    ])
    await gpuDraws('a multiresolution level', [
      { command: 'mode.sculpt', params: { on: false } },
      { command: 'select.objects', params: { objects: ['Block'] } },
      { command: 'modifier.add', params: { type: 'multires' } },
      { command: 'multires.subdivide', params: {} },
      { command: 'multires.subdivide', params: {} },
      { command: 'view.frameSelected', params: {} },
      { command: 'mode.sculpt', params: { on: true } },
    ])
    await gpuDraws('weight paint', [
      { command: 'mode.sculpt', params: { on: false } },
      { command: 'select.objects', params: { objects: ['Ball'] } },
      { command: 'view.frameSelected', params: {} },
      { command: 'mode.weightPaint', params: {} },
      { command: 'paint.group', params: { name: 'Glow' } },
      { command: 'paint.vertices', params: { group: 'Glow', vertices: Array.from({ length: 200 }, (_, index) => index), weight: 0.6 } },
    ])
    await gpuDraws('hair', [
      { command: 'mode.weightPaint', params: {} },
      { command: 'particles.add', params: { kind: 'hair', hairLength: 0.4, count: 300 } },
    ])
    await gpuDraws('armature edit mode’s bones', [
      { command: 'add.armature', params: { name: 'Bones', location: [41.5, 0, 2] } },
      { command: 'view.frameSelected', params: {} },
      { command: 'mode.armature', params: {} },
    ])
    await gpuDraws('curve edit mode’s points and handles', [
      { command: 'mode.armature', params: {} },
      { command: 'add.curveBezier', params: { name: 'Track', location: [39, 0, 2] } },
      { command: 'view.frameSelected', params: {} },
      { command: 'mode.shape', params: {} },
    ])
    await page.evaluate(() => window.devHost.connection().ai.run([{ command: 'mode.shape', params: {} }], false))
    await until(async () => (await details()).mode.mode === 'object')
    // The view framed on the cube that draws the bone, set where the bone stands, then hidden.
    await gpuDraws('a custom bone shape’s faces and edges', [
      { command: 'add.cube', params: { name: 'Knot', size: 1, location: [41.5, 0, 2.5] } },
      { command: 'view.frameSelected', params: {} },
      { command: 'view.orbit', params: { tilt: -15 } },
      { command: 'object.hide', params: { objects: ['Knot'] } },
      { command: 'bone.set', params: { object: 'Bones', bone: 'Bone', shape: 'Knot', shapeTranslation: [0, 0.5, 0], shapeRotation: [0, 30, 0], shapeScale: [0.6, 0.6, 0.6] } },
      { command: 'select.objects', params: { objects: ['Bones'] } },
    ])
    await gpuDraws('a wire custom shape in pose mode', [
      { command: 'bone.set', params: { object: 'Bones', bone: 'Bone', shapeWire: true } },
      { command: 'mode.pose', params: { object: 'Bones', on: true } },
    ])
    await page.evaluate(() => window.devHost.connection().ai.run([{ command: 'mode.pose', params: { on: false } }], false))
    await until(async () => (await details()).mode.mode === 'object')
  }

  await kit.getByTestId('scene-save').click()
  await until(async () => (await page.evaluate(() => window.devHost.files.get('Sample.kscene').version)) > 1)
  const head = await page.evaluate(() => Array.from(new Uint8Array(window.devHost.files.get('Sample.kscene').bytes).slice(0, 2)))
  step('saves a .kscene through the host', head[0] === 0x50 && head[1] === 0x4b)
} catch (error) {
  failures += 1
  console.log(`FAIL ${error instanceof Error ? error.stack : String(error)}`)
} finally {
  if (errors.length) console.log(`page errors:\n${errors.slice(0, 8).join('\n')}`)
  await browser.close()
  await server.close()
}
process.exit(failures ? 1 : 0)
