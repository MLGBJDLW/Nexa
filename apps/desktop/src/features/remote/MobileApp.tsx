import { useCallback, useEffect, useMemo, useState } from 'react';
import { Camera, Link2, Loader2, MessageCircle, Radio, ShieldCheck, Wifi } from 'lucide-react';
import { useTranslation } from '../../i18n';
import { LiveWorkspace } from '../live/LiveWorkspace';
import { RemoteClient, type PairedRemote, type RemoteConnection } from './remoteClient';
import { remoteLiveTransport } from './remoteLiveTransport';
import { RemoteChat } from './RemoteChat';
import { remoteButton, remoteField } from './remoteUi';

const STORE = 'nexa.remote.paired';
function saved(): PairedRemote | null {
  try {
    const value = JSON.parse(localStorage.getItem(STORE) || 'null') as PairedRemote | null;
    return value?.manifest?.serverId &&
      (!expectedServer || value.manifest.serverId === expectedServer)
      ? value
      : null;
  } catch {
    return null;
  }
}
const pairing = new URLSearchParams(location.hash.slice(1));
const initialCode = pairing.get('pair') || '';
const expectedServer = pairing.get('server');
if (location.hash) history.replaceState(null, '', `${location.pathname}${location.search}`);

export function MobileApp() {
  const { t, locale, setLocale, availableLocales } = useTranslation();
  const [paired, setPaired] = useState<PairedRemote | null>(saved);
  const [code, setCode] = useState(initialCode);
  const [name, setName] = useState('');
  const [pairingBusy, setPairingBusy] = useState(false);
  const [error, setError] = useState('');
  const [tab, setTab] = useState<'chat' | 'live'>('chat');
  const [settings, setSettings] = useState(false);
  const [draft, setDraft] = useState('');
  const [connection, setConnection] = useState<RemoteConnection>({
    phase: 'connecting',
    endpoint: null,
  });
  const client = useMemo(
    () =>
      paired
        ? new RemoteClient(paired, (current) =>
            localStorage.setItem(STORE, JSON.stringify(current)),
          )
        : null,
    [paired],
  );
  const liveTransport = useMemo(() => (client ? remoteLiveTransport(client) : null), [client]);
  const draftConsumed = useCallback(() => setDraft(''), []);
  const pair = useCallback(
    async (pairCode: string, deviceName: string) => {
      setPairingBusy(true);
      setError('');
      try {
        const nonceKey = `nexa.remote.pair.${expectedServer || location.origin}.${pairCode}`;
        const clientNonce = sessionStorage.getItem(nonceKey) || crypto.randomUUID();
        sessionStorage.setItem(nonceKey, clientNonce);
        const response = await fetch('/api/pair', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ code: pairCode, name: deviceName.trim() || 'Phone', clientNonce }),
          signal: AbortSignal.timeout(15_000),
        });
        const result = await response.json();
        if (!response.ok) throw new Error(result.error || t('remote.pairFailed'));
        if (expectedServer && result.manifest.serverId !== expectedServer)
          throw new Error(t('remote.identityChanged'));
        localStorage.setItem(STORE, JSON.stringify(result));
        setPaired(result);
        setCode('');
      } catch (error) {
        setError(error instanceof Error ? error.message : String(error));
      } finally {
        setPairingBusy(false);
      }
    },
    [t],
  );
  useEffect(() => {
    if (initialCode && !saved()) void pair(initialCode, 'Phone');
  }, []);
  useEffect(() => {
    if (!client) return;
    const off = client.subscribeConnection(setConnection);
    void client.start().catch((error) => setError(String(error)));
    return () => {
      off();
      client.close();
    };
  }, [client]);
  async function certificate() {
    try {
      const pem = await client!.rpc<string>('connection.certificate');
      const url = URL.createObjectURL(new Blob([pem], { type: 'application/x-x509-ca-cert' }));
      const anchor = document.createElement('a');
      anchor.href = url;
      anchor.download = 'Nexa-LAN-CA.crt';
      anchor.click();
      setTimeout(() => URL.revokeObjectURL(url), 10_000);
    } catch (error) {
      setError(String(error));
    }
  }
  return (
    <div
      className="flex min-h-dvh flex-col bg-surface-0 text-text-primary"
      data-testid="remote-app"
    >
      <header className="sticky top-0 z-20 border-b border-border bg-surface-0/95 px-4 pb-3 pt-[max(12px,env(safe-area-inset-top))] backdrop-blur">
        <div className="mx-auto flex max-w-3xl items-center justify-between gap-3">
          <div className="flex items-center gap-2.5">
            <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-accent text-white">
              <Radio size={18} />
            </div>
            <div>
              <p className="text-sm font-semibold tracking-widest">NEXA</p>
              <p className="text-[10px] font-medium uppercase tracking-[0.18em] text-text-tertiary">
                {t('remote.companion')}
              </p>
            </div>
          </div>
          <button
            className="flex items-center gap-1.5 text-xs text-text-secondary"
            onClick={() => setSettings((value) => !value)}
          >
            <span
              className={`h-1.5 w-1.5 rounded-full ${client && connection.phase === 'connected' ? 'bg-emerald-500' : 'bg-amber-400'}`}
            />
            {client
              ? connection.phase === 'connected'
                ? connection.endpoint?.kind === 'lan'
                  ? t('remote.lan')
                  : connection.endpoint?.kind === 'ssh'
                    ? 'SSH'
                    : t('remote.away')
                : connection.phase === 'revoked'
                  ? t('remote.revoked')
                  : t('remote.reconnecting')
              : t('remote.pair')}
          </button>
        </div>
      </header>
      {settings && (
        <section className="mx-auto w-full max-w-3xl space-y-3 border-b border-border p-4">
          <select
            aria-label={t('remote.language')}
            className={remoteField}
            value={locale}
            onChange={(event) => setLocale(event.target.value as typeof locale)}
          >
            {availableLocales.map((item) => (
              <option key={item.code} value={item.code}>
                {item.name}
              </option>
            ))}
          </select>
          {client && (
            <>
              <p className="break-all text-xs text-text-secondary">{connection.endpoint?.url}</p>
              <button className={remoteButton} onClick={() => void certificate()}>
                <ShieldCheck size={16} />
                {t('remote.certificate')}
              </button>
              <p className="text-xs leading-6 text-text-secondary">{t('remote.lanTrust')}</p>
              <button
                className={remoteButton}
                onClick={() => {
                  client.close();
                  localStorage.removeItem(STORE);
                  setPaired(null);
                  setError('');
                }}
              >
                {t('remote.forget')}
              </button>
            </>
          )}
        </section>
      )}
      {client && connection.phase !== 'connected' && (
        <div
          role="status"
          className="mx-auto w-full max-w-3xl border-b border-amber-500/20 bg-amber-500/5 p-3 text-sm leading-6"
        >
          {connection.phase === 'revoked' ? t('remote.revokedHint') : t('remote.reconnectHint')}
        </div>
      )}
      {error && (!client || settings) && (
        <p
          role="alert"
          className="mx-auto m-4 w-[calc(100%-32px)] max-w-3xl rounded-xl border border-danger/30 p-3 text-sm text-danger"
        >
          {error}
        </p>
      )}
      {!client ? (
        <main className="mx-auto flex w-full max-w-md flex-1 flex-col justify-center gap-6 p-6 pb-16">
          <div className="mx-auto flex h-20 w-20 items-center justify-center rounded-3xl border border-accent/20 bg-accent/5 text-accent">
            <Link2 size={32} />
          </div>
          <div className="text-center">
            <h1 className="text-2xl font-semibold">{t('remote.pairTitle')}</h1>
            <p className="mt-3 text-sm leading-7 text-text-secondary">{t('remote.pairHint')}</p>
          </div>
          <form
            className="space-y-4"
            onSubmit={(event) => {
              event.preventDefault();
              void pair(code, name);
            }}
          >
            <input
              className={remoteField}
              aria-label={t('remote.deviceName')}
              placeholder={t('remote.deviceName')}
              maxLength={80}
              value={name}
              onChange={(event) => setName(event.target.value)}
            />
            <input
              className={`${remoteField} text-center !text-2xl tracking-[0.35em]`}
              aria-label={t('remote.pairCode')}
              placeholder="000000"
              autoComplete="one-time-code"
              inputMode="numeric"
              pattern="[0-9]{6}"
              maxLength={6}
              value={code}
              onChange={(event) => setCode(event.target.value.replace(/\D/g, ''))}
            />
            <button
              className={`${remoteButton} w-full !bg-accent text-white`}
              disabled={pairingBusy || code.length !== 6}
            >
              {pairingBusy ? <Loader2 className="animate-spin" size={17} /> : <Camera size={17} />}
              {t('remote.pair')}
            </button>
          </form>
          <p className="text-center text-xs leading-6 text-text-tertiary">
            {t('remote.pairAuthority')}
          </p>
        </main>
      ) : (
        <>
          <nav
            className="mx-auto grid w-full max-w-3xl grid-cols-2 gap-2 px-4 pt-4"
            aria-label={t('remote.navigation')}
          >
            {(['chat', 'live'] as const).map((value) => (
              <button
                key={value}
                className={`${remoteButton} ${tab === value ? '!border-accent/30 !bg-accent/10 text-accent' : 'text-text-secondary'}`}
                aria-pressed={tab === value}
                onClick={() => setTab(value)}
              >
                {value === 'chat' ? <MessageCircle size={17} /> : <Wifi size={17} />}
                {value === 'chat' ? t('remote.chat') : 'Live'}
              </button>
            ))}
          </nav>
          {tab === 'chat' ? (
            <RemoteChat client={client} initialDraft={draft} onDraftConsumed={draftConsumed} />
          ) : (
            liveTransport && (
              <LiveWorkspace
                transport={liveTransport}
                onSendToChat={(text) => {
                  setDraft(text);
                  setTab('chat');
                }}
              />
            )
          )}
        </>
      )}
    </div>
  );
}
