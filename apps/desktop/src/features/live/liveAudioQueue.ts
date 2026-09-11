/** Groups small worklet frames into 100 ms packets with bounded, ordered delivery. */
export class LiveAudioQueue {
  private chunks: Uint8Array[] = [];
  private bytes = 0;
  private queue: Uint8Array[] = [];
  private inFlight = 0;
  private closed = false;
  private paused = false;
  private finishing = false;
  private finished: Array<{ resolve: () => void; reject: (error: Error) => void }> = [];
  constructor(private readonly packetBytes: number, private readonly send: (data: Uint8Array) => Promise<void>, private readonly fail: (error: unknown) => void, private readonly windowSize = 1, private readonly recover?: () => void) {}
  append(chunk: Uint8Array): boolean {
    if (this.closed || this.finishing) return false;
    // Paused samples are intentionally omitted. Returning false would make the
    // recorder's contiguous-delivery guard terminal and prevent reconnection.
    if (this.paused) return true;
    if (this.queue.length >= 8 || this.bytes + chunk.byteLength > this.packetBytes * 2) {
      if (this.recover) { this.pause(); this.recover(); return true; }
      this.close(); this.fail(new Error('Live audio is backpressured. Reconnect to continue.')); return false;
    }
    this.chunks.push(chunk); this.bytes += chunk.byteLength;
    if (this.bytes >= this.packetBytes) this.packet();
    return true;
  }
  private packet() {
    if (this.bytes) {
      const data = new Uint8Array(this.bytes); let offset = 0;
      for (const item of this.chunks) { data.set(item, offset); offset += item.byteLength; }
      this.chunks = []; this.bytes = 0; this.queue.push(data); void this.drain();
    }
  }
  private drain() {
    // Invocation order is PCM order. A remote WebSocket preserves that order;
    // acknowledgements release window slots without serializing on network RTT.
    while (!this.closed && !this.paused && this.queue.length && this.inFlight < this.windowSize) {
      const chunk = this.queue.shift()!;
      this.inFlight++;
      let delivery: Promise<void>;
      try { delivery = this.send(chunk); }
      catch (error) { delivery = Promise.reject(error); }
      void delivery.catch(error => {
        if (!this.closed) { this.close(); this.fail(error); }
      }).finally(() => { this.inFlight--; this.drain(); });
    }
    if (this.finishing && !this.inFlight && !this.queue.length) {
      for (const waiter of this.finished.splice(0)) waiter.resolve();
    }
  }
  /** After recorder.stopRecording(), include its final short packet and await all ACKs. */
  finish(): Promise<void> {
    if (this.closed || this.paused) return Promise.reject(new Error('Audio delivery was interrupted'));
    this.finishing = true;
    this.packet();
    return new Promise((resolve, reject) => { this.finished.push({ resolve, reject }); this.drain(); });
  }
  close() {
    this.closed = true; this.queue = []; this.chunks = []; this.bytes = 0;
    for (const waiter of this.finished.splice(0)) waiter.reject(new Error('Audio delivery was interrupted'));
  }
  pause() { this.paused = true; this.queue = []; this.chunks = []; this.bytes = 0; }
  resume() { this.paused = false; }
}
