import { CheckCircle2, Globe, Loader2, Terminal, XCircle } from 'lucide-react';
import { toast } from 'sonner';
import { useTranslation } from '../../i18n';
import { openNexaBrowser } from '../../features/browser/openNexaBrowser';
import type { ManagedProcessPreview } from '../../lib/processArtifacts';

export function ManagedProcessCard({ process, active = false }: { process: ManagedProcessPreview; active?: boolean }) {
  const { t } = useTranslation();
  const failed = process.state === 'failed' || process.state === 'unhealthy';
  const Icon = failed ? XCircle : process.state === 'ready' || process.state === 'completed' ? CheckCircle2
    : active ? Loader2 : Terminal;
  return (
    <section data-testid="managed-process-card" data-process-state={process.state}
      className="my-2 min-w-0 max-w-full overflow-hidden rounded-lg border border-border/60 bg-surface-1/65 p-2.5 sm:max-w-[36rem]">
      <div className="flex min-w-0 flex-wrap items-center gap-2 text-xs" role="status">
        <Icon className={`h-3.5 w-3.5 shrink-0 ${failed ? 'text-danger' : 'text-accent'} ${active && process.state === 'running' ? 'motion-safe:animate-spin' : ''}`} />
        <span className="min-w-0 flex-1 truncate font-medium text-text-primary" title={process.program}>{process.program}</span>
        <span className="text-text-secondary">{t(active ? 'chat.processLive' : 'chat.processLastReported')} · {t(`chat.processState.${process.state}`)}</span>
      </div>
      {process.readyUrl && (
        <button type="button" className="mt-2 flex max-w-full items-center gap-2 rounded-md border border-accent/25 bg-accent/10 px-2 py-1.5 text-xs text-accent hover:bg-accent/20"
          onClick={() => { if (!openNexaBrowser(process.readyUrl!)) toast.error(t('browser.openFailed')); }}
          title={process.readyUrl}>
          <Globe className="h-3.5 w-3.5 shrink-0" /><span className="shrink-0">{t('chat.processOpenBrowser')}</span>
          <span className="min-w-0 truncate">{process.readyUrl}</span>
        </button>
      )}
      {process.output && <pre data-testid="managed-process-output" className="mt-2 max-h-40 overflow-auto whitespace-pre-wrap break-all rounded bg-surface-0/70 p-2 font-mono text-[11px] leading-relaxed text-text-secondary">{process.output}</pre>}
    </section>
  );
}
