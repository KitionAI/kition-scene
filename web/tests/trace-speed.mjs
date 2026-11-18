// How much faster path tracing gets across workers: node tests/trace-speed.mjs [chromium|webkit] [samples]
// Renders the starter scene at 960 × 540 in the render dialog with 1, 4, and 8 workers (the
// first is the engine's own tracer) and prints each render's time and its speedup over one.
import { chromium, webkit } from 'playwright'
import { createServer } from 'vite'

const which = process.argv[2] ?? 'chromium'
const samples = Number(process.argv[3] ?? 16)
const server = await createServer({ configFile: new URL('../vite.config.ts', import.meta.url).pathname, server: { port: 0, host: '127.0.0.1' }, logLevel: 'error' })
await server.listen()
const origin = `http://127.0.0.1:${server.httpServer.address().port}`
const browser = await (which === 'webkit' ? webkit : chromium).launch()
const page = await (await browser.newContext({ viewport: { width: 1440, height: 900 } })).newPage()
const until = async (check, tries) => {
  for (let i = 0; i < tries; i += 1) {
    if (await check().catch(() => false)) return true
    await page.waitForTimeout(100)
  }
  return false
}

try {
  await page.goto(`${origin}/dev.html`)
  await page.waitForFunction(() => document.getElementById('status')?.textContent?.includes(' open in '), null, { timeout: 60000 })
  const kit = page.frameLocator('#kit')
  const cores = await page.evaluate(() => navigator.hardwareConcurrency)
  await page.evaluate(() => window.devHost.connection().ai.run([{ command: 'render.set', params: { width: 960, height: 540 } }]))
  await kit.getByTestId('scene-render').click()
  await until(async () => (await kit.getByTestId('scene-render-image').count()) === 1, 300)
  await kit.getByTestId('scene-render-engine-path').click()
  if ((await kit.getByTestId('scene-render-cancel').count()) === 1) await kit.getByTestId('scene-render-cancel').click()
  const samplesField = kit.getByTestId('scene-render-samples').locator('input')
  await samplesField.fill(String(samples))
  await samplesField.press('Enter')
  const progress = kit.getByTestId('scene-render-progress')
  const times = {}
  for (const workers of [1, 4, 8]) {
    const field = kit.getByTestId('scene-render-workers').locator('input')
    await field.fill(String(workers))
    await field.press('Enter')
    await kit.getByTestId('scene-render-start').click()
    const done = await until(async () => (await progress.getAttribute('data-state')) === 'done' && Number(await progress.getAttribute('data-workers')) === workers, 6000)
    if (!done) throw new Error(`the render with ${workers} workers did not finish`)
    times[workers] = Number(await progress.getAttribute('data-elapsed-ms'))
    console.log(`${which}, ${cores} cores, 960 × 540, ${samples} samples, ${workers} worker${workers === 1 ? '' : 's'}: ${times[workers]} ms, ${(times[1] / times[workers]).toFixed(2)}× one`)
  }
} finally {
  await browser.close()
  await server.close()
}
