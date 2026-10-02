import { execFileSync } from 'node:child_process';

const cargo = process.env.NEXA_CARGO || 'cargo';
const toolchain = process.env.NEXA_RUST_TOOLCHAIN;
const run = args => execFileSync(cargo, [...(toolchain ? [`+${toolchain}`] : []), ...args], { encoding: 'utf8' });
const minimal = run(['tree', '-p', 'nexa-core', '--no-default-features', '--edges', 'normal', '--prefix', 'none']);
for (const dependency of ['headless_chrome', 'windows-capture', 'lopdf', 'calamine', 'imageproc']) {
  if (minimal.split(/\r?\n/).some(line => line.startsWith(`${dependency} v`))) throw new Error(`Minimal runtime still includes host dependency ${dependency}`);
}
const catalog = run(['tree', '-p', 'nexa-model-catalog', '--edges', 'normal', '--prefix', 'none']);
for (const dependency of ['nexa-core', 'rusqlite', 'reqwest', 'tokio', 'headless_chrome', 'tauri']) {
  if (catalog.split(/\r?\n/).some(line => line.startsWith(`${dependency} v`))) throw new Error(`Catalog protocol depends on runtime/host ${dependency}`);
}
console.log('Minimal runtime excludes browser, capture, PDF, spreadsheet and OCR processing dependencies; catalog protocol is host-independent.');
