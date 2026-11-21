import type { KitModuleManifest } from '@kition/sdk/editor'

/** The engine, from this package's own folder. */
export function engineManifest(): KitModuleManifest {
  return {
    kitId: 'kit.scene',
    base: new URL('./engine/', document.baseURI).href,
    module: 'kition_scene_kit',
    documentClass: 'SceneDocument',
  }
}
