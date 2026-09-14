import { useEffect, useRef, useState } from 'react';
import { Check, Copy } from 'lucide-react';
import { toast } from 'sonner';
import { useTranslation } from '../../i18n';

export function CopyTextButton({ text, label, disabled = false }: { text: string; label: string; disabled?: boolean }) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => { setCopied(false); return () => clearTimeout(timer.current); }, [text]);
  return <button type="button" aria-label={label} title={label} disabled={disabled || !text.trim()}
    className="inline-flex shrink-0 items-center gap-1.5 rounded-md px-2 py-1.5 text-xs text-text-secondary hover:bg-surface-2 focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-40"
    onClick={async () => {
      try {
        await navigator.clipboard.writeText(text);
        clearTimeout(timer.current); setCopied(true);
        timer.current = setTimeout(() => setCopied(false), 2000);
      } catch (error) { toast.error(String(error)); }
    }}>
    {copied ? <Check size={14} /> : <Copy size={14} />}<span aria-live="polite">{t(copied ? 'chat.copied' : 'citation.copy')}</span>
  </button>;
}
