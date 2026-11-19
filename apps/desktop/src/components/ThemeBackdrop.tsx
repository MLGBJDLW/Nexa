import { useEffect, useRef, useState, useSyncExternalStore, type CSSProperties } from 'react';
import { escapeCssUrl, type CustomThemeDefinition } from '../lib/themeProfile';
import { useReducedMotionPreference } from '../lib/uiMotion';

type BackgroundMedia = { url: string; mediaType: string };
export type ResolveThemeMedia = (assetId: string) => Promise<BackgroundMedia>;

function subscribeToVisibility(notify: () => void) {
  document.addEventListener('visibilitychange', notify);
  return () => document.removeEventListener('visibilitychange', notify);
}

export function ThemeBackdrop({ theme, resolve, preview = false, onError }: {
  theme?: CustomThemeDefinition;
  resolve: ResolveThemeMedia;
  preview?: boolean;
  onError?: () => void;
}) {
  const visible = useSyncExternalStore(subscribeToVisibility, () => !document.hidden, () => false);
  const reduceMotion = useReducedMotionPreference() || theme?.motion.durationScale === 0;
  const kind = theme?.background.kind;
  const assetId = kind === 'image' || kind === 'video' ? theme?.background.assetId : undefined;
  const showMedia = visible && (kind !== 'video' || !reduceMotion);
  const [resolved, setResolved] = useState<(BackgroundMedia & { id: string; owner: ResolveThemeMedia }) | null>(null);
  useEffect(() => {
    let current = true;
    if (resolved) {
      if (resolved.id !== assetId || resolved.owner !== resolve || !resolved.mediaType.startsWith(kind + '/')) setResolved(null);
      return;
    }
    if (assetId && showMedia) void resolve(assetId).then(media => {
      if (!current) return;
      if (!media.mediaType.startsWith(kind + '/')) throw new Error('Theme background type does not match its declaration');
      setResolved({ ...media, id: assetId, owner: resolve });
    }).catch(error => {
      if (!current) return;
      console.warn('Unable to resolve managed theme background', error);
      onError?.();
    });
    return () => { current = false; };
  }, [assetId, kind, resolve, onError, showMedia, resolved]);
  const media = showMedia && resolved?.id === assetId && resolved?.owner === resolve && resolved.mediaType.startsWith(kind + '/') ? resolved : null;
  const imageUrl = kind === 'image' ? media?.url : undefined;
  // JPEG is always static. PNG (APNG), WebP and GIF can contain an animation.
  const freeze = Boolean(imageUrl && reduceMotion && media?.mediaType !== 'image/jpeg');
  const stillUrl = useStillThemeImage(freeze ? imageUrl : undefined);
  const displayedImage = freeze ? stillUrl : imageUrl;
  const style = kind === 'image' || kind === 'video' || (preview && kind !== 'gradient')
    ? { '--theme-background-image': displayedImage ? `url("${escapeCssUrl(displayedImage)}")` : 'none' } as CSSProperties
    : undefined;
  return <div className={preview ? 'theme-preview-backdrop' : 'app-theme-backdrop'} aria-hidden="true" style={style}>
    {kind === 'video' && media && !reduceMotion && <BackgroundVideo url={media.url} onError={onError} />}
  </div>;
}

function useStillThemeImage(source?: string) {
  const [frame, setFrame] = useState<{ source: string; url: string } | null>(null);
  useEffect(() => {
    if (!source) { setFrame(null); return; }
    let disposed = false;
    let url: string | undefined;
    const image = new Image();
    image.crossOrigin = 'anonymous';
    image.src = source;
    void image.decode().then(() => {
      if (disposed) return;
      // drawImage uses the default still image or first animation frame. Bound
      // the extra bitmap to the display size even for very large source images.
      const scale = Math.min(1, Math.max(window.innerWidth, window.innerHeight) * window.devicePixelRatio /
        Math.max(image.naturalWidth, image.naturalHeight));
      const canvas = document.createElement('canvas');
      canvas.width = Math.max(1, Math.round(image.naturalWidth * scale));
      canvas.height = Math.max(1, Math.round(image.naturalHeight * scale));
      const context = canvas.getContext('2d');
      if (!context) return;
      context.drawImage(image, 0, 0, canvas.width, canvas.height);
      image.removeAttribute('src');
      canvas.toBlob(blob => {
        canvas.width = canvas.height = 0;
        if (disposed || !blob) return;
        url = URL.createObjectURL(blob);
        setFrame({ source, url });
      });
    }).catch(() => { /* Keep the theme palette if a static frame cannot be decoded. */ });
    return () => {
      disposed = true;
      image.removeAttribute('src');
      if (url) URL.revokeObjectURL(url);
    };
  }, [source]);
  return frame?.source === source ? frame?.url : undefined;
}

function BackgroundVideo({ url, onError }: { url: string; onError?: () => void }) {
  const ref = useRef<HTMLVideoElement>(null);
  useEffect(() => {
    const video = ref.current;
    if (!video) return;
    video.src = url;
    return () => {
      video.pause();
      video.removeAttribute('src');
      video.load();
    };
  }, [url]);
  return <>
    <video ref={ref} autoPlay loop muted playsInline disablePictureInPicture disableRemotePlayback preload="metadata" onError={onError} />
    <div className="theme-video-dim" />
  </>;
}
