import { Paperclip, X } from 'lucide-react';
import { useRef, useState } from 'react';
import { useTranslation } from '../../i18n';
import { getAllowedAttachmentMediaType } from '../../lib/chatAttachments';
import type { ImageAttachment } from '../../types/conversation';
import { remoteButton, remoteField } from './remoteUi';

export interface RemoteComposerSettings {
  executionMode: 'normal' | 'plan';
  powerMode: 'standard' | 'nexus';
  collaborationMode: 'direct' | 'mixtureOfAgents';
}
export const defaultComposerSettings: RemoteComposerSettings = { executionMode:'normal', powerMode:'standard', collaborationMode:'direct' };

export function RemoteComposerOptions({ attachments, onAttachments, settings, onSettings, disabled, onBusy }: {
  attachments: ImageAttachment[]; onAttachments: (items: ImageAttachment[]) => void;
  settings: RemoteComposerSettings; onSettings: (value: RemoteComposerSettings) => void;
  disabled: boolean; onBusy: (busy: boolean) => void;
}) {
  const { t } = useTranslation();
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  async function add(files: File[]) {
    if (loading || disabled || !files.length) return;
    setLoading(true); onBusy(true); setError('');
    try {
      if (attachments.length + files.length > 8 || attachments.reduce((sum, item) => sum + item.base64Data.length * 0.75, 0) + files.reduce((sum, file) => sum + file.size, 0) > 8 * 1024 * 1024) throw new Error(t('remote.attachmentLimit'));
      const next = await Promise.all(files.map(async file => {
        const mediaType = getAllowedAttachmentMediaType(file.type, file.name);
        if (!mediaType) throw new Error(t('remote.attachmentUnsupported', { name:file.name }));
        const base64Data = await new Promise<string>((resolve, reject) => {
          const reader = new FileReader(); reader.onload = () => resolve(String(reader.result).split(',')[1]);
          reader.onerror = () => reject(reader.error); reader.readAsDataURL(file);
        });
        return { base64Data, mediaType, originalName:file.name };
      }));
      onAttachments([...attachments, ...next]);
    } catch (error) { setError(error instanceof Error ? error.message : String(error)); }
    finally { setLoading(false); onBusy(false); if (input.current) input.current.value = ''; }
  }
  return <div className="flex flex-wrap items-center gap-2" onDragOver={event => event.preventDefault()} onDrop={event => { event.preventDefault(); void add(Array.from(event.dataTransfer.files)); }}>
    <input ref={input} type="file" multiple className="hidden" aria-label={t('remote.addAttachments')} disabled={disabled || loading} onChange={event => void add(Array.from(event.target.files ?? []))} />
    <button type="button" className={remoteButton} disabled={disabled || loading} onClick={() => input.current?.click()}><Paperclip size={16} />{t('remote.addAttachments')}</button>
    <select className={`${remoteField} !w-auto min-w-24 flex-1`} aria-label={t('remote.executionMode')} disabled={disabled} value={settings.executionMode} onChange={event => onSettings({...settings, executionMode:event.target.value as RemoteComposerSettings['executionMode']})}>
      <option value="normal">{t('remote.modeNormal')}</option><option value="plan">{t('remote.modePlan')}</option>
    </select>
    <label className="flex items-center gap-2 text-sm"><input type="checkbox" checked={settings.powerMode === 'nexus'} disabled={disabled} onChange={event => onSettings({...settings, powerMode:event.target.checked ? 'nexus' : 'standard'})} />Nexus</label>
    <label className="flex items-center gap-2 text-sm"><input type="checkbox" checked={settings.collaborationMode === 'mixtureOfAgents'} disabled={disabled} onChange={event => onSettings({...settings, collaborationMode:event.target.checked ? 'mixtureOfAgents' : 'direct'})} />{t('remote.collaboration')}</label>
    {attachments.length > 0 && <div className="flex w-full flex-wrap gap-2">{attachments.map((item, index) => <span key={index} className="flex max-w-full items-center gap-2 rounded-lg border border-border p-2 text-xs">
      {item.mediaType.startsWith('image/') && <img src={`data:${item.mediaType};base64,${item.base64Data}`} alt="" className="h-12 w-12 rounded object-cover" />}
      <span className="truncate">{item.originalName}</span><button type="button" disabled={disabled} aria-label={t('chat.removeAttachment')} onClick={() => onAttachments(attachments.filter((_, i) => i !== index))}><X size={16} /></button>
    </span>)}</div>}
    {error && <p role="alert" className="w-full text-xs text-danger">{error}</p>}
  </div>;
}
