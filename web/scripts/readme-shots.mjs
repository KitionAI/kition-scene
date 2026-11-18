// web/scripts/readme-shots.mjs: the README pictures of Kition Scene (assets/readme/), taken with
// kition-sdk's shared runner (tools/readme-shots). Every shot opens the showcase sample "Glazed
// Donut with Sprinkles" in the development host, in Classic display mode.
//
//   node scripts/readme-shots.mjs [name ...]     # from web/, with the engine built in public/
//
// The viewport draws on the GPU (WebGL2). Headless Chromium has none, so point CHROME at a browser
// started with a WebGL implementation (docs/development.md shows a wrapper); without one the
// viewport steps aside for the engine's software viewport. The sample comes from a
// kition-showcase checkout next to this repository (or the one KITION_SHOWCASE names); kition-sdk
// likewise (or KITION_SDK). Run it with no HTTP proxy set. When pngquant is installed, each picture
// is then shrunk with --quality 70-90 --nofs.
import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const web = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const workspace = path.resolve(web, '..', '..')
const sdk = path.resolve(process.env.KITION_SDK ?? path.join(workspace, 'kition-sdk'))
const showcase = path.resolve(process.env.KITION_SHOWCASE ?? path.join(workspace, 'kition-showcase'))
const { takeShots, sizeLines, sizeReport } = await import(pathToFileURL(path.join(sdk, 'tools/readme-shots/shots.mjs')).href)

const sample = path.join(showcase, 'scene', 'glazed-donut-sprinkles', 'glazed-donut-sprinkles.kscene')

/** Waits until `check` answers true (every 100 ms, `tries` times). */
const until = async (page, check, tries = 80) => {
  for (let i = 0; i < tries; i += 1) {
    if (await check().catch(() => false)) return true
    await page.waitForTimeout(100)
  }
  return false
}

/** Picks a viewport shading (Wireframe, Solid, Material, Rendered) in the status bar's switch. */
const shading = (kit, label) => kit.locator('xpath=//*[normalize-space(text())="Wireframe"]/ancestor::*[.//*[normalize-space(text())="Rendered"]][1]').getByText(label, { exact: true }).click()

/** Selects an object by clicking its row in the outliner (shift adds to the selection). */
const pick = (kit, name, modifiers = []) => kit.locator(`[data-testid="scene-outliner-item"][data-name="${name}"] button`).first().click({ modifiers })

/** The scene as the host's agent connection inspects it. */
const details = async (page) => (await page.evaluate(async () => window.devHost.connection().ai.inspect())).details

const report = await takeShots({
  web,
  sample,
  query: { modes: 'classic,line,menus,focus' },
  ready: '[data-testid="scene-viewport"] canvas',
  settle: { open: 4000 },
  shots: [
    {
      // The whole editor on the donut, in Material shading.
      name: 'hero',
      steps: async (page, { kit, frame }) => {
        const gl = await frame.evaluate(() => window.sceneGl ?? null)
        if (!gl?.active) console.log(`hero: the GPU viewport is off (${gl?.reason ?? 'no WebGL2'}); the software viewport draws instead`)
        await shading(kit, 'Material')
        await page.waitForTimeout(2000)
      },
    },
    {
      // Rendered shading: the viewport traces the scene's lighting progressively.
      name: 'see-1',
      steps: async (page, { kit }) => {
        await shading(kit, 'Rendered')
        // Until every sample is in (the badge reads "Samples 32 / 32", then goes), at most 3 minutes.
        await page.waitForTimeout(3000)
        await until(page, async () => !/Samples \d+ \/ \d+/.test(await kit.getByTestId('scene-viewport').innerText()) || /Samples (\d+) \/ \1\b/.test(await kit.getByTestId('scene-viewport').innerText()), 1800)
        await page.waitForTimeout(1500)
      },
    },
    {
      // The outliner and the properties of the selected icing: transform, modifiers, material.
      name: 'see-2',
      steps: async (page, { kit }) => {
        await shading(kit, 'Material')
        await pick(kit, 'Icing')
        await until(page, async () => (await details(page)).selection.active === (await details(page)).objects.find((o) => o.name === 'Icing').id)
        await page.waitForTimeout(1500)
      },
    },
    {
      // Materials: the icing's material, with its base color open in the color picker.
      name: 'see-3',
      steps: async (page, { kit }) => {
        await shading(kit, 'Material')
        await pick(kit, 'Icing')
        await kit.getByTestId('scene-material').scrollIntoViewIfNeeded()
        await kit.getByTestId('scene-base-color').click()
        await kit.getByTestId('scene-base-color-picker').waitFor()
        await page.waitForTimeout(1000)
      },
    },
    {
      // Animation: the dough and the icing keyed on frame 1, lifted and tilted and keyed on frame
      // 120, shown at frame 50 with the graph editor's curves.
      name: 'see-4',
      steps: async (page, { kit }) => {
        const animation = async () => (await details(page)).animation
        await shading(kit, 'Material')
        await pick(kit, 'Dough')
        await pick(kit, 'Icing', ['Shift'])
        await until(page, async () => (await details(page)).selection.objects?.length === 2 || true)
        await page.waitForTimeout(500)
        await kit.getByTestId('scene-timeline-insert').click()
        await until(page, async () => (await animation()).keyFrames.includes(1))
        const frameField = kit.getByTestId('scene-timeline-frame').locator('input')
        const goTo = async (frame) => {
          await frameField.fill(String(frame))
          await frameField.press('Enter')
          await until(page, async () => (await animation()).frame === frame)
        }
        await goTo(120)
        const field = async (id, value) => {
          const input = kit.getByTestId(id).locator('input')
          await input.fill(String(value))
          await input.press('Enter')
          await page.waitForTimeout(400)
        }
        await field('scene-location-z', 1.6)
        await field('scene-rotation-x', 24)
        await kit.getByTestId('scene-timeline-insert').click()
        await until(page, async () => (await animation()).keyFrames.includes(120))
        await goTo(50)
        await kit.getByTestId('scene-graph-toggle').check()
        await page.waitForTimeout(2000)
      },
    },
  ],
})

// Shrink each picture when pngquant is there (the README's are at most 400 KB each). No dithering
// (--nofs): the path-traced picture's noise defeats it otherwise.
let shrunk = false
for (const { file } of report) {
  try {
    execFileSync('pngquant', ['--quality', '70-90', '--nofs', '--force', '--skip-if-larger', '--ext', '.png', path.resolve(web, '..', file)], { stdio: 'ignore' })
    shrunk = true
  } catch (error) {
    if (error.code === 'ENOENT') break
  }
}
if (shrunk) {
  console.log('after pngquant --quality 70-90 --nofs:')
  for (const line of sizeLines(sizeReport(report.map(({ file }) => ({ file, bytes: fs.statSync(path.resolve(web, '..', file)).size }))))) console.log(line)
}
