import { createRoot } from 'react-dom/client';
import { I18nProvider } from '../../src/i18n';
import { PhoneWebSettings } from '../../../mobile/src/PhoneWebSettings';
import { PhoneSpeechSettings } from '../../../mobile/src/PhoneSpeechSettings';
import { ConversationStorageManager } from '../../src/components/settings/ConversationStorageManager';
import { inputDefaults, outputDefaults } from '../../../mobile/src/PhoneCloudSpeechFields';
import type { StorageService } from '../../src/lib/api';
import { MobileDialog } from '../../src/features/remote/MobileChatShell';
import '../../../mobile/src/mobile.css';

export function renderMobileSettings(kind = 'web') {
  localStorage.setItem('nexa-locale', 'en');
  const calls: unknown[] = [];
  (window as any).__MOBILE_SETTINGS__ = calls;
  let speech = { mode: 'system', outputMode: 'system', language: '', systemAvailable: false, onDeviceAvailable: false, microphoneGranted: true, speechToText: inputDefaults(), textToSpeech: outputDefaults() };
  let finish: ((value: {text:string})=>void) | null = null;
  (window as any).__TAURI_INTERNALS__ = {
    invoke: async (command: string, args: any) => {
      calls.push({ command, args });
      if (command === 'mobile_speech') {
        if (args.action === 'configure') { speech = { ...speech, ...args }; return speech; }
        if (args.action === 'listen') return new Promise(resolve => { finish = resolve; });
        if (args.action === 'finish') { finish?.({text:'你好，手机自配语音已转写'}); finish = null; return {}; }
        if (args.action === 'cancel') { finish?.({text:''}); finish = null; return {}; }
        return speech;
      }
      if (args.action === 'status') return [];
      return { webSearch: args.webSearch ?? { executionMode: 'auto', providerNativeEngine: 'auto', providerMode: 'built_in_first', providerProfile: 'default', reranker: 'auto', customProviders: [] } };
    },
  };
  const host = document.createElement('div'); host.className = 'mobile-app';
  document.body.appendChild(host);
  const storage = { page:async()=>({entries:[{id:'chat',title:'Saved phone conversation'}],next:null}), archive:async(ids:string[])=>calls.push({action:'archive',ids}),remove:async(ids:string[])=>calls.push({action:'remove',ids}) } as unknown as StorageService;
  createRoot(host).render(<I18nProvider><MobileDialog title="Settings" onClose={() => {}}>{kind === 'speech' ? <PhoneSpeechSettings open/> : kind === 'storage' ? <ConversationStorageManager mobile service={storage} onClose={()=>{}} onChanged={()=>{}}/> : <PhoneWebSettings/>}</MobileDialog></I18nProvider>);
}
