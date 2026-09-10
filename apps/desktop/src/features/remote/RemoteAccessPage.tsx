import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  ArrowRight,
  Check,
  ChevronDown,
  Copy,
  Globe2,
  Loader2,
  MessageCircle,
  Mic,
  Plus,
  QrCode,
  ShieldCheck,
  Smartphone,
  Square,
  Wifi,
  X,
} from 'lucide-react';
import { useTranslation } from '../../i18n';
import { remoteButton, remoteField } from './remoteUi';
import { refreshRemoteStatus, useRemoteDesktopStatus } from './remoteDesktopStatus';

interface Pairing {
  pairing: { code: string; expiresAt: string };
  url: string;
  qrSvg: string;
}
interface ConnectionOptions {
  lan: boolean;
  quickTunnel: boolean;
  publicUrl: string;
}
let cachedPairing: Pairing | null = null;
function savedOptions(): ConnectionOptions {
  try {
    return {
      lan: true,
      quickTunnel: true,
      publicUrl: '',
      ...JSON.parse(localStorage.getItem('nexa.remote.options') || '{}'),
    };
  } catch {
    return { lan: true, quickTunnel: true, publicUrl: '' };
  }
}
export function RemoteAccessPage() {
  const { t } = useTranslation();
  const { status, error: statusError } = useRemoteDesktopStatus();
  const [pairing, setPairing] = useState<Pairing | null>(cachedPairing);
  const [options, setOptions] = useState<ConnectionOptions>(savedOptions);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [copied, setCopied] = useState(false);
  const [now, setNow] = useState(Date.now());
  const cancelled = useRef(false);
  const actionPending = useRef(false);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    const clock = setInterval(() => setNow(Date.now()), 1000);
    return () => {
      mounted.current = false;
      clearInterval(clock);
    };
  }, []);
  const online = status?.connectedDeviceIds || [];
  const enabled = status?.enabled || false;
  const preparing = status?.preparing || (busy && !enabled);
  const pairedDevice = status?.devices.find((device) => device.id === status.pairingDeviceId);
  const pairingComplete = Boolean(pairing && pairedDevice);
  const remaining = pairing
    ? Math.max(0, Math.ceil((Date.parse(pairing.pairing.expiresAt) - now) / 1000))
    : 0;
  const connectedView = pairingComplete || (!pairing && Boolean(status?.devices.length));
  async function newCode() {
    const next = await invoke<Pairing>('remote_pairing_cmd');
    cachedPairing = next;
    await refreshRemoteStatus();
    if (mounted.current) setPairing(next);
  }
  async function act(action: () => Promise<void>) {
    if (actionPending.current) return;
    actionPending.current = true;
    cancelled.current = false;
    setBusy(true);
    setError('');
    try {
      await action();
    } catch (error) {
      if (mounted.current && !cancelled.current) setError(String(error));
    } finally {
      actionPending.current = false;
      if (mounted.current) setBusy(false);
    }
  }
  async function start() {
    localStorage.setItem('nexa.remote.options', JSON.stringify(options));
    await invoke('start_remote_cmd', {
      options: { ...options, publicUrl: options.publicUrl || null },
    });
    await refreshRemoteStatus();
    if (!cancelled.current) await newCode();
  }
  async function stop() {
    cancelled.current = true;
    try {
      await invoke('stop_remote_cmd');
      cachedPairing = null;
      if (mounted.current) {
        setPairing(null);
        setError('');
      }
      await refreshRemoteStatus();
    } catch (error) {
      if (mounted.current) setError(String(error));
    }
  }
  const stateLabel = preparing
    ? t('remote.starting')
    : online.length
      ? t('remote.connectedCount', { count: online.length })
      : enabled
        ? t('remote.ready')
        : t('remote.disabled');
  return (
    <main
      className="mx-auto max-w-[1100px] space-y-7 p-5 pb-12 text-text-primary sm:p-9"
      data-testid="remote-settings"
    >
      <header className="flex flex-wrap items-start justify-between gap-4">
        <div>
          <div className="mb-3 text-[11px] font-semibold tracking-[0.18em] text-accent">
            NEXA CONNECT
          </div>
          <h1 className="text-3xl font-semibold tracking-tight">{t('remote.title')}</h1>
          <p className="mt-3 max-w-xl text-sm leading-7 text-text-secondary">
            {t('remote.subtitle')}
          </p>
        </div>
        <span role="status" className="mt-1 flex items-center gap-2 text-xs text-text-secondary">
          <span
            className={`h-2 w-2 rounded-full ${preparing ? 'animate-pulse bg-amber-400' : online.length ? 'bg-emerald-500' : enabled ? 'bg-accent' : 'bg-text-tertiary/40'}`}
          />
          {stateLabel}
        </span>
      </header>
      <ol
        className="grid grid-cols-3 gap-3 border-y border-border py-5"
        aria-label={t('remote.setupSteps')}
      >
        {[t('remote.stepEnable'), t('remote.stepScan'), t('remote.stepConnected')].map(
          (label, index) => {
            const done =
              index === 0
                ? enabled
                : index === 1
                  ? online.length > 0 || pairingComplete
                  : online.length > 0;
            const current =
              index === 0 ? !enabled : index === 1 ? !online.length : online.length > 0;
            return (
              <li key={label} className="flex items-center gap-2.5 text-xs sm:text-sm">
                <span
                  className={`grid h-7 w-7 shrink-0 place-items-center rounded-full text-xs ${done ? 'bg-accent text-white' : current ? 'border border-accent/30 bg-accent/5 text-accent' : 'border border-border text-text-tertiary'}`}
                >
                  {done ? <Check size={14} /> : index + 1}
                </span>
                <span
                  className={
                    done || current ? 'font-medium text-text-primary' : 'text-text-tertiary'
                  }
                >
                  {label}
                </span>
                {index < 2 && (
                  <ArrowRight size={14} className="ml-auto hidden text-text-tertiary/40 sm:block" />
                )}
              </li>
            );
          },
        )}
      </ol>
      {(error || statusError) && (
        <div
          role="alert"
          className="flex items-center justify-between gap-3 rounded-xl border border-danger/25 bg-danger/5 p-3 text-sm text-danger"
        >
          <span>{error || statusError}</span>
          {statusError && (
            <button className={remoteButton} onClick={() => void refreshRemoteStatus()}>
              {t('common.retry')}
            </button>
          )}
        </div>
      )}
      {status?.warning && (
        <p
          role="status"
          className="rounded-xl border border-amber-500/25 bg-amber-500/5 p-3 text-sm leading-6"
        >
          {status.warning}
        </p>
      )}
      <div className="grid items-start gap-7 md:grid-cols-[minmax(0,1fr)_340px]">
        <div className="space-y-6">
          <section className="space-y-5 pt-1">
            <h2 className="text-xl font-semibold">{t('remote.continueAnywhere')}</h2>
            <div className="space-y-5">
              {[
                {
                  icon: MessageCircle,
                  title: t('remote.chatFeature'),
                  body: t('remote.chatFeatureHint'),
                },
                { icon: Mic, title: t('remote.liveFeature'), body: t('remote.liveFeatureHint') },
              ].map(({ icon: Icon, title, body }) => (
                <div className="flex gap-3.5" key={title}>
                  <span className="grid h-10 w-10 shrink-0 place-items-center rounded-xl border border-border bg-surface-1 text-accent">
                    <Icon size={19} strokeWidth={1.6} />
                  </span>
                  <div>
                    <h3 className="text-sm font-medium">{title}</h3>
                    <p className="mt-1 text-xs leading-6 text-text-secondary">{body}</p>
                  </div>
                </div>
              ))}
            </div>
            <div className="space-y-3 pt-1">
              {preparing ? (
                <div className="flex flex-wrap gap-2">
                  <button className={`${remoteButton} !border-accent/30 text-accent`} disabled>
                    <Loader2 size={16} className="animate-spin" />
                    {t('remote.starting')}
                  </button>
                  <button className={remoteButton} onClick={() => void stop()}>
                    <X size={15} />
                    {t('remote.cancelSetup')}
                  </button>
                </div>
              ) : enabled ? (
                <button
                  className={`${remoteButton} text-text-secondary`}
                  disabled={busy}
                  onClick={() => void act(stop)}
                >
                  <Square size={14} />
                  {t('remote.disable')}
                </button>
              ) : (
                <button
                  className={`${remoteButton} !border-accent !bg-accent px-5 text-white hover:!bg-accent-hover`}
                  disabled={busy || !status}
                  onClick={() => void act(start)}
                >
                  <QrCode size={17} />
                  {t('remote.enable')}
                  <ArrowRight size={15} />
                </button>
              )}
              <p className="max-w-md text-xs leading-6 text-text-tertiary">
                {t('remote.keepRunning')}
              </p>
              {options.quickTunnel && (
                <p className="max-w-md text-xs leading-6 text-text-tertiary">
                  {t('remote.temporaryHint')}
                </p>
              )}
            </div>
          </section>
          <details
            className="group rounded-xl border border-border bg-surface-1/40"
            data-testid="remote-advanced"
          >
            <summary className="flex cursor-pointer list-none items-center justify-between px-4 py-3.5 text-sm font-medium">
              {t('remote.advancedOptions')}
              <ChevronDown
                size={15}
                className="text-text-tertiary transition-transform group-open:rotate-180"
              />
            </summary>
            <div className="space-y-4 border-t border-border px-4 py-4">
              <fieldset disabled={enabled || busy} className="space-y-4">
                <label className="flex items-start gap-3 text-sm">
                  <input
                    type="checkbox"
                    checked={options.lan}
                    onChange={(event) =>
                      setOptions((current) => ({ ...current, lan: event.target.checked }))
                    }
                    className="mt-1"
                  />
                  <span>
                    {t('remote.lanEnable')}
                    <small className="mt-1 block leading-6 text-text-secondary">
                      {t('remote.lanEnableHint')}
                    </small>
                  </span>
                </label>
                <label className="flex items-start gap-3 text-sm">
                  <input
                    type="checkbox"
                    checked={options.quickTunnel}
                    onChange={(event) =>
                      setOptions((current) => ({ ...current, quickTunnel: event.target.checked }))
                    }
                    className="mt-1"
                  />
                  <span>
                    {t('remote.quickEnable')}
                    <small className="mt-1 block leading-6 text-text-secondary">
                      {t('remote.quickHint')}
                    </small>
                  </span>
                </label>
                <label className="block space-y-2 text-sm">
                  <span>{t('remote.fixedUrl')}</span>
                  <input
                    className={remoteField}
                    type="url"
                    value={options.publicUrl}
                    placeholder="https://nexa.example.com"
                    onChange={(event) =>
                      setOptions((current) => ({
                        ...current,
                        publicUrl: event.target.value,
                        quickTunnel: event.target.value.trim() ? false : current.quickTunnel,
                      }))
                    }
                  />
                </label>
                <p className="text-xs leading-6 text-text-secondary">{t('remote.fixedHint')}</p>
              </fieldset>
              {enabled && <p className="text-xs text-text-tertiary">{t('remote.stopToChange')}</p>}
              <p className="text-xs leading-6 text-text-secondary">{t('remote.sshHint')}</p>
              {status?.manifest?.endpoints.map((endpoint) => (
                <div
                  key={endpoint.url}
                  className="flex items-start gap-2 break-all rounded-lg bg-surface-2 p-2.5 text-xs text-text-secondary"
                >
                  {endpoint.kind === 'lan' ? (
                    <Wifi size={14} className="shrink-0" />
                  ) : (
                    <Globe2 size={14} className="shrink-0" />
                  )}
                  <span>{endpoint.url}</span>
                </div>
              ))}
            </div>
          </details>
        </div>
        <section
          className={`overflow-hidden rounded-2xl border bg-surface-1 text-center ${connectedView ? 'border-emerald-500/25' : 'border-border'}`}
          data-testid="remote-pair-card"
        >
          <div className="border-b border-border px-5 py-4 text-sm font-medium">
            {connectedView ? t('remote.connectionComplete') : t('remote.scan')}
          </div>
          <div className="space-y-4 p-5">
            {connectedView ? (
              <div className="space-y-4 py-5">
                <div className="relative mx-auto grid h-24 w-24 place-items-center rounded-3xl bg-emerald-500/8 text-emerald-500">
                  <Smartphone size={47} strokeWidth={1.4} />
                  <span className="absolute bottom-2 right-2 rounded-full border-4 border-surface-1 bg-emerald-500 p-1 text-white">
                    <Check size={12} />
                  </span>
                </div>
                <div>
                  <h3 className="font-medium">{pairedDevice?.name || status?.devices[0]?.name}</h3>
                  <p className="mt-2 text-xs leading-6 text-text-secondary">
                    {online.length ? t('remote.connectedHint') : t('remote.pairedHint')}
                  </p>
                </div>
              </div>
            ) : pairing && remaining > 0 ? (
              <>
                <img
                  className="mx-auto w-full max-w-64 rounded-xl bg-white p-2"
                  alt={t('remote.qrAlt')}
                  src={`data:image/svg+xml;base64,${btoa(pairing.qrSvg)}`}
                />
                <div>
                  <p className="font-mono text-3xl tracking-[0.2em]">{pairing.pairing.code}</p>
                  <p className="mt-2 text-xs tabular-nums text-text-tertiary">
                    {t('remote.expires', { seconds: remaining })}
                  </p>
                </div>
                <button
                  className={remoteButton}
                  onClick={() =>
                    void navigator.clipboard
                      .writeText(pairing.url)
                      .then(() => {
                        setCopied(true);
                        setTimeout(() => {
                          if (mounted.current) setCopied(false);
                        }, 1500);
                      })
                      .catch((error) => setError(String(error)))
                  }
                >
                  {copied ? <Check size={15} /> : <Copy size={15} />}
                  {t('remote.copyLink')}
                </button>
              </>
            ) : (
              <div className="flex min-h-56 flex-col items-center justify-center gap-5 rounded-xl border border-dashed border-border bg-surface-0/40 text-text-tertiary">
                {preparing ? (
                  <Loader2 size={40} strokeWidth={1.3} className="animate-spin text-accent" />
                ) : (
                  <QrCode size={66} strokeWidth={1} />
                )}
                <p className="max-w-52 text-xs leading-6">
                  {preparing
                    ? t('remote.preparingHint')
                    : pairing
                      ? t('remote.expired')
                      : t('remote.qrHint')}
                </p>
              </div>
            )}
            {enabled && (
              <button
                className={`${remoteButton} w-full`}
                disabled={busy}
                onClick={() => void act(newCode)}
              >
                <Plus size={15} />
                {connectedView ? t('remote.addDevice') : t('remote.newCode')}
              </button>
            )}
            <p className="text-[11px] leading-6 text-text-tertiary">{t('remote.pairAuthority')}</p>
          </div>
        </section>
      </div>
      {Boolean(status?.devices.length) && (
        <section className="space-y-4 border-t border-border pt-6">
          <h2 className="flex items-center gap-2 text-sm font-medium">
            <ShieldCheck size={16} className="text-text-tertiary" />
            {t('remote.devices')}
            <span className="font-normal text-text-tertiary">{status?.devices.length}</span>
          </h2>
          {status?.devices.map((device) => (
            <div
              key={device.id}
              className="flex items-center justify-between gap-4 rounded-xl border border-border bg-surface-1/50 p-4"
            >
              <div className="flex items-center gap-3">
                <Smartphone size={22} strokeWidth={1.5} className="text-text-secondary" />
                <div>
                  <p className="text-sm font-medium">{device.name}</p>
                  <p className="mt-1 flex items-center gap-1.5 text-xs text-text-tertiary">
                    <span
                      className={`h-1.5 w-1.5 rounded-full ${online.includes(device.id) ? 'bg-emerald-500' : 'bg-text-tertiary/40'}`}
                    />
                    {online.includes(device.id) ? t('remote.online') : t('remote.savedDevice')}
                  </p>
                </div>
              </div>
              <button
                className="rounded-lg px-3 py-2 text-xs text-text-tertiary hover:bg-danger/5 hover:text-danger disabled:opacity-40"
                disabled={busy}
                onClick={() =>
                  void act(async () => {
                    await invoke('revoke_remote_device_cmd', { deviceId: device.id });
                    await refreshRemoteStatus();
                  })
                }
              >
                {t('remote.revoke')}
              </button>
            </div>
          ))}
        </section>
      )}
    </main>
  );
}
