// The GPU viewport against the software one on a heavy scene, in Chromium at 1440 × 900, DPR 2:
//   node tests/gl-bench.mjs
// Adds a UV sphere of about 100,000 triangles, orbits with 60 middle-button moves, and reports the
// time per picture shown in each mode, and memory. Chromium runs WebGL on the machine's GPU
// (on macOS through ANGLE's Metal backend), as browsers do; headless Chromium otherwise emulates it
// on the CPU (SwiftShader). KITION_SCENE_BENCH_SWIFTSHADER=1 measures that instead.
import { chromium } from 'playwright'
import { createServer } from 'vite'

const server = await createServer({ configFile: new URL('../vite.config.ts', import.meta.url).pathname, server: { port: 0, host: '127.0.0.1' }, logLevel: 'error' })
await server.listen()
const origin = `http://127.0.0.1:${server.httpServer.address().port}`
const browser = await chromium.launch({ args: process.env.KITION_SCENE_BENCH_SWIFTSHADER ? [] : ['--use-angle=metal', '--enable-gpu', '--ignore-gpu-blocklist'] })

async function measure(gpu) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 2 })
  if (!gpu) await context.addInitScript(() => window.localStorage.setItem('kition.scene.gl', 'off'))
  const page = await context.newPage()
  await page.goto(`${origin}/dev.html`)
  await page.waitForFunction(() => document.getElementById('status')?.textContent?.includes(' open in '), null, { timeout: 60000 })
  const frame = page.frames().find((item) => item.url().includes('index.html'))
  await page.evaluate(() => window.devHost.connection().ai.run([{ command: 'object.addUvSphere', params: { segments: 256, rings: 200, radius: 1.5, location: [0, 0, 1] } }]))
  const triangles = (await page.evaluate(async () => (await window.devHost.connection().ai.inspect()).details.objects)).reduce((sum, object) => sum + (object.mesh?.evaluated?.faces ?? 0) * 2, 0)
  await page.waitForTimeout(gpu ? 3000 : 8000)
  // Each animation frame: the GPU frames drawn, or a digest of the software picture's pixels.
  await frame.evaluate(() => {
    const surface = document.querySelector('[data-testid="scene-viewport"] canvas.kui-canvas-surface')
    const context = surface.getContext('2d', { willReadFrequently: true })
    const samples = []
    window.benchSamples = samples
    const tick = (time) => {
      let digest = 0
      if (!window.sceneGl?.active) {
        for (let i = 0; i < 64; i += 1) {
          const [x, y] = [Math.floor(surface.width * ((i % 8) + 0.5) / 8), Math.floor(surface.height * (Math.floor(i / 8) + 0.5) / 8)]
          const pixel = context.getImageData(x, y, 1, 1).data
          digest = (digest * 31 + pixel[0] * 7 + pixel[1] * 3 + pixel[2]) >>> 0
        }
      }
      samples.push({ time, frames: window.sceneGl?.frames ?? 0, cpu: window.sceneGl?.lastFrameMs ?? 0, digest })
      if (window.benchSamples === samples) requestAnimationFrame(tick)
    }
    requestAnimationFrame(tick)
  })
  const box = await frame.locator('[data-testid="scene-viewport"]').boundingBox()
  const [cx, cy] = [box.x + box.width / 2 + 200, box.y + box.height / 2 + 100]
  await page.mouse.move(cx, cy)
  await page.mouse.down({ button: 'middle' })
  const started = Date.now()
  for (let i = 1; i <= 60; i += 1) {
    await page.mouse.move(cx + i * 5, cy + i * 2)
    await page.waitForTimeout(16)
  }
  const dragMs = Date.now() - started
  await page.mouse.up({ button: 'middle' })
  // Software pictures land after the drag too: wait for them to stop changing.
  await page.waitForTimeout(gpu ? 500 : 6000)
  const samples = await frame.evaluate(() => {
    const out = window.benchSamples
    window.benchSamples = null
    return out
  })
  const memory = await frame.evaluate(() => performance.memory?.usedJSHeapSize ?? null)
  const renderer = await frame.evaluate(() => {
    const gl = document.createElement('canvas').getContext('webgl2')
    const ext = gl?.getExtension('WEBGL_debug_renderer_info')
    return ext ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) : null
  })
  const probe = await frame.evaluate(() => window.sceneGl ?? null)
  const gpuMsPerFrame = gpu ? await frame.evaluate(() => window.sceneGl.bench(60)) : null
  let pictures = 0
  let cpu = 0
  for (let i = 1; i < samples.length; i += 1) {
    if (gpu ? samples[i].frames !== samples[i - 1].frames : samples[i].digest !== samples[i - 1].digest) {
      pictures += 1
      cpu += samples[i].cpu
    }
  }
  const first = samples[0].time
  const last = [...samples].reverse().find((sample, index, all) => index + 1 < all.length && (gpu ? sample.frames !== all[index + 1].frames : sample.digest !== all[index + 1].digest))?.time ?? samples.at(-1).time
  await context.close()
  return { gpu, renderer, triangles, dragMs, pictures, msPerPicture: pictures ? (last - first) / pictures : null, cpuMsPerGpuFrame: gpu && pictures ? cpu / pictures : null, gpuMsPerFrame, heapMB: memory ? memory / 1048576 : null, probe }
}

const results = [await measure(true), await measure(false)]
for (const result of results) console.log(JSON.stringify(result))
await browser.close()
await server.close()
