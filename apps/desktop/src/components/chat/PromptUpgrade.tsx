import { useState } from 'react';
import { WandSparkles } from 'lucide-react';
import { useTranslation } from '../../i18n';
import { suggestedPrompt } from '../../lib/portableChatCommands';

export function PromptUpgrade({draft, onApply}: {draft: string; onApply: (next: string) => void}) {
  const {t} = useTranslation();
  const [previewFor, setPreviewFor] = useState<string | null>(null);
  const suggestion = suggestedPrompt(draft);
  if (!suggestion) return null;
  return <div className="min-w-0 px-2 text-xs">
    <button type="button" aria-expanded={previewFor === draft} className="inline-flex min-h-9 items-center gap-2 text-text-secondary"
      onClick={() => setPreviewFor(previewFor === draft ? null : draft)}><WandSparkles size={15}/>{t('remote.improvePrompt')}</button>
    {previewFor === draft && <div className="max-h-52 overflow-auto rounded-lg border border-border p-3">
      <p className="whitespace-pre-wrap break-words">{suggestion}</p>
      <button type="button" className="mt-2 min-h-9 font-medium text-accent" onClick={() => { setPreviewFor(null); onApply(suggestion); }}>{t('remote.usePrompt')}</button>
    </div>}
  </div>;
}
