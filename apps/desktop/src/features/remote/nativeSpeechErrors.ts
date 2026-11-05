import type { TranslationKey } from '../../i18n';

const messages = {
  speech_configuration: 'remote.speechConfiguration',
  speech_cloud_failed: 'remote.speechCloudFailed',
  speech_interrupted: 'remote.speechInterrupted',
  speech_on_device_unavailable: 'remote.speechOnDeviceUnavailable',
  speech_permission: 'remote.speechPermission', speech_unavailable: 'remote.speechUnavailable',
  speech_network: 'remote.speechNetwork', speech_no_match: 'remote.speechNoMatch',
  speech_timeout: 'remote.speechTimeout', speech_language: 'remote.speechLanguage',
  speech_busy: 'remote.speechBusy',
} as const satisfies Record<string, TranslationKey>;

export function nativeSpeechErrorKey(error: unknown): TranslationKey {
  const message = String(error);
  return Object.entries(messages).find(([code]) => message.includes(code))?.[1] ?? 'remote.speechFailed';
}
