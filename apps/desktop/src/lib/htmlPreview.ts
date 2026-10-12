import DOMPurify from 'dompurify';
import { MAX_UI_BYTES } from './chatUi';

export function staticHtmlPreview(source: string): string {
  if (source.length > MAX_UI_BYTES || new TextEncoder().encode(source).length > MAX_UI_BYTES) throw new Error('HTML preview exceeds 512 KiB');
  const html = DOMPurify.sanitize(source, {
    WHOLE_DOCUMENT: true,
    ADD_TAGS: ['style'],
    FORBID_TAGS: ['script', 'iframe', 'object', 'embed', 'base', 'link', 'meta', 'form'],
    FORBID_ATTR: ['href', 'xlink:href', 'srcdoc', 'srcset', 'action', 'formaction', 'target', 'autofocus'],
    ALLOWED_URI_REGEXP: /^data:image\/(?:png|jpeg|gif|webp);base64,[a-z0-9+/=\s]+$/i,
  });
  const policy = "default-src 'none'; script-src 'none'; style-src 'unsafe-inline'; img-src data:; font-src data:; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";
  return html.replace(/<head>/i, `<head><meta http-equiv="Content-Security-Policy" content="${policy}"><meta name="viewport" content="width=device-width,initial-scale=1"><style>html{color-scheme:light}body{margin:16px;font:14px/1.5 system-ui,sans-serif;overflow-wrap:anywhere}img,svg,canvas{max-width:100%}*{box-sizing:border-box}@media(prefers-reduced-motion:reduce){*,*::before,*::after{animation:none!important;transition:none!important}}</style>`);
}
