import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { encodeLiveAudio, type LiveEvent, type LiveTransport } from './liveTransport';
import { cachedModelChoices } from '../models/modelChoices';

export const desktopLiveTransport: LiveTransport = {
  connections: () => invoke('live_connections_cmd'),
  models: cachedModelChoices(connectionId => invoke('model_choices_cmd', { connectionId })),
  start: request => invoke('start_live_cmd', { request }),
  snapshot: sessionId => invoke('live_snapshot_cmd', { sessionId }),
  frame: (sessionId, mimeType, data) => invoke('live_frame_cmd', { sessionId, mimeType, data }),
  audio: (sessionId, data) => invoke('live_audio_cmd', { sessionId, data: encodeLiveAudio(data) }),
  stop: sessionId => invoke('stop_live_cmd', { sessionId }),
  summarize: (sessionId, connectionId, modelSelection) => invoke('summarize_live_cmd', { sessionId, connectionId, modelSelection }),
  list: () => invoke('list_live_records_cmd'),
  load: sessionId => invoke('load_live_record_cmd', { sessionId }),
  subscribe: listener => listen<LiveEvent>('live:event', event => listener(event.payload)),
};
