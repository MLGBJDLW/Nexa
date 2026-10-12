import { useEffect, useMemo, useRef, useState } from 'react';
import { Code2, Maximize2, Minimize2, Play, Square } from 'lucide-react';
import { useFilePreview } from '../../features/preview/filePreviewContext';
import type { HtmlPreviewLease } from '../../features/preview/filePreviewContext';
import { staticHtmlPreview } from '../../lib/htmlPreview';
import { useTranslation } from '../../i18n';

/** Static streaming is inert. Explicit Run creates a revocable sandbox lease;
 * partial code, edits, unmounts, and stale async responses never keep it alive. */
export function HtmlPreviewFrame({ source, streaming = false, showSource = true }: { source: string; streaming?: boolean; showSource?: boolean }) {
  const { t } = useTranslation(); const { createHtmlPreview } = useFilePreview();
  const [presented, setPresented] = useState(source); const latest = useRef(source);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [lease, setLease] = useState<(HtmlPreviewLease & { source: string }) | null>(null);
  const liveLease = useRef<HtmlPreviewLease | null>(null);
  const generation = useRef(0); const [loading, setLoading] = useState(false);
  const [error, setError] = useState(''); const [expanded, setExpanded] = useState(false);
  latest.current = source;
  useEffect(() => {
    if (!streaming) { if (timer.current) clearTimeout(timer.current); timer.current = null; setPresented(source); }
    else if (!timer.current) timer.current = setTimeout(() => { timer.current = null; setPresented(latest.current); }, 180);
  }, [source, streaming]);
  useEffect(() => () => { if (timer.current) clearTimeout(timer.current); }, []);
  useEffect(() => {
    generation.current++; setLease(null); setLoading(false); setError('');
    liveLease.current?.release(); liveLease.current = null;
    return () => { generation.current++; liveLease.current?.release(); liveLease.current = null; };
  }, [source, streaming, createHtmlPreview]);
  const preview = useMemo(() => {
    try { return { html: staticHtmlPreview(presented) }; }
    catch (error) { return { error: String(error) }; }
  }, [presented]);
  const running = !streaming && lease?.source === source ? lease : null;
  const stop = () => { generation.current++; liveLease.current?.release(); liveLease.current = null; setLease(null); setLoading(false); };
  async function run() {
    if (!createHtmlPreview || streaming || preview.error) return;
    stop(); const mine = ++generation.current; setLoading(true); setError('');
    try {
      const next = await createHtmlPreview(source);
      if (mine !== generation.current) { next.release(); return; }
      liveLease.current = next; setLease({ ...next, source });
    } catch (error) { if (mine === generation.current) setError(String(error)); }
    finally { if (mine === generation.current) setLoading(false); }
  }
  return <section className="my-3 min-w-0 overflow-hidden rounded-xl border border-border bg-surface-1" data-testid="html-preview">
    <header className="flex flex-wrap items-center gap-2 border-b border-border px-3 py-2 text-xs">
      <Code2 size={14} className="text-accent" aria-hidden="true" /><span className="mr-auto font-medium">{t('chat.htmlPreview')}</span>
      <span className="text-[10px] text-text-tertiary">{streaming ? t('chat.visualBuilding') : running ? t('chat.htmlRunning') : t('chat.htmlStatic')}</span>
      {running || loading ? <button type="button" onClick={stop} className="flex items-center gap-1 rounded px-2 py-1 hover:bg-surface-3"><Square size={12} />{t('chat.htmlStop')}</button>
        : createHtmlPreview && <button type="button" onClick={() => void run()} disabled={streaming || !!preview.error} className="flex items-center gap-1 rounded px-2 py-1 text-accent hover:bg-surface-3 disabled:opacity-40"><Play size={12} />{t('chat.htmlRun')}</button>}
      <button type="button" onClick={() => setExpanded(value => !value)} aria-label={t(expanded ? 'chat.visualCollapse' : 'chat.visualExpand')} aria-pressed={expanded} className="rounded p-1 hover:bg-surface-3">{expanded ? <Minimize2 size={14} /> : <Maximize2 size={14} />}</button>
    </header>
    {(error || preview.error) && <p role="alert" className="px-3 py-2 text-xs text-danger">{error || preview.error}</p>}
    {!preview.error && <iframe key={running?.url ?? 'static'} src={running?.url} srcDoc={running ? undefined : preview.html}
      sandbox={running ? 'allow-scripts' : ''} referrerPolicy="no-referrer" title={t('chat.htmlPreview')}
      className={`block w-full border-0 bg-white ${expanded ? 'h-[min(75vh,800px)]' : 'h-[360px]'}`} />}
    {showSource && <details className="border-t border-border px-3 py-2 text-xs text-text-tertiary"><summary className="cursor-pointer">{t('chat.visualSource')}</summary><pre className="max-h-72 overflow-auto py-2"><code>{source}</code></pre></details>}
  </section>;
}
