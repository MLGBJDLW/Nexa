import { useEffect } from 'react';
import * as api from './api';
import { interactionStore } from './interactionStore';
import { streamStore } from './streamStore';
import { attentionInbox } from './attentionInbox';

/** One application-lifetime refresh owner, including while ChatPage is unmounted. */
export function InteractionSync() {
  useEffect(() => {
    let active = true;
    let inFlight = false;
    let dirty = false;
    const revisions = new Map<string, string>();
    const refresh = async () => {
      if (!active) return;
      if (inFlight) { dirty = true; return; }
      inFlight = true;
      const approvalRevision = attentionInbox.getApprovalRevision();
      try {
        const [interactions, approvals] = await Promise.allSettled([
          api.listInteractionRequests(null, false),
          api.listPendingToolApprovals(),
        ]);
        if (!active) return;
        if (interactions.status === 'fulfilled' && Array.isArray(interactions.value)) interactionStore.replaceRequests(null, interactions.value);
        if (approvals.status === 'fulfilled' && Array.isArray(approvals.value)
          && !attentionInbox.replaceApprovalSnapshot(approvals.value, approvalRevision)) dirty = true;
      } catch { /* Keep the last committed request while the host reconnects. */ }
      finally {
        inFlight = false;
        if (active && dirty) { dirty = false; void refresh(); }
      }
    };
    const unsubscribe = streamStore.subscribe(conversationId => {
      const stream = streamStore.getStream(conversationId);
      const run = stream?.taskRun;
      const revision = `${run?.id ?? ''}:${run?.status ?? ''}:${stream?.isStreaming ?? false}`;
      if (revisions.get(conversationId) === revision) return;
      revisions.set(conversationId, revision);
      void refresh();
    });
    const onVisible = () => { if (document.visibilityState !== 'hidden') void refresh(); };
    const timer = window.setInterval(onVisible, 15_000);
    document.addEventListener('visibilitychange', onVisible);
    void refresh();
    return () => {
      active = false;
      unsubscribe();
      window.clearInterval(timer);
      document.removeEventListener('visibilitychange', onVisible);
    };
  }, []);
  return null;
}
