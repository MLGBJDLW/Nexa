import { useState } from 'react';
import { useNavigate } from 'react-router';
import { BellRing, ChevronRight, CircleHelp, ShieldCheck } from 'lucide-react';
import { useTranslation } from '../../i18n';
import { useAttentionInbox } from '../../lib/useAttentionInbox';
import { useRequestDeadline } from '../../lib/useRequestDeadline';
import type { AttentionItem } from '../../lib/attentionInbox';
import { NexaPopover, NexaPopoverContent, NexaPopoverTrigger } from '../ui/overlay/Popover';

function AttentionRow({ item, onOpen }: { item: AttentionItem; onOpen: () => void }) {
  const { t } = useTranslation();
  const remaining = useRequestDeadline(item.expiresAt);
  const Icon = item.kind === 'approval' ? ShieldCheck : CircleHelp;
  return (
    <button type="button" onClick={onOpen} data-testid={`attention-item-${item.id}`}
      className="flex w-full items-start gap-2 rounded-md p-2 text-left hover:bg-surface-2">
      <Icon className="mt-0.5 h-4 w-4 shrink-0 text-accent" aria-hidden="true" />
      <span className="min-w-0 flex-1">
        <span className="block text-[11px] text-text-secondary">{t(item.kind === 'approval' ? 'chat.attentionApproval' : 'chat.attentionQuestion')}</span>
        <span className="block break-words text-sm font-medium">{item.title}</span>
        {item.description && <span className="block line-clamp-2 break-words text-xs text-text-secondary">{item.description}</span>}
        {remaining !== null && <span className="block text-xs text-warning">{remaining === 0 ? t('chat.approvalExpired') : t('chat.approvalExpiresIn', { seconds: String(remaining) })}</span>}
      </span>
      <ChevronRight className="mt-2 h-4 w-4 shrink-0 text-text-tertiary" aria-hidden="true" />
    </button>
  );
}

export function AttentionInboxButton() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const items = useAttentionInbox();
  const [open, setOpen] = useState(false);
  const label = items.length ? t('chat.attentionCount', { count: String(items.length) }) : t('chat.attentionTitle');
  return (
    <NexaPopover open={open} onOpenChange={setOpen}>
      <NexaPopoverTrigger asChild>
        <button type="button" aria-label={label} title={label} data-testid="attention-inbox-toggle"
          className={`relative grid h-10 w-10 place-items-center rounded-md transition-colors hover:bg-surface-2 ${items.length ? 'text-accent' : 'text-text-tertiary'}`}>
          <BellRing className="h-4.5 w-4.5" aria-hidden="true" />
          {items.length > 0 && <span data-testid="attention-inbox-count" className="absolute right-0 top-0 flex h-4 min-w-4 items-center justify-center rounded-full bg-accent px-1 text-[10px] font-semibold text-white">{items.length}</span>}
        </button>
      </NexaPopoverTrigger>
      <NexaPopoverContent side="right" align="end" aria-label={t('chat.attentionTitle')}
        className="max-h-[min(75dvh,560px)] w-[min(360px,calc(100vw-5rem))] overflow-y-auto rounded-xl border border-border bg-surface-1 p-2 text-text-primary shadow-xl" data-testid="attention-inbox">
        <h2 className="px-2 py-1 text-sm font-semibold">{label}</h2>
        {items.length === 0 ? <p className="p-2 text-xs text-text-secondary">{t('chat.attentionEmpty')}</p> : items.map(item => (
          <AttentionRow key={item.id} item={item} onOpen={() => {
            setOpen(false);
            navigate(`/chat/${encodeURIComponent(item.conversationId)}`);
          }} />
        ))}
      </NexaPopoverContent>
    </NexaPopover>
  );
}
