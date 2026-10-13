import { useEffect, useState } from 'react';
import { Loader2, WifiOff } from 'lucide-react';
import { useTranslation } from '../../i18n';
import { FlipStatusText } from '../ui/FlipNumber';
import type { ProviderConnectionState } from '../../types/conversation';

interface ConnectionStatusBannerProps { connection: ProviderConnectionState | null }

export function ConnectionStatusBanner({ connection }: ConnectionStatusBannerProps) {
  const { t } = useTranslation();
  const [now, setNow] = useState(Date.now);
  const retrying = connection?.state === 'reconnecting' || connection?.state === 'degraded';
  const retryAt = connection?.nextRetryAt ? Date.parse(connection.nextRetryAt) : NaN;
  useEffect(() => {
    setNow(Date.now());
    if (!retrying || !Number.isFinite(retryAt)) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [retrying, retryAt]);
  const failed = connection?.state === 'failed' || connection?.state === 'offline';
  if (!connection || (!failed && !retrying)) return null;

  const label = retrying ? t('chat.connectionReconnecting') : connection.state === 'offline' ? t('chat.connectionOffline') : t('chat.connectionFailed');
  const detail = connection.turnPreserved ? t('chat.connectionTurnPreserved') : t('chat.connectionTurnNotPreserved');
  const seconds = Number.isFinite(retryAt) ? Math.max(0, Math.ceil((retryAt - now) / 1000)) : 0;
  const retryLabel = seconds > 0 ? t('chat.connectionRetryIn', { seconds: String(seconds) }) : t('chat.connectionTryingNow');
  const attemptLabel = t('chat.connectionAttempt', { attempt: String(connection.attempt), total: String(connection.maxAttempts) });

  return <div className="shrink-0 px-4 pb-2" data-testid="provider-connection-status">
    <div role={failed ? 'alert' : undefined} aria-atomic={failed ? true : undefined} className={`mx-auto flex max-w-4xl items-start gap-2 py-2 text-xs ${failed ? 'text-danger' : 'text-text-secondary'}`}>
      {failed ? <WifiOff className="mt-0.5 h-3.5 w-3.5 shrink-0" /> : <Loader2 className="mt-0.5 h-3.5 w-3.5 shrink-0 animate-spin motion-reduce:animate-none" />}
      <div className="min-w-0 flex-1">
        <div role={retrying ? 'status' : undefined} aria-live={retrying ? 'polite' : undefined} className="flex flex-wrap items-center gap-x-2 gap-y-1">
          <span className="font-medium">{label}</span>
          {retrying && connection.maxAttempts > 0 && <span className="text-text-tertiary"><FlipStatusText text={attemptLabel} /></span>}
        </div>
        {retrying && <div aria-live="off" className="mt-1 text-text-tertiary" data-testid="connection-retry-countdown"><FlipStatusText text={retryLabel} /></div>}
        <div className="mt-1 break-words text-[11px] text-text-tertiary">
          {connection.providerId} · {connection.modelId} · {detail}
          {failed && ` ${t('chat.connectionNextAction')}`}
        </div>
      </div>
    </div>
  </div>;
}
