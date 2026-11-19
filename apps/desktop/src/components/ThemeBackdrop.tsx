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
  const image = kind === 'image' && media && !(reduceMotion && media.mediaType === 'image/gif');
  const style = kind === 'image' || kind === 'video' || (preview && kind !== 'gradient')
    ? { '--theme-background-image': image ? `url("${escapeCssUrl(media.url)}")` : 'none' } as CSSProperties
    : undefined;
  return <div className={preview ? 'theme-preview-backdrop' : 'app-theme-backdrop'} aria-hidden="true" style={style}>
    {kind === 'video' && media && !reduceMotion && <BackgroundVideo url={media.url} onError={onError} />}
  </div>;
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
