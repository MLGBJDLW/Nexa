import { expect, test } from '@playwright/test';
import { readFileSync, existsSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { resolveProviderIconMeta } from '../src/lib/providerIcons';

const presets = ['external-agent-presets', 'subscription-runtime-presets'].flatMap(name => JSON.parse(readFileSync(new URL(`../../../shared/${name}.json`, import.meta.url), 'utf8')));

test('every external agent resolves a bundled SVG identity independently of its model', () => {
  for (const preset of presets) {
    const icon = resolveProviderIconMeta({ provider: preset.provider, providerId: preset.id, model: 'gpt-6', label: preset.name });
    expect(icon.label, preset.provider).toBe(preset.name);
    expect(icon.asset, preset.provider).toMatch(/^\/provider-icons\/.+\.svg$/);
    expect(existsSync(new URL(`../public${icon.asset}`, import.meta.url)), preset.provider).toBe(true);
  }
});

test('external agent SVG assets match their provenance manifest and contain no external content', () => {
  const sources = JSON.parse(readFileSync(new URL('../public/provider-icons/agents/sources.json', import.meta.url), 'utf8'));
  for (const entry of sources) {
    const svg = readFileSync(new URL(`../public${entry.asset}`, import.meta.url), 'utf8');
    expect(createHash('sha256').update(svg).digest('hex')).toBe(entry.sha256);
    expect(svg).toMatch(/<svg\b/);
    expect(svg).not.toMatch(/<(?:script|foreignObject|image)\b|\bon\w+\s*=|(?:href|src)\s*=\s*["'](?!#)|<!ENTITY|<!DOCTYPE/i);
  }
});
