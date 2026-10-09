import { useEffect, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { FolderOpen, HardDrive, RotateCcw } from 'lucide-react';
import { useTranslation } from '../../i18n';
import * as api from '../../lib/api';
import type { UserExtensionLayout } from '../../types/extensions';
import { Button } from '../ui/Button';
import { Input } from '../ui/Input';
import { Section } from './SettingsSection';

export function LocalStorageSection({ managedModelPaths, modelStorageSaving, disabled, onApplyManagedModelRoot, onResetManagedModelRoot }: {
  managedModelPaths: api.ManagedModelPaths | null;
  modelStorageSaving: boolean;
  disabled: boolean;
  onApplyManagedModelRoot: (root: string) => void | Promise<void>;
  onResetManagedModelRoot: () => void | Promise<void>;
}) {
  const { t } = useTranslation();
  const [modelRootDraft, setModelRootDraft] = useState('');
  const [layout, setLayout] = useState<UserExtensionLayout | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let disposed = false;
    void api.getUserExtensionLayout().then(value => { if (!disposed) setLayout(value); }).catch(error => { if (!disposed) setError(String(error)); });
    return () => { disposed = true; };
  }, []);
  useEffect(() => {
    if (managedModelPaths?.root) setModelRootDraft(managedModelPaths.root);
  }, [managedModelPaths?.root]);

  const chooseModelRoot = async () => {
    const selected = await open({
      directory: true,
      multiple: false,
      title: t('settings.localModelStorageChoose'),
      defaultPath: modelRootDraft || undefined,
    });
    if (typeof selected === 'string') setModelRootDraft(selected);
  };

  return <Section icon={<HardDrive size={20} />} title={t('settings.storageSection')} description={t('settings.storageSectionDesc')} collapsible defaultOpen>
    <div data-testid="local-storage-settings" className="min-w-0 space-y-3">
      {error && <p role="alert" className="text-xs text-danger">{error}</p>}
      {layout && <div className="space-y-2 rounded-lg border border-border p-3 text-xs">
        <p className="font-medium text-text-primary">{t('settings.userExtensionHome')}</p>
        <p className="break-all font-mono text-text-secondary">{layout.root}</p>
        <p className="break-words text-text-tertiary">capabilities/ · skills/ · themes/ · workflows/ · connectors/mcp.json</p>
        <p className="text-text-tertiary">{t('settings.storageProjectContracts')}</p>
        <p className="text-text-tertiary">{t('settings.storageInternalState')}</p>
        <p className="break-all font-mono text-text-secondary">{layout.legacyAppDataDir}</p>
        <Button size="sm" variant="ghost" icon={<FolderOpen size={14} />} onClick={() => { void api.openFileInDefaultApp(layout.root).catch(error => setError(String(error))); }}>{t('settings.userExtensionOpenHome')}</Button>
      </div>}
        <div className="rounded-xl border border-border bg-surface-1/70 p-4">
          <div className="flex items-start gap-3">
            <span className="mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-accent/10 text-accent">
              <HardDrive size={18} />
            </span>
            <div className="min-w-0 flex-1">
              <p className="text-sm font-semibold text-text-primary">{t('settings.localModelStorage')}</p>
              <p className="mt-1 text-xs leading-relaxed text-text-tertiary">{t('settings.localModelStorageDesc')}</p>
              <div className="mt-3 flex gap-2">
                <Input
                  value={modelRootDraft}
                  onChange={(event) => setModelRootDraft(event.target.value)}
                  placeholder={managedModelPaths?.root ?? ''}
                  aria-label={t('settings.localModelStorage')}
                  className="min-w-0 flex-1 font-mono text-xs"
                />
                <Button
                  variant="secondary"
                  size="sm"
                  icon={<FolderOpen size={14} />}
                  onClick={() => { void chooseModelRoot(); }}
                  disabled={modelStorageSaving || disabled}
                >
                  {t('settings.localModelStorageBrowse')}
                </Button>
              </div>
              <div className="mt-3 flex flex-wrap gap-2">
                <Button
                  variant="primary"
                  size="sm"
                  onClick={() => { void onApplyManagedModelRoot(modelRootDraft.trim()); }}
                  loading={modelStorageSaving}
                  disabled={disabled || !modelRootDraft.trim()}
                >
                  {t('settings.localModelStorageApply')}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  icon={<RotateCcw size={14} />}
                  onClick={() => { void onResetManagedModelRoot(); }}
                  disabled={modelStorageSaving || disabled}
                >
                  {t('settings.localModelStorageDefault')}
                </Button>
              </div>
              {managedModelPaths && (
                <div className="mt-3 grid gap-1 rounded-lg border border-border/70 bg-surface-2/60 p-3 font-mono text-[11px] leading-relaxed text-text-tertiary">
                  <span>{t('settings.modelsEmbedding')}: {managedModelPaths.embedding}</span>
                  <span>{t('settings.modelsOcr')}: {managedModelPaths.ocr}</span>
                  <span>{t('settings.modelsWhisper')}: {managedModelPaths.whisper}</span>
                </div>
              )}
              <p className="mt-2 text-[11px] leading-relaxed text-warning">{t('settings.localModelStorageWarning')}</p>
            </div>
          </div>
        </div>


    </div>
  </Section>;
}
