import { useEffect, useRef, useState } from 'react';
import { Link } from 'react-router';
import { Camera, Loader2, Monitor, X } from 'lucide-react';
import { toast } from 'sonner';
import { useAppCommand } from '../../lib/appCommands';
import { useTranslation } from '../../i18n';
import { captureScreenSnapshot, useDesktopMonitors, useDesktopWindows } from '../../lib/screenCapture';
import { NexaPopover, NexaPopoverTrigger, NexaPopoverContent } from '../ui/overlay/Popover';

/** Chat captures once into its normal attachment draft. Live owns continuous capture. */
export function ScreenCaptureButton({ draftKey, disabled, onCapture }: {
  draftKey: string;
  disabled: boolean;
  onCapture: (dataUrl: string, name: string) => boolean;
}) {
  const { t } = useTranslation();
  const operation = useRef<AbortController | null>(null);
  const [pending, setPending] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false);
  const browserSupported = typeof navigator.mediaDevices?.getDisplayMedia === 'function';
  const monitors = useDesktopMonitors(pickerOpen || !browserSupported);
  const desktopWindows = useDesktopWindows(pickerOpen || !browserSupported);
  const supported = browserSupported || monitors.length > 0 || desktopWindows.windows.length > 0;

  const cancel = () => {
    operation.current?.abort();
    operation.current = null;
    setPending(false);
  };
  useEffect(() => {
    setPending(false);
    setPickerOpen(false);
    return () => { operation.current?.abort(); operation.current = null; };
  }, [draftKey]);

  const capture = async (monitorId?: string, windowId?: string) => {
    if (operation.current || disabled) return;
    const controller = new AbortController();
    operation.current = controller;
    setPickerOpen(false);
    setPending(true);
    try {
      const image = await captureScreenSnapshot(controller.signal, monitorId, windowId);
      if (!controller.signal.aborted && !onCapture(image, `screenshot-${Date.now()}.jpg`)) {
        throw new Error(t('chat.pasteImageFailed'));
      }
    } catch (error) {
      if (!controller.signal.aborted && !(error instanceof DOMException && error.name === 'NotAllowedError')) {
        toast.error(String(error));
      }
    } finally {
      if (operation.current === controller) { operation.current = null; setPending(false); }
    }
  };

  useAppCommand({
    id: 'chat.screen', label: 'chat.captureScreen', keywords: '/screen screenshot 截图 屏幕 窗口',
    enabled: supported && !disabled,
    run: () => { if (pending) cancel(); else setPickerOpen(true); },
  });

  return <NexaPopover open={pickerOpen} onOpenChange={setPickerOpen}>
    <NexaPopoverTrigger asChild>
      <button type="button" data-testid="screen-capture-toggle" disabled={disabled || !supported}
        aria-label={t(pending ? 'common.cancel' : 'chat.captureScreen')}
        title={t(pending ? 'common.cancel' : 'chat.captureScreen')}
        onClick={event => { if (pending) { event.preventDefault(); cancel(); } }}
        className="flex h-8 shrink-0 items-center gap-1.5 rounded-md px-2 text-xs text-text-tertiary transition-colors hover:bg-surface-2 disabled:opacity-40">
        {pending ? <><Loader2 size={15} className="animate-spin" /><X size={12} /></> : <Camera size={15} />}
      </button>
    </NexaPopoverTrigger>
    <NexaPopoverContent side="top" align="start" aria-label={t('chat.captureScreen')} className="w-72 rounded-lg border border-border bg-surface-1 p-2 text-text-primary shadow-xl">
      <p className="px-2 py-1 text-xs font-medium">{t('chat.captureScreen')}</p>
      <p className="px-2 pb-2 text-xs text-text-tertiary">{t('chat.captureScreenHint')}</p>
      {monitors.map((monitor, index) => <button type="button" key={monitor.id} className="flex w-full items-center gap-2 rounded-md p-2 text-left text-sm hover:bg-surface-2" onClick={() => void capture(monitor.id)}>
        <Monitor size={16} /><span>{t('live.screen')} {index + 1}<span className="ml-2 text-xs text-text-tertiary">{monitor.width} × {monitor.height}</span></span>
      </button>)}
      <div className="max-h-60 overflow-y-auto">{desktopWindows.windows.map(window => <button key={window.id} type="button" data-testid="screen-capture-window" className="flex w-full flex-col rounded-md p-2 text-left text-sm hover:bg-surface-2" onClick={() => void capture(undefined, window.id)}>
        <span className="w-full truncate">{window.title}</span><span className="text-xs text-text-tertiary">{window.appName}</span>
      </button>)}</div>
      {desktopWindows.supported === null && <Loader2 size={15} className="m-2 animate-spin" />}
      {browserSupported && desktopWindows.supported === false && <button type="button" className="w-full rounded-md p-2 text-left text-sm hover:bg-surface-2" onClick={() => void capture()}>{t('chat.shareWindow')}</button>}
      <Link to="/live" className="mt-2 block border-t border-border px-2 pt-2 text-xs text-accent">{t('chat.captureScreenLive')}</Link>
    </NexaPopoverContent>
  </NexaPopover>;
}
