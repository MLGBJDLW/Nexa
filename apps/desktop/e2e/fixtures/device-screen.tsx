import { useMemo, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { DeviceScreen, type ScreenFrame, type ScreenTransport } from '../../src/features/remote/DeviceScreen';
import { DesktopDeviceScreen } from '../../src/features/remote/DesktopDeviceScreen';
import { I18nProvider } from '../../src/i18n';

const phone = new URLSearchParams(location.search).has('phone');
const desktop = new URLSearchParams(location.search).has('desktop');
const canvas = document.createElement('canvas');
canvas.width = phone ? 800 : 1280; canvas.height = phone ? 1600 : 720;
const context = canvas.getContext('2d')!;
context.fillStyle = '#16223a'; context.fillRect(0, 0, canvas.width, canvas.height);
context.fillStyle = '#7dd3fc'; context.fillRect(0, 0, canvas.width / 2, canvas.height / 2);
context.fillStyle = 'white'; context.font = '30px sans-serif'; context.fillText('Verified screen fixture', 30, 80);
const image = { width: canvas.width, height: canvas.height, mimeType: 'image/png', base64: canvas.toDataURL('image/png').split(',')[1] };
const state = {
  frames: 0, pending: 0, maxPending: 0, stops: 0, hold: false, failInput: false, failFrame: false,
  inputs: [] as Record<string, unknown>[], releases: [] as (() => void)[], controlAvailable: true,
};
function makeTransport(device: string): ScreenTransport {
  return {
    phone,
    async frame() {
      const sequence = ++state.frames;
      state.pending++; state.maxPending = Math.max(state.maxPending, state.pending);
      try {
        if (state.hold) await new Promise<void>(resolve => state.releases.push(resolve));
        if (state.failFrame) throw new Error('Screen disconnected');
        return { observationId: `${device}:${sequence}`, windowId: 42, packageName: `example.${device}`, title: `Screen ${device}`,
          controlAvailable: state.controlAvailable, image,
          nodes: [{ nodeId: 7, focused: true, editable: true, scrollable: false }, { nodeId: 8, focused: false, editable: false, scrollable: true }],
        } satisfies ScreenFrame;
      } finally { state.pending--; }
    },
    async input(value) {
      state.inputs.push(value);
      return state.failInput ? { code: 'stale_observation', message: 'Screen changed; inspect the new image.' } : { accepted: true };
    },
    async stop() { state.stops++; },
  };
}
function Harness() {
  const [device, setDevice] = useState('first');
  const [mounted, setMounted] = useState(true);
  const transport = useMemo(() => makeTransport(device), [device]);
  Object.assign(window, { __screenFixture: state, __releaseScreen: () => { state.hold = false; state.releases.splice(0).forEach(resolve => resolve()); } });
  if (desktop) return <main className="mx-auto max-w-3xl p-3"><DesktopDeviceScreen deviceId="first" connected/></main>;
  return <main className="mx-auto max-w-3xl p-3"><button onClick={() => setDevice('second')}>Switch device</button><button onClick={() => setMounted(value => !value)}>Toggle viewer</button>
    {mounted && <DeviceScreen transport={transport} onClose={() => setMounted(false)}/>}</main>;
}
if (desktop) {
  const transport = makeTransport('first');
  let callback = 0;
  Object.assign(window, { __TAURI_INTERNALS__: {
    invoke: async (command: string, arguments_: Record<string, any> = {}) => {
      switch (command) {
        case 'remote_screen_status_cmd': return [];
        case 'list_desktop_windows_cmd': return { supported: true, windows: [] };
        case 'remote_phone_screen_open_cmd': return 'phone-view-1';
        case 'remote_phone_screen_stop_cmd': return transport.stop!();
        case 'remote_phone_screen_cmd': return arguments_.arguments.action === 'screenshot' ? transport.frame() : transport.input(arguments_.arguments);
        case 'plugin:event|listen': return ++callback;
        default: return null;
      }
    }, transformCallback: () => ++callback, unregisterCallback: () => {},
  }, __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => {} } });
}
export function renderFixture() { createRoot(document.body.appendChild(document.createElement('div'))).render(<I18nProvider><Harness/></I18nProvider>); }
