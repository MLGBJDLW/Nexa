import { useRef, useState } from 'react';
import { useTranslation } from '../../i18n';
import { Button } from '../ui/Button';
import { Modal } from '../ui/Modal';

export interface CapabilityLinkOption {
  id: string;
  label: string;
  detail?: string;
  group?: string;
  disabled?: boolean;
  linked?: boolean;
}

interface Props {
  providerName: string;
  options: CapabilityLinkOption[];
  onApply: (ids: string[]) => Promise<void>;
  onDismiss: () => void;
}

export function CapabilityLinkDialog({ providerName, options, onApply, onDismiss }: Props) {
  const { t } = useTranslation();
  const [selected, setSelected] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const saving = useRef(false);
  const available = options.filter(option => !option.disabled && !option.linked);
  function toggle(option: CapabilityLinkOption) {
    setSelected(current => current.includes(option.id)
      ? current.filter(id => id !== option.id)
      : [...current.filter(id => !option.group || !options.some(other => other.id === id && other.group === option.group)), option.id]);
  }
  function selectAll() {
    const groups = new Set<string>();
    const result: string[] = [];
    // Keep a previously chosen alternative for each single-provider setting.
    for (const option of [...available.filter(row => selected.includes(row.id)), ...available]) {
      const group = option.group ?? option.id;
      if (groups.has(group)) continue;
      groups.add(group);
      result.push(option.id);
    }
    setSelected(result);
  }
  async function apply() {
    if (saving.current || !selected.length) return;
    saving.current = true;
    setBusy(true);
    setError('');
    try {
      await onApply(selected);
      onDismiss();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      saving.current = false;
      setBusy(false);
    }
  }
  return <Modal open onClose={() => { if (!saving.current) onDismiss(); }}
    title={t('providers.capabilityTitle', { name: providerName })}
    footer={<>
      <Button variant="secondary" disabled={busy} onClick={onDismiss}>{t('providers.capabilitySkip')}</Button>
      {available.length > 0 && <Button loading={busy} disabled={!selected.length || busy} onClick={apply}>{t('providers.capabilityApply')}</Button>}
    </>}>
    <p className="text-sm text-text-secondary">{t('providers.capabilityIntro')}</p>
    {available.length > 0 && <div className="my-2 flex gap-2">
      <Button size="sm" variant="ghost" disabled={busy} onClick={selectAll}>{t('common.selectAll')}</Button>
      <Button size="sm" variant="ghost" disabled={busy || !selected.length} onClick={() => setSelected([])}>{t('common.clearAll')}</Button>
    </div>}
    <ul className="divide-y divide-border">
      {options.map(option => <li key={option.id}>
        <label className="flex items-start gap-3 py-2.5">
          <input type="checkbox" className="mt-1 accent-accent" checked={!!option.linked || selected.includes(option.id)}
            disabled={busy || option.disabled || option.linked} onChange={() => toggle(option)} />
          <span className="min-w-0"><span className="block text-sm font-medium">{option.label}</span>
            {option.detail && <span className="block break-words text-xs text-text-secondary">{option.detail}</span>}</span>
        </label>
      </li>)}
    </ul>
    {error && <p role="alert" className="mt-2 text-sm text-danger">{error}</p>}
  </Modal>;
}
