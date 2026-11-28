import { useRef, useState } from 'react';
import { Loader2, WandSparkles } from 'lucide-react';
import { useTranslation } from '../../i18n';
import { suggestedPrompt } from '../../lib/portableChatCommands';

export function PromptUpgrade({draft, model, refine, onApply}: {
  draft: string; model?: string; refine?: (draft: string) => Promise<string>; onApply: (next: string) => void;
}) {
  const {t} = useTranslation();
  const [preview, setPreview] = useState<{source: string; text?: string; error?: string} | null>(null);
  const inFlight = useRef(false);
  const [pending, setPending] = useState(false);
  const active = preview?.source === draft ? preview : null;
  const local = refine ? null : suggestedPrompt(draft);
  if (!draft.trim() || draft.startsWith('/') || (!refine && !local)) return null;
  async function show() {
    if (inFlight.current) return;
    if (active) { setPreview(null); return; }
    inFlight.current = true;
    setPending(true);
    setPreview({source:draft});
    try {
      const text = refine ? await refine(draft) : local!;
      setPreview({source:draft, text});
    } catch (cause) {
      setPreview({source:draft, error:String(cause)});
    } finally {
      inFlight.current = false;
      setPending(false);
    }
  }
  return <div className="min-w-0 px-2 text-xs">
    <button type="button" disabled={pending} aria-expanded={!!active} className="inline-flex min-h-9 max-w-full items-center gap-2 text-text-secondary"
      onClick={() => void show()}><WandSparkles size={15}/>{t('remote.improvePrompt')}{refine && model && <span className="truncate">· {model}</span>}</button>
    {active && <div className="max-h-52 overflow-auto rounded-lg border border-border p-3">
      {active.error ? <p role="alert">{active.error}</p> : active.text ? <>
        <p className="whitespace-pre-wrap break-words">{active.text}</p>
        <button type="button" className="mt-2 min-h-9 font-medium text-accent" onClick={() => {
          setPreview(null); onApply(active.text!);
        }}>{t('remote.usePrompt')}</button>
      </> : <span role="status" className="flex items-center gap-2"><Loader2 size={14} className="animate-spin"/>{t('common.loading')}</span>}
    </div>}
  </div>;
}
