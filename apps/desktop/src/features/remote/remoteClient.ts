import { cachedModelChoices, type ModelChoices } from '../models/modelChoices';
export interface RemoteEndpoint {
  url: string;
  kind: "lan" | "tunnel" | "ssh";
}
export interface RemoteManifest {
  serverId: string;
  endpoints: RemoteEndpoint[];
  reconnectGraceSeconds: number;
}
export interface PairedRemote {
  token: string;
  device: { id: string; name: string };
  manifest: RemoteManifest;
  preferredEndpoint?: string;
}
export interface RemoteEnvelope {
  event: string;
  payload: any;
}
export interface RemoteConnection {
  phase: "connecting" | "connected" | "reconnecting" | "revoked" | "closed";
  endpoint: RemoteEndpoint | null;
  latencyMs?: number;
}
export function remoteRouteHeaders(origin: string): Record<string, string> {
  const host = new URL(origin).hostname;
  return ['.free.pinggy.net', '.pinggy.link', '.pinggy.online', '.run.pinggy-free.link'].some(suffix => host.endsWith(suffix))
    ? { 'X-Pinggy-No-Screen':'1' } : {};
}
export class RemoteNetworkError extends Error {}
const timeoutSignal = (ms: number) => AbortSignal.timeout(ms);

/** A page stays on its trusted origin while the transport chooses a reachable route. */
export class RemoteClient {
  readonly models = cachedModelChoices(id => this.rpc<ModelChoices>('models.list', { connectionId:id }));
  private socket: WebSocket | null = null;
  private disposed = false;
  private connecting: Promise<void> | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private monitor: ReturnType<typeof setInterval> | null = null;
  private lastMessage = Date.now();
  private lastProbe = 0;
  private failures = 0;
  private sequence = 0;
  private probeLatencies = new Map<string, number>();
  private lastSwitch = 0;
  private listeners = new Set<(event: RemoteEnvelope) => void>();
  private connectionListeners = new Set<(state: RemoteConnection) => void>();
  private audioPending = new Map<
    number,
    {
      resolve: () => void;
      reject: (error: Error) => void;
      timer: ReturnType<typeof setTimeout>;
    }
  >();
  state: RemoteConnection = { phase: "connecting", endpoint: null };
  constructor(
    readonly paired: PairedRemote,
    private readonly changed: (paired: PairedRemote) => void = () => {},
  ) {}
  subscribe(listener: (event: RemoteEnvelope) => void) {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }
  subscribeConnection(listener: (state: RemoteConnection) => void) {
    this.connectionListeners.add(listener);
    listener(this.state);
    return () => {
      this.connectionListeners.delete(listener);
    };
  }
  private setState(
    phase: RemoteConnection["phase"],
    endpoint = this.state.endpoint,
  ) {
    this.state = { phase, endpoint, latencyMs:endpoint ? this.probeLatencies.get(endpoint.url) : undefined };
    for (const listener of this.connectionListeners) listener(this.state);
  }
  async start() {
    if (!this.monitor) {
      this.monitor = setInterval(() => {
        if (this.socket?.readyState === WebSocket.OPEN) {
          if (Date.now() - this.lastMessage > 15_000) {
            this.socket.close();
            this.lost();
            return;
          }
          this.socket.send("ping");
          if (
            Date.now() - this.lastProbe > 30_000 &&
            this.state.endpoint?.kind !== "lan"
          ) {
            this.lastProbe = Date.now();
            void this.connect(true).catch(() => {});
          }
        }
      }, 5_000);
      window.addEventListener("online", this.online);
    }
    return this.connect();
  }
  private online = () => {
    this.failures = 0;
    void this.connect(true).catch(() => {});
  };
  async preferEndpoint(url: string | null) {
    if (url && !this.paired.manifest.endpoints.some(endpoint => endpoint.url === url)) throw new Error('The selected connection is unavailable.');
    this.paired.preferredEndpoint = url || undefined;
    this.changed(this.paired);
    this.lastSwitch = 0;
    await this.connect(true);
  }
  private async *candidates(): AsyncGenerator<RemoteEndpoint> {
    const allowed = this.paired.manifest.endpoints.filter((endpoint) => {
      try {
        const url = new URL(endpoint.url);
        if (
          url.username ||
          url.password ||
          url.search ||
          url.hash ||
          url.pathname !== "/"
        )
          return false;
        return (
          url.protocol === "https:" ||
          (endpoint.kind === "ssh" &&
            endpoint.url === location.origin &&
            ["127.0.0.1", "localhost", "[::1]"].includes(url.hostname))
        );
      } catch {
        return false;
      }
    });
    const pending = allowed.map((endpoint) => ({
      endpoint,
      result: (async () => {
        const started = performance.now();
        try {
          const response = await fetch(`${endpoint.url}/api/health`, {
            signal: timeoutSignal(endpoint.kind === "tunnel" ? 6000 : 1800),
            cache: "no-store",
            credentials: "omit",
            headers: remoteRouteHeaders(endpoint.url),
          });
          const health = await response.json();
          if (!response.ok || health.serverId !== this.paired.manifest.serverId) return null;
          this.probeLatencies.set(endpoint.url, Math.round(performance.now() - started));
          return endpoint;
        } catch {
          return null;
        }
      })(),
    }));
    // Probe concurrently without making a ready LAN route wait for the public Internet.
    const preferred = pending.find(probe => probe.endpoint.url === this.paired.preferredEndpoint);
    if (preferred) { const endpoint = await preferred.result; if (endpoint) yield endpoint; }
    for (const kind of ["lan", "tunnel", "ssh"] as const) {
      const probes = pending.filter(probe => probe.endpoint.kind === kind && probe !== preferred);
      const remaining = new Map(probes.map((probe, index) => [index, probe.result.then(endpoint => ({ index, endpoint }))]));
      while (remaining.size) {
        const { index, endpoint } = await Promise.race(remaining.values());
        remaining.delete(index);
        if (endpoint) yield endpoint;
      }
    }
  }
  private connect(probe = false): Promise<void> {
    if (this.disposed || this.state.phase === "revoked")
      return Promise.reject(
        new RemoteNetworkError("This remote connection is closed."),
      );
    if (this.connecting) return this.connecting;
    if (
      !probe &&
      this.socket?.readyState === WebSocket.OPEN &&
      this.state.phase === "connected"
    )
      return Promise.resolve();
    this.connecting = (async () => {
      for await (const endpoint of this.candidates()) {
        if (
          probe &&
          endpoint.url === this.state.endpoint?.url &&
          this.socket?.readyState === WebSocket.OPEN
        )
          return;
        if (probe && this.socket?.readyState === WebSocket.OPEN && endpoint.kind !== 'lan'
          && this.paired.preferredEndpoint !== endpoint.url
          && this.paired.manifest.endpoints.some(candidate => candidate.url === this.state.endpoint?.url)) {
          const current = this.probeLatencies.get(this.state.endpoint!.url) ?? Infinity;
          const candidate = this.probeLatencies.get(endpoint.url) ?? Infinity;
          // Hysteresis avoids disturbing a healthy Live session for small timing fluctuations.
          if (Date.now() - this.lastSwitch < 60_000 || candidate + 200 >= current * 0.65) continue;
        }
        try {
          await this.openSocket(endpoint);
          this.failures = 0;
          return;
        } catch {
          if (this.state.phase === "revoked" || this.disposed) break;
        }
      }
      if (this.socket?.readyState !== WebSocket.OPEN) {
        this.lost();
        throw new RemoteNetworkError(
          "Nexa is temporarily unreachable. Keep this page open to reconnect.",
        );
      }
    })().finally(() => {
      this.connecting = null;
    });
    return this.connecting;
  }
  private openSocket(endpoint: RemoteEndpoint): Promise<void> {
    return new Promise((resolve, reject) => {
      const socket = new WebSocket(
        `${endpoint.url.replace(/^http/, "ws")}/api/events`,
      );
      let accepted = false;
      let expired = false;
      const timer = setTimeout(() => {
        expired = true;
        socket.close();
        reject(new RemoteNetworkError("Connection timed out."));
      }, 5_000);
      socket.onopen = () =>
        socket.send(JSON.stringify({ token: this.paired.token }));
      socket.onmessage = (message) => {
        let event: RemoteEnvelope;
        try {
          event = JSON.parse(String(message.data));
        } catch {
          return;
        }
        if (event.event === "connection:ready") {
          if (
            expired || this.disposed || socket.readyState !== WebSocket.OPEN ||
            event.payload.manifest.serverId !== this.paired.manifest.serverId
          ) {
            socket.close();
            reject(new RemoteNetworkError("The remote identity changed."));
            return;
          }
          accepted = true;
          clearTimeout(timer);
          const previous = this.socket;
          if (previous && previous !== socket) {
            this.setState("reconnecting");
            this.clearAudio();
          }
          this.socket = socket;
          this.lastSwitch = Date.now();
          this.lastMessage = Date.now();
          this.paired.manifest = event.payload.manifest;
          this.changed(this.paired);
          this.setState("connected", endpoint);
          previous?.close();
          resolve();
          for (const listener of this.listeners)
            listener({ event: "connection:resync", payload: {} });
          return;
        }
        if (socket !== this.socket) return;
        this.lastMessage = Date.now();
        if (event.event === "connection:manifest") {
          this.paired.manifest = event.payload;
          this.changed(this.paired);
          if (!this.paired.manifest.endpoints.some(endpoint => endpoint.url === this.state.endpoint?.url)) void this.connect(true).catch(() => {});
        }
        if (
          event.event === "connection:ack" ||
          event.event === "connection:input-error"
        ) {
          const pending = this.audioPending.get(event.payload.id);
          if (pending) {
            this.audioPending.delete(event.payload.id);
            clearTimeout(pending.timer);
            if (event.event === "connection:ack") pending.resolve();
            else pending.reject(new Error(event.payload.message));
          }
        }
        for (const listener of this.listeners) listener(event);
      };
      socket.onerror = () => {
        expired = true;
        socket.close();
        clearTimeout(timer);
        reject(new RemoteNetworkError("Connection failed."));
      };
      socket.onclose = (event) => {
        clearTimeout(timer);
        if (event.code === 1008) this.revoked();
        if (!accepted)
          reject(new RemoteNetworkError("Device authentication failed."));
        if (this.socket === socket) {
          this.socket = null;
          this.lost();
        }
      };
    });
  }
  private clearAudio() {
    for (const pending of this.audioPending.values()) {
      clearTimeout(pending.timer);
      pending.resolve();
    }
    this.audioPending.clear();
  }
  private lost() {
    if (this.disposed || this.state.phase === "revoked") return;
    this.setState("reconnecting");
    this.clearAudio();
    if (this.reconnectTimer) return;
    this.reconnectTimer = setTimeout(
      () => {
        this.reconnectTimer = null;
        void this.connect().catch(() => {});
      },
      Math.min(15_000, 800 * 2 ** Math.min(this.failures++, 4)),
    );
  }
  private revoked() {
    this.setState("revoked");
    this.socket?.close();
    this.clearAudio();
  }
  async rpc<T>(method: string, params?: unknown): Promise<T> {
    const retryable =
      /^(connections\.list|models\.list|chat\.(list|read|message|resume|start|stop)|live\.(connections|snapshot|list|load|stop)|interactions\.list|approvals\.list|connection\.certificate)$/.test(
        method,
      );
    for (let attempt = 0; attempt < (retryable ? 2 : 1); attempt++) {
      await this.connect();
      const endpoint = this.state.endpoint!;
      try {
        const response = await fetch(`${endpoint.url}/api/rpc`, {
          method: "POST",
          headers: {
            ...remoteRouteHeaders(endpoint.url),
            Authorization: `Bearer ${this.paired.token}`,
            "Content-Type": "application/json",
          },
          body: JSON.stringify({
            id: ++this.sequence,
            method,
            ...(params === undefined ? {} : { params }),
          }),
          signal: timeoutSignal(method === "live.summarize" || (method === 'chat.start' && (params as { attachments?: unknown[] })?.attachments?.length) ? 145_000 : 35_000),
          credentials: "omit",
        });
        if (response.status === 401) this.revoked();
        if ([502, 503, 504].includes(response.status))
          throw new RemoteNetworkError("The remote route is reconnecting.");
        const result = await response.json();
        if (!response.ok)
          throw new Error(
            result.error || `Request failed (${response.status})`,
          );
        return result.result as T;
      } catch (error) {
        if (
          error instanceof RemoteNetworkError ||
          error instanceof TypeError ||
          (error instanceof DOMException &&
            ["TimeoutError", "AbortError"].includes(error.name))
        ) {
          this.socket?.close();
          this.socket = null;
          this.lost();
          if (retryable && attempt === 0) continue;
          throw new RemoteNetworkError(
            "The connection changed. Reconnect and retry this operation.",
          );
        }
        throw error;
      }
    }
    throw new RemoteNetworkError("Nexa is unreachable.");
  }
  recoverAudio() {
    this.socket?.close();
    this.lost();
  }
  audio(sessionId: string, data: string, method: 'live.audio' | 'voice.audio' = 'live.audio'): Promise<void> {
    if (
      this.state.phase !== "connected" ||
      this.socket?.readyState !== WebSocket.OPEN
    )
      return Promise.resolve();
    if (this.audioPending.size >= 16 || this.socket.bufferedAmount > 128 * 1024) {
      // A stalled route pauses capture through the existing reconnect protocol.
      // Never replay stale microphone data on the replacement connection.
      this.socket.close();
      this.lost();
      return Promise.resolve();
    }
    const id = ++this.sequence;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.socket?.close();
        this.lost();
      }, 3_000);
      this.audioPending.set(id, { resolve, reject, timer });
      this.socket!.send(
        JSON.stringify({
          id,
          method,
          params: { sessionId, data },
        }),
      );
    });
  }
  close() {
    this.disposed = true;
    if (this.monitor) clearInterval(this.monitor);
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
    window.removeEventListener("online", this.online);
    this.socket?.close();
    this.socket = null;
    this.clearAudio();
    this.setState("closed");
  }
}
