import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { fontsourceAssets, omitLegacyFontSources } from './fontsource-assets.mjs';

test('offline math fonts preserve every WOFF2 source and face without redundant formats', () => {
  for (const filename of ['katex.css', 'katex.min.css']) {
    const css = readFileSync(new URL(`../node_modules/katex/dist/${filename}`, import.meta.url), 'utf8');
    const result = fontsourceAssets().transform(css, `/node_modules/katex/dist/${filename}`);
    assert.ok(css.includes('.woff)'));
    assert.ok(css.includes('.ttf)'));
    assert.doesNotMatch(result, /\.(?:woff|ttf)\)/);
    assert.deepEqual(result.match(/url\([^)]*\.woff2\)/g), css.match(/url\([^)]*\.woff2\)/g));
    assert.equal(result.match(/@font-face/g).length, css.match(/@font-face/g).length);
    // Nothing other than src declarations may change: family, weight, style,
    // font-display, glyph metrics and all math layout rules stay identical.
    const withoutSources = value => value.replace(/\bsrc\s*:[^;{}]+(?=;|})/g, '');
    assert.equal(withoutSources(result), withoutSources(css));
  }
});

test('legacy-only and application font sources are not removed', () => {
  const legacy = '@font-face { src: url(font.woff) format("woff"), url(font.ttf) format("truetype"); }';
  assert.equal(omitLegacyFontSources(legacy), legacy);
  assert.equal(fontsourceAssets().transform(legacy, '/src/custom-font.css'), null);
});
