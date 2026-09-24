import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { test } from 'node:test';

// Execute the production bootstrap with both Windows PowerShell 5.1 and
// PowerShell 7. Source-string assertions cannot detect unsupported escapes.
const source = readFileSync(new URL('../src-tauri/src/commands/terminal.rs', import.meta.url), 'utf8');
const literal = source.match(/if shell\.contains\("PowerShell"\)\s*\{\s*return Some\(\s*("(?:\\.|[^"\\])*")/);
assert.ok(literal, 'production PowerShell integration bootstrap must be found');
const bootstrap = JSON.parse(literal[1]);

for (const shell of ['powershell.exe', 'pwsh.exe']) {
  test(`${shell}: prompt emits OSC bytes without visible control text`, { skip: process.platform !== 'win32' }, (context) => {
    const script = `${bootstrap}\n$null = prompt`;
    const result = spawnSync(shell, ['-NoLogo', '-NoProfile', '-NonInteractive', '-EncodedCommand', Buffer.from(script, 'utf16le').toString('base64')], { encoding: 'utf8', windowsHide: true, timeout: 15_000 });
    if (shell === 'pwsh.exe' && result.error?.code === 'ENOENT') return context.skip('PowerShell 7 is not installed');
    assert.equal(result.error, undefined);
    assert.equal(result.status, 0, result.stderr);
    assert.ok(result.stdout.includes('\x1b]633;D;0\x07'), JSON.stringify(result.stdout));
    assert.ok(result.stdout.includes('\x1b]633;P;Cwd='), JSON.stringify(result.stdout));
    assert.ok(result.stdout.includes('\x1b]633;A\x07'), JSON.stringify(result.stdout));
    assert.ok(!result.stdout.replaceAll('\x1b]633', '').includes('e]633'), 'unsupported escape must never be rendered as prompt text');
  });
}
