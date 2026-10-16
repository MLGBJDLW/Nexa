#!/usr/bin/env node
import crypto from 'node:crypto';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
export const pins = JSON.parse(await fs.readFile(path.join(repository, 'shared/shell-tools.json'), 'utf8'));
export const sha256 = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
export function platformsFor(host) {
  if (!Object.hasOwn(pins.tools.rg.assets, host)) throw new Error(`Unsupported shell-tool platform: ${host}`);
  return host === 'windows-x64' ? [host, 'linux-x64', 'linux-arm64'] : [host];
}
export function verifyAsset(bytes, pin) {
  if (sha256(bytes) !== pin.sha256) throw new Error(`SHA-256 mismatch for ${pin.file}`);
  if (!bytes.length || bytes.length > 32 * 1024 * 1024) throw new Error(`Invalid asset size: ${pin.file}`);
}

async function download(tool, pin) {
  const url = `https://github.com/${tool.repository}/releases/download/${tool.tag}/${pin.file}`;
  const response = await fetch(url, { signal: AbortSignal.timeout(120_000) });
  if (!response.ok) throw new Error(`${pin.file}: HTTP ${response.status}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  verifyAsset(bytes, pin);
  return bytes;
}

export async function bundle(output, host) {
  const platforms = platformsFor(host);
  const temp = await fs.mkdtemp(path.join(os.tmpdir(), 'nexa-shell-tools-'));
  const files = [];
  try {
    await fs.mkdir(output, { recursive: true });
    for (const platform of platforms) {
      const destination = path.join(output, platform);
      await fs.mkdir(destination, { recursive: true });
      for (const [name, tool] of Object.entries(pins.tools)) {
        const pin = tool.assets[platform];
        const bytes = await download(tool, pin);
        let binary = bytes;
        if (pin.member) {
          const archive = path.join(temp, pin.file);
          await fs.writeFile(archive, bytes);
          // Extract only the pinned member to stdout. No archive path is ever
          // allowed to write into the checkout or follow an archive symlink.
          binary = execFileSync('tar', ['-xOf', archive, pin.member], { maxBuffer: 32 * 1024 * 1024, windowsHide: true });
        }
        const filename = name + (platform.startsWith('windows-') ? '.exe' : '');
        await fs.writeFile(path.join(destination, filename), binary, { mode: 0o755 });
        await fs.chmod(path.join(destination, filename), 0o755);
        const license = path.join(repository, 'apps/desktop/licenses', `${name}.txt`);
        await fs.copyFile(license, path.join(destination, `${name}-LICENSE.txt`));
        files.push({ path: `${platform}/${filename}`, size: binary.length, sha256: sha256(binary), version: tool.version });
      }
    }
    const manifest = { kind: 'nexaShellTools', manifestVersion: 1, platforms, files };
    await fs.writeFile(path.join(output, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`);
    return manifest;
  } finally {
    // This is the exact directory returned by mkdtemp, never a caller path.
    await fs.rm(temp, { recursive: true, force: true });
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  const host = args[0] ?? `${process.platform === 'win32' ? 'windows' : process.platform}-${process.arch}`;
  const output = path.join(repository, 'apps/desktop/src-tauri/resources/shell-tools');
  const manifest = await bundle(output, host);
  process.stdout.write(`${JSON.stringify({ output, platforms: manifest.platforms, binaries: manifest.files.length })}\n`);
}
