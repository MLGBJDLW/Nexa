import { useState } from 'react';
import { Globe } from 'lucide-react';

function siteIconUrl(source?: string): string | undefined {
  if (!source) return;
  try {
    const url = new URL(source);
    const host = url.hostname.toLowerCase().replace(/\.$/, '');
    // Packaged WebViews permit HTTPS images. Keep local, credentialed and
    // nonstandard-port references as links without issuing an image request.
    if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password || url.port
      || !host.includes('.') || /^[\d.]+$/.test(host) || host.includes(':')
      || /\.(localhost|local|internal|lan|home|test|invalid|onion)$/.test(host)
      || host.endsWith('.home.arpa')) return;
    return `https://${host}/favicon.ico`;
  } catch {
    return;
  }
}

export function WebSourceIcon({ source, className }: { source?: string; className: string }) {
  const src = siteIconUrl(source);
  const [failed, setFailed] = useState<string>();
  if (!src || src === failed) return <Globe aria-hidden="true" className={className} />;
  return <img aria-hidden="true" alt="" className={`${className} object-contain`} width={14} height={14}
    src={src} loading="lazy" decoding="async" referrerPolicy="no-referrer" onError={() => setFailed(src)} />;
}
