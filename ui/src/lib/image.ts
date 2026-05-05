/**
 * Image / blob utilities shared across the composer's clipboard +
 * file-picker + drag-drop paths. Pure helpers; no Vue, no Tauri.
 */

/** RGBA pixel buffer → PNG blob via offscreen canvas. */
export async function rgbaToPngBlob(rgba: Uint8Array, width: number, height: number): Promise<Blob | undefined> {
  if (width === 0 || height === 0) {
    return undefined
  }
  const canvas = document.createElement('canvas')

  canvas.width = width
  canvas.height = height
  const ctx = canvas.getContext('2d')

  if (!ctx) {
    return undefined
  }
  // Copy into a fresh `Uint8ClampedArray` (own ArrayBuffer) so the
  // TS lib's `ImageDataArray` parameter type accepts it — recent
  // lib.dom.d.ts narrows to `Uint8ClampedArray<ArrayBuffer>`, while
  // the view-of-rgba.buffer reads as `ArrayBufferLike` (which the
  // SharedArrayBuffer branch rejects).
  const data = new Uint8ClampedArray(rgba.byteLength)

  data.set(new Uint8ClampedArray(rgba.buffer, rgba.byteOffset, rgba.byteLength))
  ctx.putImageData(new ImageData(data, width, height), 0, 0)

  return new Promise((resolve) => {
    canvas.toBlob((blob) => resolve(blob ?? undefined), 'image/png')
  })
}

/** FileReader-based base64 dataURL — async, off the main thread. */
export function blobToDataUrl(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const r = new FileReader()

    r.onload = () => resolve(r.result as string)
    r.onerror = () => reject(r.error)
    r.readAsDataURL(blob)
  })
}

/** Compact "MB / KB / B" size label for attachment chrome. */
export function formatSize(bytes: number): string {
  if (bytes >= 1024 * 1024) {
    return `${(bytes / (1024 * 1024)).toFixed(1)}MB`
  }

  if (bytes >= 1024) {
    return `${(bytes / 1024).toFixed(1)}KB`
  }

  return `${Math.round(bytes)}B`
}
