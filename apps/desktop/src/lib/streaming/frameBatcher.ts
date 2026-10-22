export type FrameRequest = (callback: () => void) => () => void;

function requestBrowserFrame(callback: () => void): () => void {
  if (typeof globalThis.requestAnimationFrame === 'function') {
    const frame = globalThis.requestAnimationFrame(callback);
    return () => globalThis.cancelAnimationFrame(frame);
  }
  const timer = setTimeout(callback, 16);
  return () => clearTimeout(timer);
}

/** Coalesce ordinary stream mutations into one subscriber notification per frame. */
export class ConversationFrameBatcher {
  private readonly pending = new Set<string>();
  private scheduled: { cancel: () => void } | null = null;

  constructor(
    private readonly flush: (conversationId: string) => void,
    private readonly requestFrame: FrameRequest = requestBrowserFrame,
  ) {}

  schedule(conversationId: string): void {
    this.pending.add(conversationId);
    if (this.scheduled) return;

    const batch = { cancel: () => {} };
    this.scheduled = batch;
    let deadline: ReturnType<typeof setTimeout> | undefined;
    const deliver = () => {
      if (this.scheduled !== batch) return;
      this.scheduled = null;
      batch.cancel();
      const conversations = [...this.pending];
      this.pending.clear();
      conversations.forEach(this.flush);
    };
    // Occluded native WebViews can suspend requestAnimationFrame. A bounded
    // timer keeps task state responsive, and the batch token rejects a late
    // callback when the WebView starts painting again.
    deadline = setTimeout(deliver, 100);
    const cancelFrame = this.requestFrame(deliver);
    batch.cancel = () => {
      clearTimeout(deadline);
      cancelFrame();
    };
    // Also support synchronous schedulers without retaining their handles.
    if (this.scheduled !== batch) batch.cancel();
  }

  /** Urgent state must be visible immediately and must not be replayed next frame. */
  flushNow(conversationId: string): void {
    this.pending.delete(conversationId);
    if (this.pending.size === 0) {
      this.scheduled?.cancel();
      this.scheduled = null;
    }
    this.flush(conversationId);
  }
}
