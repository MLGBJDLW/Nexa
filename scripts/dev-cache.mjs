import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';

const root = fileURLToPath(new URL('../', import.meta.url));
const GiB = 1024 ** 3;

export function within(rootPath, candidate) {
  const relative = path.relative(rootPath, candidate);
  return relative !== '' && relative !== '..' && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative);
}

async function measure(entry) {
  const info = await fs.lstat(entry);
  if (info.isSymbolicLink()) throw new Error(`Refusing linked cache path: ${entry}`);
  if (!info.isDirectory()) return { bytes: info.size, modified: info.mtimeMs };
  let bytes = 0;
  let modified = info.mtimeMs;
  for (const child of await fs.readdir(entry)) {
    const result = await measure(path.join(entry, child));
    bytes += result.bytes;
    modified = Math.max(modified, result.modified);
  }
  return { bytes, modified };
}

export async function inventory(workspace) {
  const bases = [
    ['.ruff_cache'], ['apps', 'desktop', 'node_modules', '.vite'],
    ['apps', 'desktop', 'playwright-report'],
  ];
  const desktop = path.join(workspace, 'apps', 'desktop');
  for (const name of await fs.readdir(desktop).catch(() => [])) {
    if (/^test-results(?:[-\w]*)$/.test(name)) bases.push(['apps', 'desktop', name]);
  }
  // Cargo has no build-output quota. Manage whole debug profiles, including
  // deps/build/incremental together, so fingerprint and artifact state agree.
  // Release bundles and the pinned Copilot archive are deliberately excluded.
  for (const base of [['target'], ['apps', 'desktop', 'src-tauri', 'target']]) {
    bases.push([...base, 'debug']);
    for (const name of await fs.readdir(path.join(workspace, ...base)).catch(() => [])) {
      if (/^(?:x86_64|aarch64|i686|armv7|wasm32)-/.test(name)) bases.push([...base, name, 'debug']);
    }
  }
  const entries = [];
  for (const parts of bases) {
    const candidate = path.resolve(workspace, ...parts);
    if (!within(workspace, candidate)) throw new Error('Cache path escaped workspace');
    try {
      // Check every ancestor as well: a nested cache must not traverse a junction.
      for (let i = 1; i <= parts.length; i++) {
        if ((await fs.lstat(path.join(workspace, ...parts.slice(0, i)))).isSymbolicLink()) throw new Error(`Refusing linked cache ancestor: ${candidate}`);
      }
      entries.push({ path: candidate, ...await measure(candidate) });
    } catch (error) {
      if (error.code !== 'ENOENT') throw error;
    }
  }
  return entries.sort((a, b) => a.modified - b.modified);
}

export function selectPrunable(entries, maxBytes, maxAgeDays, now = Date.now()) {
  let remaining = entries.reduce((sum, entry) => sum + entry.bytes, 0);
  return entries.filter(entry => {
    const remove = remaining > maxBytes || now - entry.modified > maxAgeDays * 86400000;
    if (remove) remaining -= entry.bytes;
    return remove;
  });
}

function assertIdle() {
  // Never clean an active compiler/test/server's output, including another
  // checkout sharing this machine. Failure to inspect processes fails closed.
  const listing = process.platform === 'win32'
    ? execFileSync('powershell.exe', ['-NoProfile', '-Command', "Get-CimInstance Win32_Process | Where-Object { $_.Name -match '^(cargo|rustc|rust-lld|link|nexa-desktop|node)(\\.exe)?$' } | Select-Object ProcessId,Name,CommandLine | ConvertTo-Json -Compress"], { encoding: 'utf8', windowsHide: true })
    : execFileSync('ps', ['-A', '-o', 'comm=', '-o', 'args='], { encoding: 'utf8' });
  if (/(?:^|["/\\\s])(?:cargo|rustc|rust-lld|link|nexa-desktop)(?:\.exe)?(?:["\s]|$)/im.test(listing)
    || /\b(?:vite|playwright)(?:[.\/\\\s"]|$)/i.test(listing)) {
    throw new Error('Stop compilers, Nexa development app, Vite, and Playwright before applying cache cleanup.');
  }
}

export async function main(args) {
  const apply = args.includes('--apply');
  const numberOption = (name, fallback) => {
    const arg = args.find(value => value.startsWith(`${name}=`));
    const value = arg ? Number(arg.split('=')[1]) : fallback;
    if (!Number.isFinite(value) || value <= 0) throw new Error(`Invalid ${name}`);
    return value;
  };
  const entries = await inventory(root);
  const selected = selectPrunable(entries, numberOption('--max-gib', 20) * GiB, numberOption('--max-age-days', 14));
  console.log(JSON.stringify({ mode: apply ? 'apply' : 'preview', totalGiB: entries.reduce((sum, item) => sum + item.bytes, 0) / GiB, caches: entries.map(item => ({ ...item, selected: selected.includes(item) })) }, null, 2));
  if (!apply || !selected.length) return;
  assertIdle();
  for (const item of selected) {
    const resolved = await fs.realpath(item.path);
    if (!within(root, resolved) || path.relative(item.path, resolved) !== '') throw new Error(`Unsafe cache target: ${item.path}`);
    await measure(item.path); // Revalidate no links immediately before removal.
    await fs.rm(item.path, { recursive: true, force: false });
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).catch(error => { console.error(error.message); process.exitCode = 1; });
}
