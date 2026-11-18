import { useEffect, useState } from 'react'
import { ImagePlusIcon, TrashIcon } from '@kition/tessera-icons/react'
import { IconButton, useKitEditor, useReadOnly } from '@kition/kit-editor'
import type { SceneInfo, SceneMaterial } from './scene'

/** The image types a material slot takes. */
const IMAGE_FORMATS = ['png', 'jpg', 'jpeg']

export function encodeBase64(bytes: Uint8Array) {
  let binary = ''
  for (let start = 0; start < bytes.length; start += 0x8000) binary += String.fromCharCode(...bytes.subarray(start, start + 0x8000))
  return btoa(binary)
}

/** A thumbnail URL for image `hash`, from the engine's copy of its bytes. */
export function useImageUrl(hash: string | null) {
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
      .invoke<Uint8Array>('imageBytes', hash)
      .then((bytes) => {
        if (!live || !bytes?.length) return
        made = URL.createObjectURL(new Blob([new Uint8Array(bytes)]))
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
 * One image slot of a material: a thumbnail, a button that picks a PNG or JPEG from the
 * workspace (`material.setTexture` with its bytes), and one that takes the image out.
 */
export function TextureSlot({ material, info, slot, label }: { material: SceneMaterial; info: SceneInfo; slot: 'baseColor' | 'metallicRoughness' | 'normal'; label: string }) {
  const { host, run, report, t } = useKitEditor()
  const readOnly = useReadOnly()
  const hash = (slot === 'baseColor' ? material.baseColorTexture : slot === 'normal' ? material.normalTexture : material.metallicRoughnessTexture) ?? null
  const image = hash ? info.images.find((item) => item.hash === hash) : undefined
  const url = useImageUrl(hash)
  const choose = async () => {
    try {
      const path = await host.pick('import', IMAGE_FORMATS)
      if (!path) return
      const { bytes } = await host.files.read(path)
      const name = path.split('/').pop() ?? path
      await run('material.setTexture', { material: material.id, slot, base64: encodeBase64(new Uint8Array(bytes)), name })
    } catch (error) {
      report(t('imageFailed'), error)
    }
  }
  return (
    <div className="scene-texture" data-testid={`scene-texture-${slot}`}>
      <span className="scene-texture-thumb" aria-hidden={!url}>
        {url ? <img src={url} alt="" draggable={false} /> : null}
      </span>
      <span className="scene-texture-name" title={image?.name ?? ''}>
        {image ? image.name : t('noImage')}
      </span>
      <IconButton icon={ImagePlusIcon} label={`${label}: ${t('chooseImage')}`} disabled={readOnly} onClick={() => void choose()} data-testid={`scene-texture-${slot}-choose`} />
      {hash ? (
        <IconButton icon={TrashIcon} label={`${label}: ${t('removeImage')}`} disabled={readOnly} onClick={() => void run('material.setTexture', { material: material.id, slot, clear: true })} data-testid={`scene-texture-${slot}-clear`} />
      ) : null}
    </div>
  )
}
