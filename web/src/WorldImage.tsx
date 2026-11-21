import { useEffect, useState } from 'react'
import { ImagePlusIcon, TrashIcon } from '@kition/tessera/icons/react'
import { IconButton, useKitEditor, useReadOnly } from '@kition/sdk/editor'
import { encodeBase64 } from './TextureSlot'
import type { SceneInfo } from './scene'

/** The image types a world takes: equirectangular HDR, PNG, or JPEG. */
const WORLD_FORMATS = ['hdr', 'png', 'jpg', 'jpeg']

/** A small PNG of image `hash` made by the engine (HDR images clipped to the screen's range). */
function usePreviewUrl(hash: string | null) {
  const { session } = useKitEditor()
  const doc = session.doc
  const [url, setUrl] = useState<string | null>(null)
  useEffect(() => {
    if (!hash || !doc) {
      setUrl(null)
      return
    }
    let made: string | null = null
    let live = true
    void doc
      .invoke<Uint8Array>('imagePreview', hash, 128)
      .then((bytes) => {
        if (!live || !bytes?.length) return
        made = URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type: 'image/png' }))
        setUrl(made)
      })
      .catch(() => setUrl(null))
    return () => {
      live = false
      if (made) URL.revokeObjectURL(made)
    }
  }, [doc, hash])
  return url
}

/**
 * The world image: a thumbnail, a button that picks an equirectangular image from the workspace
 * (`world.setImage` with its bytes), and one that takes it out (the world colour shows again).
 */
export function WorldImageSlot({ info }: { info: SceneInfo }) {
  const { host, run, report, t } = useKitEditor()
  const readOnly = useReadOnly()
  const hash = info.world.image
  const image = hash ? info.images.find((item) => item.hash === hash) : undefined
  const url = usePreviewUrl(hash)
  const choose = async () => {
    try {
      const path = await host.pick('import', WORLD_FORMATS)
      if (!path) return
      const { bytes } = await host.files.read(path)
      const name = path.split('/').pop() ?? path
      await run('world.setImage', { base64: encodeBase64(new Uint8Array(bytes)), name })
    } catch (error) {
      report(t('imageFailed'), error)
    }
  }
  return (
    <div className="scene-texture scene-world-image" data-testid="scene-world-image">
      <span className="scene-texture-thumb" aria-hidden={!url}>
        {url ? <img src={url} alt="" draggable={false} /> : null}
      </span>
      <span className="scene-texture-name" title={image?.name ?? ''}>
        {image ? image.name : t('noWorldImage')}
      </span>
      <IconButton icon={ImagePlusIcon} label={`${t('worldImage')}: ${t('chooseImage')}`} disabled={readOnly} onClick={() => void choose()} data-testid="scene-world-image-choose" />
      {hash ? (
        <IconButton icon={TrashIcon} label={`${t('worldImage')}: ${t('removeImage')}`} disabled={readOnly} onClick={() => void run('world.setImage', { clear: true })} data-testid="scene-world-image-clear" />
      ) : null}
    </div>
  )
}
