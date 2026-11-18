// The editor in each display mode (kit-editor's EditorFrame: Menus, Single line, Classic, and
// Focus), through the development host: node tests/modes.mjs [chromium|webkit]
import { chromium, webkit } from 'playwright'
import { createServer } from 'vite'
import { runModeSmoke } from '@kition/kit-dev-host/mode-smoke'

await runModeSmoke({
  kit: 'kit.scene',
  config: new URL('../vite.config.ts', import.meta.url).pathname,
  browser: process.argv[2] === 'webkit' ? webkit : chromium,
  createServer,
  line: { tab: 'Add', primary: ['scene-cmd-add.cube', 'scene-cmd-add.uvSphere', 'scene-cmd-add.cylinder', 'scene-cmd-add.plane', 'scene-cmd-add.light', 'scene-cmd-add.camera'] },
})
