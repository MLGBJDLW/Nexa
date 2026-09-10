/** Groups small worklet frames into 100 ms packets with bounded, ordered delivery. */
export class LiveAudioQueue {
  private chunks: Uint8Array[] = [];
  private bytes = 0;
  private queue: Uint8Array[] = [];
  private running = false;
  private closed = false;
  constructor(private readonly packetBytes: number, private readonly send: (data: Uint8Array) => Promise<void>, private readonly fail: (error: unknown) => void) {}
  append(chunk: Uint8Array): boolean {
    if (this.closed) return false;
    if (this.queue.length >= 8 || this.bytes + chunk.byteLength > this.packetBytes * 2) {
      this.close(); this.fail(new Error('Live audio is backpressured. Reconnect to continue.')); return false;
    }
    this.chunks.push(chunk); this.bytes += chunk.byteLength;
    if (this.bytes >= this.packetBytes) {
      const data = new Uint8Array(this.bytes); let offset = 0;
      for (const item of this.chunks) { data.set(item, offset); offset += item.byteLength; }
      this.chunks = []; this.bytes = 0; this.queue.push(data); void this.drain();
    }
    return true;
  }
  private async drain() {
    if (this.running) return;
    this.running = true;
    try { while (!this.closed && this.queue.length) await this.send(this.queue.shift()!); }
    catch (error) { if (!this.closed) { this.close(); this.fail(error); } }
    finally { this.running = false; }
  }
  close() { this.closed = true; this.queue = []; this.chunks = []; this.bytes = 0; }
}
