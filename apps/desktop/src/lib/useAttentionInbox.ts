import { useSyncExternalStore } from 'react';
import { attentionInbox } from './attentionInbox';

export function useAttentionInbox() {
  return useSyncExternalStore(attentionInbox.subscribe, attentionInbox.getSnapshot, attentionInbox.getSnapshot);
}
