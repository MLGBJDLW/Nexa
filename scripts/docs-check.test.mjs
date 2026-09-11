import test from 'node:test';
import assert from 'node:assert/strict';
import { CATALOG_START, CATALOG_END, parseMarkdown, validateDocumentation, renderToolCatalog, replaceToolCatalog } from './docs-check.mjs';

test('checks inline, reference, encoded-path, and duplicate-heading navigation', () => {
  const documents = new Map([
    ['README.md', '[guide](docs/My%20Guide.md#use-it-1)\n[code][impl]\n\n[impl]: src/main.rs\n'],
    ['docs/README.md', '[guide](My%20Guide.md)\n'],
    ['docs/My Guide.md', '# Guide\n## Use it\n## Use it\n[home](../README.md)\n'],
  ]);
  assert.deepEqual(validateDocumentation(documents, new Set([...documents.keys(), 'src/main.rs'])), []);
});

test('reports missing targets, headings, undefined references, and orphan documents', () => {
  const documents = new Map([
    ['README.md', '[bad](missing.md)\n[heading](docs/guide.md#absent)\n[x][missing]\n'],
    ['docs/README.md', '# Index\n'],
    ['docs/guide.md', '# Guide\n'],
  ]);
  const errors = validateDocumentation(documents, new Set(documents.keys()));
  assert.equal(errors.length, 4);
  assert.ok(errors.some((error) => error.includes('missing repository target: missing.md')));
  assert.ok(errors.some((error) => error.includes('missing heading: docs/guide.md#absent')));
  assert.ok(errors.some((error) => error.includes('undefined reference [missing]')));
  assert.ok(errors.some((error) => error.includes('not linked from docs/README.md')));
});

test('ignores links and headings in fenced/inline code and skips external URLs', () => {
  const markdown = '# Real\n```md\n# Fake\n[x](missing.md)\n```\n~~~\n[x][missing]\n~~~\n`[x](missing.md)`\n[web](https://example.com)\n[local](#real)\n';
  const parsed = parseMarkdown(markdown);
  assert.deepEqual([...parsed.headings], ['real']);
  assert.equal(parsed.links.length, 2);
  assert.deepEqual(validateDocumentation(new Map([['README.md', markdown]]), new Set(['README.md'])), []);
});

test('accepts directory links only for repository content and rejects escaped/root paths', () => {
  const docs = new Map([['README.md', '[dir](src/)\n[local research](docs/research/private.md)\n[escape](../private.md)\n[root](/src/main.rs)\n']]);
  const errors = validateDocumentation(docs, new Set(['README.md', 'src/main.rs']));
  assert.equal(errors.length, 3);
  assert.ok(errors.every((error) => !error.includes('target: src/')));
});

test('generates deterministic schema links and preserves surrounding authored prose', () => {
  const schemas = new Map([
    ['crates/core/prompts/tools/z.json', { name: 'z', description: 'Last tool. More detail.', parameters: {} }],
    ['crates/core/prompts/tools/a.json', { description: 'First | tool', parameters: {} }],
  ]);
  const catalog = renderToolCatalog(schemas);
  assert.ok(catalog.indexOf('[`a`]') < catalog.indexOf('[`z`]'));
  assert.ok(catalog.includes('First \\| tool'));
  const input = `# Tools\n${CATALOG_START}\nold\n${CATALOG_END}\n## Usage\n`;
  const result = replaceToolCatalog(input, catalog);
  assert.equal(result, `# Tools\n${catalog}\n## Usage\n`);
  assert.equal(replaceToolCatalog(result, catalog), result);
  assert.notEqual(renderToolCatalog(new Map([...schemas, ['crates/core/prompts/tools/new.json', { description: 'New tool', parameters: {} }]])), catalog);
});

test('fails rather than rewriting ambiguous or missing generated markers', () => {
  assert.throws(() => replaceToolCatalog('# Tools', 'new'), /marker pair/u);
  assert.throws(() => replaceToolCatalog(`${CATALOG_END}\n${CATALOG_START}`, 'new'), /marker pair/u);
  assert.throws(() => replaceToolCatalog(`${CATALOG_START}\n${CATALOG_START}\n${CATALOG_END}`, 'new'), /marker pair/u);
});
