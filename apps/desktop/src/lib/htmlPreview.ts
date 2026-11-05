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
  const document = new DOMParser().parseFromString(html, 'text/html');
  const csp = document.createElement('meta');
  csp.httpEquiv = 'Content-Security-Policy';
  csp.content = policy;
  const viewport = document.createElement('meta');
  viewport.name = 'viewport';
  viewport.content = 'width=device-width,initial-scale=1';
  const style = document.createElement('style');
  style.textContent = 'html{color-scheme:light}body{margin:16px;font:14px/1.5 system-ui,sans-serif;overflow-wrap:anywhere}img,svg,canvas{max-width:100%}*{box-sizing:border-box}@media(prefers-reduced-motion:reduce){*,*::before,*::after{animation:none!important;transition:none!important}}';
  // Insert before generated styles regardless of head attributes or source
  // formatting. The static frame must block resources before any opt-in run.
  document.head.prepend(csp, viewport, style);
  // Generated desktop layouts often carry fixed pixel widths. Keep the static
  // mobile document inside its own viewport without modifying source or scripts.
  const mobile = document.createElement('style');
  mobile.textContent = '@media(max-width:900px){html,body{min-width:0!important;max-width:100%!important}body{width:auto!important;margin:12px!important}body *{box-sizing:border-box;min-width:0!important;max-width:100%!important}img,svg,canvas,video{height:auto!important}pre{white-space:pre;overflow:auto}table{display:block;overflow-x:auto}th,td{overflow-wrap:break-word}[style*="grid-template-columns"]{grid-template-columns:minmax(0,1fr)!important}[style*="display:flex"],[style*="display: flex"]{flex-wrap:wrap}}';
  document.head.append(mobile);
  return '<!doctype html>' + document.documentElement.outerHTML;
}
