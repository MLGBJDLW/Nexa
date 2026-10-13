import { Save, ScanLine } from 'lucide-react';
import { useTranslation } from '../../i18n';
import type { OcrConfig } from '../../types/ocr';
import { Button } from '../ui/Button';
import { Input } from '../ui/Input';
import { CollapsiblePanel, Section } from './SettingsSection';
import { SettingsRow } from './SettingsRow';
import { SettingsToggle } from './SettingsToggle';

interface OcrSettingsSectionProps {
  ocrConfig: OcrConfig | null;
  ocrSaveLoading: boolean;
  onConfigChange: (config: OcrConfig) => void;
  onMarkDirty: () => void;
  onSave: () => void;
}

export function OcrSettingsSection({
  ocrConfig,
  ocrSaveLoading,
  onConfigChange,
  onMarkDirty,
  onSave,
}: OcrSettingsSectionProps) {
  const { t } = useTranslation();

  const updateConfig = (patch: Partial<OcrConfig>) => {
    if (!ocrConfig) return;
    onConfigChange({ ...ocrConfig, ...patch });
    onMarkDirty();
  };

  return (
    <Section
      icon={<ScanLine size={20} />}
      title={t('settings.ocrSection')}
      delay={0.03}
      collapsible
      defaultOpen={false}
      summary={ocrConfig ? (
        <span className="rounded-full border border-border/60 bg-surface-2 px-2 py-1 text-[11px] text-text-secondary">
          {ocrConfig.enabled ? t('settings.skillFilterEnabled') : t('settings.skillFilterDisabled')}
        </span>
      ) : undefined}
    >
      {ocrConfig && (
        <div className="min-w-0 space-y-3">
          <SettingsRow label={t('settings.ocrEnabled')} description={t('settings.ocrEnabledDesc')} htmlFor="ocr-enabled">
            <div className="flex sm:justify-end"><SettingsToggle id="ocr-enabled" label={t('settings.ocrEnabled')} checked={ocrConfig.enabled} onChange={enabled => updateConfig({ enabled })} /></div>
          </SettingsRow>
          <SettingsRow label={t('settings.ocrLlmFallback')} description={t('settings.ocrLlmFallbackDesc')} htmlFor="ocr-fallback">
            <div className="flex sm:justify-end"><SettingsToggle id="ocr-fallback" label={t('settings.ocrLlmFallback')} checked={ocrConfig.llmFallbackEnabled} onChange={llmFallbackEnabled => updateConfig({ llmFallbackEnabled })} /></div>
          </SettingsRow>
          <CollapsiblePanel title={t('settings.advanced')}>
            <SettingsRow label={t('settings.ocrConfidence')} description={t('settings.ocrConfidenceDesc')} htmlFor="ocr-confidence">
              <div className="flex items-center gap-3"><input id="ocr-confidence" type="range" min="0" max="1" step="0.05" value={ocrConfig.confidenceThreshold} onChange={event => updateConfig({ confidenceThreshold: Number(event.target.value) })} className="min-w-0 flex-1 accent-accent" /><output htmlFor="ocr-confidence" className="w-8 text-right text-xs tabular-nums text-text-secondary">{ocrConfig.confidenceThreshold.toFixed(2)}</output></div>
            </SettingsRow>
            <SettingsRow label={t('settings.ocrDetLimit')} description={t('settings.ocrDetLimitDesc')} htmlFor="ocr-detection-limit">
              <Input id="ocr-detection-limit" type="number" value={ocrConfig.detLimitSideLen} onChange={event => updateConfig({ detLimitSideLen: parseInt(event.target.value) || 960 })} />
            </SettingsRow>
            <SettingsRow label={t('settings.ocrUseCls')} description={t('settings.ocrUseClsDesc')} htmlFor="ocr-orientation">
              <div className="flex sm:justify-end"><SettingsToggle id="ocr-orientation" label={t('settings.ocrUseCls')} checked={ocrConfig.useCls} onChange={useCls => updateConfig({ useCls })} /></div>
            </SettingsRow>
          </CollapsiblePanel>

          {/* Save button */}
          <div className="flex justify-end pt-2">
            <Button
              variant="primary"
              size="sm"
              icon={<Save size={16} />}
              loading={ocrSaveLoading}
              onClick={onSave}
            >
              {t('settings.saveConfig')}
            </Button>
          </div>
        </div>
      )}
    </Section>
  );
}
