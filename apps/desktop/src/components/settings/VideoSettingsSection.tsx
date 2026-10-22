import { SettingsRow } from './SettingsRow';
import {
  AlertTriangle,
  CheckCircle,
  Film,
  Loader2,
  Save,
  Trash2,
  XCircle,
} from 'lucide-react';
import { NexaSelect } from '../ui/overlay';
import { useTranslation } from '../../i18n';
import type { FfmpegDownloadProgress, VideoConfig } from '../../types/video';
import { Badge } from '../ui/Badge';
import { Button } from '../ui/Button';
import { ConfirmDialog } from '../ui/ConfirmDialog';
import { Input } from '../ui/Input';
import { CollapsiblePanel, Section } from './SettingsSection';
import { SettingsToggle } from './SettingsToggle';

interface VideoSettingsSectionProps {
  videoConfig: VideoConfig | null;
  ffmpegAvailable: boolean | null;
  ffmpegDownloading: boolean;
  ffmpegProgress: FfmpegDownloadProgress | null;
  whisperModelExists: boolean | null;
  videoSaveLoading: boolean;
  showAdvancedVideo: boolean;
  deleteModelConfirmOpen: boolean;
  onConfigChange: (config: VideoConfig) => void;
  onMarkDirty: () => void;
  onFfmpegDownload: () => void;
  onAdvancedToggle: () => void;
  onRequestDeleteModel: () => void;
  onCloseDeleteModel: () => void;
  onConfirmDeleteModel: () => void;
  onSave: () => void;
}

export function VideoSettingsSection({
  videoConfig,
  ffmpegAvailable,
  ffmpegDownloading,
  ffmpegProgress,
  whisperModelExists,
  videoSaveLoading,
  showAdvancedVideo,
  deleteModelConfirmOpen,
  onConfigChange,
  onMarkDirty,
  onFfmpegDownload,
  onAdvancedToggle,
  onRequestDeleteModel,
  onCloseDeleteModel,
  onConfirmDeleteModel,
  onSave,
}: VideoSettingsSectionProps) {
  const { t } = useTranslation();

  const updateConfig = (patch: Partial<VideoConfig>) => {
    if (!videoConfig) return;
    onConfigChange({ ...videoConfig, ...patch });
    onMarkDirty();
  };

  return (
    <Section
      icon={<Film size={20} />}
      title={t('settings.videoSection')}
      delay={0.06}
      collapsible
      defaultOpen={false}
      summary={videoConfig ? (
        <span className="rounded-full border border-border/60 bg-surface-2 px-2 py-1 text-[11px] text-text-secondary">
          {videoConfig.enabled ? t('settings.skillFilterEnabled') : t('settings.skillFilterDisabled')}
        </span>
      ) : undefined}
    >
      {videoConfig ? (
        <div className="min-w-0 space-y-3">
          {/* Enable toggle */}
          <SettingsRow label={t('settings.videoEnabled')} description={t('settings.videoEnabledDesc')}><SettingsToggle label={t('settings.videoEnabled')} checked={videoConfig.enabled} onChange={enabled => updateConfig({ enabled })} /></SettingsRow>

          <div className="divide-y divide-border/50">
            <SettingsRow label={t('settings.videoTranscriptionMode')} description={t('settings.videoTranscriptionModeDesc')}><NexaSelect aria-label={t('settings.videoTranscriptionMode')}
                value={videoConfig.transcriptionMode}
                onChange={(event) => updateConfig({
                  transcriptionMode: event.target.value as VideoConfig['transcriptionMode'],
                })}
                className="w-full rounded-lg border border-border bg-surface-0 px-3 py-2 text-sm text-text-primary"
              >
                <option value="inherit_speech_to_text">{t('settings.videoTranscriptionInherit')}</option>
                <option value="local_whisper">{t('settings.videoTranscriptionLocal')}</option>
                <option value="disabled">{t('settings.videoTranscriptionDisabled')}</option>
              </NexaSelect></SettingsRow>
            <SettingsRow label={t('settings.videoFailurePolicy')} description={t('settings.videoFailurePolicyDesc')}><NexaSelect aria-label={t('settings.videoFailurePolicy')}
                value={videoConfig.failurePolicy}
                onChange={(event) => updateConfig({
                  failurePolicy: event.target.value as VideoConfig['failurePolicy'],
                })}
                className="w-full rounded-lg border border-border bg-surface-0 px-3 py-2 text-sm text-text-primary"
              >
                <option value="best_effort">{t('settings.videoFailureBestEffort')}</option>
                <option value="require_transcript">{t('settings.videoFailureRequireTranscript')}</option>
              </NexaSelect></SettingsRow>
          </div>

          {/* FFmpeg Status */}
          <div className="flex items-center gap-2 text-sm">
            <span className="text-text-secondary">{t('settings.videoFfmpegStatus')}:</span>
            {ffmpegAvailable === null ? (
              <Loader2 size={14} className="animate-spin text-text-tertiary" />
            ) : ffmpegAvailable ? (
              <Badge variant="default" className="gap-1">
                <CheckCircle size={12} className="text-success" />
                {t('settings.videoFfmpegAvailable')}
              </Badge>
            ) : ffmpegDownloading ? (
              <Badge variant="default" className="gap-1">
                <Loader2 size={12} className="animate-spin" />
                {t('settings.videoFfmpegDownloading')}
              </Badge>
            ) : (
              <Badge variant="default" className="gap-1">
                <XCircle size={12} className="text-danger" />
                {t('settings.videoFfmpegNotFound')}
              </Badge>
            )}
          </div>
          {ffmpegDownloading && ffmpegProgress && (
            <div className="space-y-1">
              <div className="h-2 w-full overflow-hidden rounded-full bg-surface-3">
                <div
                  className="h-full rounded-full bg-accent transition-all duration-300"
                  style={{ width: `${Math.min(ffmpegProgress.progressPct, 100)}%` }}
                />
              </div>
              <p className="text-xs text-text-tertiary">{ffmpegProgress.status}</p>
            </div>
          )}
          {ffmpegAvailable === false && !ffmpegDownloading && (
            <div className="flex items-start gap-2 rounded-lg border border-warning/30 bg-warning/5 p-2">
              <AlertTriangle size={14} className="mt-0.5 shrink-0 text-warning" />
              <div className="flex flex-col gap-2">
                <p className="text-xs text-warning">{t('settings.videoFfmpegHint')}</p>
                <Button
                  size="sm"
                  variant="secondary"
                  onClick={onFfmpegDownload}
                  disabled={ffmpegDownloading}
                >
                  {t('settings.videoFfmpegDownload')}
                </Button>
              </div>
            </div>
          )}

          {/* Local Whisper language */}
          {videoConfig.transcriptionMode === 'local_whisper' && <SettingsRow label={t('settings.videoLanguage')} description={t('settings.videoLanguageDesc')}><Input aria-label={t('settings.videoLanguage')}
              type="text"
              value={videoConfig.language ?? ''}
              onChange={(e) => updateConfig({ language: e.target.value || null })}
              placeholder="en, zh, ja..."
              className="w-full"
            /></SettingsRow>}

          {/* Translate to English */}
          {videoConfig.transcriptionMode === 'local_whisper' && <SettingsRow label={t('settings.videoTranslate')} description={t('settings.videoTranslateDesc')}>
            <SettingsToggle label={t('settings.videoTranslate')} checked={videoConfig.translateToEnglish} onChange={translateToEnglish => updateConfig({ translateToEnglish })} />
          </SettingsRow>}

          {/* Frame Extraction */}
          <SettingsRow label={t('settings.videoFrameExtraction')} description={t('settings.videoFrameExtractionDesc')}><SettingsToggle label={t('settings.videoFrameExtraction')} checked={videoConfig.frameExtractionEnabled} onChange={frameExtractionEnabled => updateConfig({ frameExtractionEnabled })} /></SettingsRow>

          {/* Frame Interval */}
          {videoConfig.frameExtractionEnabled && (
            <SettingsRow label={t('settings.videoFrameInterval')} description={t('settings.videoFrameIntervalDesc')}><Input aria-label={t('settings.videoFrameInterval')}
                type="number"
                value={videoConfig.frameIntervalSecs}
                onChange={(e) => updateConfig({ frameIntervalSecs: parseInt(e.target.value) || 30 })}
                className="w-full"
              /></SettingsRow>
          )}

          <CollapsiblePanel title={t('settings.videoAdvanced')} open={showAdvancedVideo} onOpenChange={onAdvancedToggle}>
              <div className="space-y-3">
                {/* Prefer Embedded Subtitles */}
                <SettingsRow label={t('settings.videoPreferSubtitles')} description={t('settings.videoPreferSubtitlesDesc')}><SettingsToggle label={t('settings.videoPreferSubtitles')} checked={videoConfig.preferEmbeddedSubtitles} onChange={preferEmbeddedSubtitles => updateConfig({ preferEmbeddedSubtitles })} /></SettingsRow>

                {/* Scene Detection Threshold */}
                <SettingsRow label={t('settings.videoSceneThreshold')} description={t('settings.videoSceneThresholdDesc')}><div className="flex items-center gap-3">
                    <input aria-label={t('settings.videoSceneThreshold')}
                      type="range"
                      min={10}
                      max={90}
                      step={5}
                      value={Math.round(videoConfig.sceneThreshold * 100)}
                      onChange={(e) => updateConfig({ sceneThreshold: parseInt(e.target.value) / 100 })}
                      className="min-w-0 flex-1 accent-accent"
                    />
                    <span className="text-xs text-text-secondary w-10 text-right">{videoConfig.sceneThreshold.toFixed(2)}</span>
                  </div></SettingsRow>

              </div>
          </CollapsiblePanel>

          {/* Save button */}
          <div data-settings-actions className="flex items-center justify-between pt-2">
            {whisperModelExists && (
              <Button
                variant="ghost"
                size="sm"
                icon={<Trash2 size={14} />}
                onClick={onRequestDeleteModel}
                className="text-danger hover:bg-danger/10"
              >
                {t('settings.videoDeleteModel')}
              </Button>
            )}
            <div className="flex-1" />
            <Button
              variant="primary"
              size="sm"
              icon={<Save size={16} />}
              loading={videoSaveLoading}
              onClick={onSave}
            >
              {t('settings.videoSave')}
            </Button>
          </div>

          {/* Delete model confirmation */}
          <ConfirmDialog
            open={deleteModelConfirmOpen}
            onClose={onCloseDeleteModel}
            onConfirm={onConfirmDeleteModel}
            title={t('settings.videoDeleteModel')}
            message={t('settings.videoDeleteConfirm')}
            confirmText={t('common.delete')}
            variant="danger"
          />
        </div>
      ) : (
        <div className="flex flex-col items-center justify-center py-12 text-text-tertiary">
          <Film size={32} className="mb-3 opacity-50" />
          <p className="text-sm font-medium text-text-primary mb-1">{t('settings.videoUnavailable')}</p>
          <p className="text-xs text-center max-w-md">
            {t('settings.videoUnavailableRequires')}{' '}
            {t('settings.videoUnavailableRebuild')}
          </p>
        </div>
      )}
    </Section>
  );
}
