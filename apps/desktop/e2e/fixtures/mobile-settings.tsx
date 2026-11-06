import { createRoot } from 'react-dom/client';
import { I18nProvider, useTranslation } from '../../src/i18n';
import PhoneSettingsSheet from '../../../mobile/src/PhoneSettingsSheet';
import { PhoneWebSettings } from '../../../mobile/src/PhoneWebSettings';
import { PhoneSpeechSettings } from '../../../mobile/src/PhoneSpeechSettings';
import { ConversationStorageManager } from '../../src/components/settings/ConversationStorageManager';
import { inputDefaults, outputDefaults } from '../../../mobile/src/PhoneCloudSpeechFields';
import type { StorageService } from '../../src/lib/api';
import { MobileDialog } from '../../src/features/remote/MobileChatShell';
import '../../../mobile/src/mobile.css';

function LocaleSwitch() { const {setLocale}=useTranslation();return <button onClick={()=>setLocale('zh-CN')}>Switch UI language</button>; }

export function renderMobileSettings(kind = 'web') {
  localStorage.setItem('nexa-locale', 'en');
  const calls: unknown[] = [];
  (window as any).__MOBILE_SETTINGS__ = calls;
  const connections = ['Personal Bailian', 'Work Bailian'].map((name, index) => ({
    id: 'synced-' + index, name,
    input: [{ presetId: 'alibaba-qwen-asr', baseUrl: 'https://dashscope.aliyuncs.com/compatible-mode/v1' }],
    output: [{ presetId: 'dashscope-cosyvoice', baseUrl: 'https://dashscope.aliyuncs.com/api-ws/v1/inference' }],
  }));
  let speech = { mode: 'system', outputMode: 'system', language: '', systemAvailable: false, onDeviceAvailable: false, microphoneGranted: true, speechToText: inputDefaults(), textToSpeech: outputDefaults(),
    speechConnections: new URLSearchParams(location.search).has('synced') ? connections : [] };
  if (new URLSearchParams(location.search).has('legacy')) {
    speech.mode = 'cloud';
    speech.outputMode = 'cloud';
    speech.speechToText = { ...speech.speechToText, apiKey: '__NEXA_SAVED_KEY__', model: 'private-stt' };
    speech.textToSpeech = { ...speech.textToSpeech, provider: 'qwen', apiStyle: 'dashscope_speech',
      baseUrl: connections[0].output[0].baseUrl, model: 'private-tts', voice: 'private-voice' };
  }
  let finish: ((value: {text:string})=>void) | null = null;
  (window as any).__TAURI_INTERNALS__ = {
    invoke: async (command: string, args: any) => {
      calls.push({ command, args });
      if (command === 'mobile_storage') return {totalBytes:1024,categories:[{id:'chats',bytes:1024}],activeConversations:1,archivedConversations:0};
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
  if (kind === 'all') {
    createRoot(host).render(<I18nProvider><PhoneSettingsSheet open state={{version:'fixture',settings:null,connections:[],hosts:[],capabilities:{accessibility:true,notifications:true}}} syncBusy={{}} syncErrors={{}} onSync={async()=>({supported:true,checkedAt:'',connections:0,received:0,sent:0,conflicts:0,deviceOnly:0})} onClose={()=>{}} onChanged={async()=>{}} onForgot={()=>{}}/></I18nProvider>);
    return;
  }
  createRoot(host).render(<I18nProvider><MobileDialog title="Settings" onClose={() => {}}>{kind === 'speech' ? <><LocaleSwitch/><PhoneSpeechSettings open/></> : kind === 'storage' ? <ConversationStorageManager mobile service={storage} onClose={()=>{}} onChanged={()=>{}}/> : <PhoneWebSettings/>}</MobileDialog></I18nProvider>);
}
