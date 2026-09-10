export const OPEN_BROWSER_WORKSPACE_EVENT = 'nexa:open-browser-workspace';

export interface OpenNexaBrowserDetail {
  url: string;
  title?: string;
}

type BrowserOpener = (url: string, signal: AbortSignal) => Promise<void>;
const owners = new Map<string, BrowserOpener>();
const pending = new Set<{ owner: string; run: (opener: BrowserOpener) => void }>();
export function registerBrowserOpener(owner: string, opener: BrowserOpener) {
  owners.set(owner, opener);
  for (const request of [...pending]) if (request.owner === owner) request.run(opener);
  return () => { if (owners.get(owner) === opener) owners.delete(owner); };
}
/** Survives route mounting; completes only when the native tab finishes loading. */
export function requestNexaBrowser(url: string, owner: string, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    const controller = new AbortController();
    let started = false;
    const finish = (error?: unknown) => {
      clearTimeout(timer); pending.delete(request); signal?.removeEventListener('abort', abort);
      if (error) { controller.abort(); reject(error); } else resolve();
    };
    const abort = () => finish(new Error('The browser preview was cancelled.'));
    const timer = setTimeout(() => finish(new Error('The browser did not finish opening the page.')), 20_000);
    const request = { owner, run: (opener: BrowserOpener) => {
      if (started || controller.signal.aborted) return;
      started = true; pending.delete(request);
      void opener(url, controller.signal).then(() => finish(), finish);
    } };
    if (signal?.aborted) { abort(); return; }
    signal?.addEventListener('abort', abort, { once: true });
    const opener = owners.get(owner);
    if (opener) request.run(opener); else pending.add(request);
  });
}

/**
 * Route an HTTP(S) page to the conversation-owned Nexa Browser Workspace.
 * A mounted BrowserDock acknowledges ownership with preventDefault().
 */
export function openNexaBrowser(url: string, title?: string): boolean {
  return !window.dispatchEvent(new CustomEvent<OpenNexaBrowserDetail>(
    OPEN_BROWSER_WORKSPACE_EVENT,
    {
      detail: { url, title },
      cancelable: true,
    },
  ));
}
