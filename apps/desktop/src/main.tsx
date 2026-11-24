import { runLocalStorageMigrations } from "./lib/localStorageMigrations";
import "@fontsource-variable/inter";
import "katex/dist/katex.min.css";
import "./index.css";
import { invoke, isTauri } from '@tauri-apps/api/core';
import { relaunch } from '@tauri-apps/plugin-process';

let nativeStartupFailed = false;
async function start() {
  runLocalStorageMigrations();
  if (isTauri()) {
    try {
      await invoke('wait_for_desktop_startup');
    } catch (error) {
      nativeStartupFailed = true;
      throw error;
    }
  }
  const { mountApp } = await import('./bootstrap');
  mountApp();
}

void start().catch((error: unknown) => {
  console.error('Unable to load the Nexa interface', error);
  const root = document.getElementById('root');
  if (!root) return;
  const message = document.createElement('p');
  message.setAttribute('role', 'alert');
  message.textContent = 'Unable to load Nexa.';
  const retry = document.createElement('button');
  retry.type = 'button';
  retry.textContent = nativeStartupFailed ? 'Restart Nexa' : 'Reload';
  retry.onclick = () => nativeStartupFailed ? void relaunch() : window.location.reload();
  root.replaceChildren(message, retry);
});
