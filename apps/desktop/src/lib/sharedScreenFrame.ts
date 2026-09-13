// Keep this transport envelope in sync with core/shared_desktop.rs.
export const MAX_SHARED_SCREEN_BASE64 = 1_400_000;
const MAX_EDGE = 1568;

/** Preserve legibility first, then reduce resolution for high-entropy frames. */
export function encodeSharedScreenFrame(
  source: CanvasImageSource,
  width: number,
  height: number,
  canvas: HTMLCanvasElement,
): { url: string; base64: string } | null {
  if (!Number.isFinite(width) || !Number.isFinite(height) || width < 1 || height < 1) return null;
  const context = canvas.getContext('2d');
  if (!context) return null;
  const scale = Math.min(1, MAX_EDGE / Math.max(width, height));
  for (const resolution of [1, 0.75, 0.5]) {
    canvas.width = Math.max(1, Math.round(width * scale * resolution));
    canvas.height = Math.max(1, Math.round(height * scale * resolution));
    context.drawImage(source, 0, 0, canvas.width, canvas.height);
    for (const quality of [0.65, 0.5, 0.35]) {
      const url = canvas.toDataURL('image/jpeg', quality);
      const prefix = 'data:image/jpeg;base64,';
      if (!url.startsWith(prefix)) continue;
      const base64 = url.slice(prefix.length);
      if (base64.length > 0 && base64.length <= MAX_SHARED_SCREEN_BASE64) return { url, base64 };
    }
  }
  return null;
}
