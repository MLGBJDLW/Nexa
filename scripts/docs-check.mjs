import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

export const CATALOG_START = '<!-- BEGIN GENERATED TOOL SCHEMAS -->';
export const CATALOG_END = '<!-- END GENERATED TOOL SCHEMAS -->';

const normalizeReference = (value) => value.trim().replace(/\s+/gu, ' ').toLowerCase();

function withoutFences(markdown) {
  let fence;
  return markdown.split('\n').map((line) => {
    const marker = /^ {0,3}(`{3,}|~{3,})/u.exec(line)?.[1];
    if (fence) {
      if (marker?.[0] === fence[0] && marker.length >= fence.length
        && line.trim() === marker) fence = undefined;
      return '';
    }
    if (marker) {
      fence = marker;
      return '';
    }
    return line;
  });
}

export function parseMarkdown(markdown) {
  const lines = withoutFences(markdown.replace(/\r\n/gu, '\n'));
  const headings = new Set();
  const references = new Map();
  const links = [];
  for (const [index, line] of lines.entries()) {
    const heading = /^ {0,3}#{1,6}\s+(.+?)(?:\s+#+)?\s*$/u.exec(line)?.[1]
      ?? (index > 0 && /^ {0,3}(?:=+|-+)\s*$/u.test(line) ? lines[index - 1].trim() : null);
    if (heading) {
      const base = heading.replace(/<[^>]*>/gu, '')
        .replace(/!?\[([^\]]*)\]\([^)]*\)/gu, '$1')
        .toLowerCase().replace(/[^\p{L}\p{N}\p{M}\p{Pc}\- ]/gu, '')
        .replace(/ /gu, '-');
      let slug = base;
      for (let suffix = 1; headings.has(slug); suffix += 1) slug = `${base}-${suffix}`;
      headings.add(slug);
    }
    for (const match of line.matchAll(/\b(?:id|name)=["']([^"']+)["']/gu)) {
      if (/<(?:a|h[1-6])\b/iu.test(line)) headings.add(match[1]);
    }
    const definition = /^ {0,3}\[([^\]]+)\]:\s*(?:<([^>]+)>|(\S+))/u.exec(line);
    if (definition) references.set(normalizeReference(definition[1]), definition[2] ?? definition[3]);
  }
  for (const [index, line] of lines.entries()) {
    if (/^ {0,3}\[[^\]]+\]:/u.test(line)) continue;
    const text = line.replace(/(`+)[\s\S]*?\1/gu, (match) => ' '.repeat(match.length));
    // Maintained docs use inline, reference, or shortcut links. Parenthesized
    // destinations support one nested pair; use <...> for more complex paths.
    const pattern = /!?\[([^\[\]]*)\](?:\(\s*(?:<([^>]+)>|((?:[^\s()]|\([^()]*\))+))(?:\s+["'][^\n]*?["'])?\s*\)|\[([^\]]*)\])?/gu;
    for (const match of text.matchAll(pattern)) {
      const explicitReference = match[4] !== undefined;
      const reference = normalizeReference(match[4] || match[1]);
      const target = match[2] ?? match[3] ?? references.get(reference);
      if (target) links.push({ target, line: index + 1 });
      else if (explicitReference) links.push({ error: `undefined reference [${reference}]`, line: index + 1 });
    }
  }
  return { headings, links };
}

export function validateDocumentation(documents, repositoryPaths) {
  const parsed = new Map([...documents].map(([file, content]) => [file, parseMarkdown(content)]));
  const errors = [];
  const indexed = new Set();
  const exists = (target) => target === '.' || repositoryPaths.has(target)
    || [...repositoryPaths].some((file) => file.startsWith(`${target}/`));
  for (const [file, document] of parsed) {
    for (const link of document.links) {
      const fail = (message) => errors.push(`${file}:${link.line}: ${message}`);
      if (link.error) {
        fail(link.error);
        continue;
      }
      if (/^(?:[a-z][a-z\d+.-]*:|\/\/)/iu.test(link.target)) continue;
      let pathname;
      let anchor;
      try {
        const hash = link.target.indexOf('#');
        pathname = decodeURIComponent(hash < 0 ? link.target : link.target.slice(0, hash));
        anchor = hash < 0 ? '' : decodeURIComponent(link.target.slice(hash + 1));
      } catch {
        fail(`invalid URL encoding: ${link.target}`);
        continue;
      }
      if (pathname.startsWith('/')) {
        fail(`use a repository-relative link: ${link.target}`);
        continue;
      }
      const target = pathname
        ? path.posix.normalize(path.posix.join(path.posix.dirname(file), pathname)).replace(/\/$/u, '')
        : file;
      if (target === '..' || target.startsWith('../') || !exists(target)) {
        fail(`missing repository target: ${link.target}`);
        continue;
      }
      if (file === 'docs/README.md') indexed.add(target);
      if (anchor && parsed.has(target) && !parsed.get(target).headings.has(anchor)) {
        fail(`missing heading: ${link.target}`);
      }
    }
  }
  for (const file of documents.keys()) {
    if (/^docs\/[^/]+\.md$/u.test(file) && file !== 'docs/README.md' && !indexed.has(file)) {
      errors.push(`${file}: not linked from docs/README.md`);
    }
  }
  return errors;
}

export function renderToolCatalog(schemas) {
  const rows = [...schemas].sort(([a], [b]) => a.localeCompare(b, 'en')).map(([file, schema]) => {
    const name = schema.name ?? path.posix.basename(file, '.json');
    if (!/^[a-z][a-z\d_]*$/u.test(name) || typeof schema.description !== 'string' || !schema.parameters) {
      throw new Error(`Invalid tool definition: ${file}`);
    }
    const sentence = schema.description.split(/\n|\.\s/u)[0].replace(/\|/gu, '\\|');
    const description = sentence.length > 180 ? `${sentence.slice(0, 177)}...` : sentence;
    return `| [\`${name}\`](../${file}) | ${description} |`;
  });
  return `${CATALOG_START}\n\n| Core schema | Purpose (abridged; follow the schema for full rules) |\n| --- | --- |\n${rows.join('\n')}\n\n${CATALOG_END}`;
}

export function replaceToolCatalog(markdown, catalog) {
  const start = markdown.indexOf(CATALOG_START);
  const end = markdown.indexOf(CATALOG_END);
  if (start < 0 || end < start || markdown.indexOf(CATALOG_START, start + 1) >= 0
    || markdown.indexOf(CATALOG_END, end + 1) >= 0) {
    throw new Error('docs/TOOLS.md must contain one ordered generated-schema marker pair');
  }
  return markdown.slice(0, start) + catalog + markdown.slice(end + CATALOG_END.length);
}

function maintainedDocument(file) {
  return /^(?:README(?:\.zh-CN)?|CONTRIBUTING|CONTEXT)\.md$/u.test(file)
    || /^docs\/[^/]+\.md$/u.test(file)
    || file === 'integrations/office-addin/README.md'
    || /^apps\/desktop\/(?:public\/provider-icons\/README|src-tauri\/resources\/[^/]+\/README)\.md$/u.test(file);
}

function main() {
  const root = fileURLToPath(new URL('../', import.meta.url));
  const mode = process.argv[2] ?? 'check';
  if (!['check', 'generate'].includes(mode)) throw new Error('Usage: node scripts/docs-check.mjs [check|generate]');
  const files = execFileSync('git', ['ls-files', '-z', '--cached', '--others', '--exclude-standard'], {
    cwd: root, encoding: 'utf8',
  }).split('\0').filter(Boolean);
  const paths = new Set(files);
  const read = (file) => fs.readFileSync(path.join(root, file), 'utf8').replace(/\r\n/gu, '\n');
  const schemas = new Map(files.filter((file) => /^crates\/core\/prompts\/tools\/[^/]+\.json$/u.test(file))
    .map((file) => [file, JSON.parse(read(file))]));
  const tools = read('docs/TOOLS.md');
  const generated = replaceToolCatalog(tools, renderToolCatalog(schemas));
  if (mode === 'generate') {
    fs.writeFileSync(path.join(root, 'docs/TOOLS.md'), generated);
    console.log(`Updated tool schema index (${schemas.size} definitions).`);
    return;
  }
  const documents = new Map(files.filter(maintainedDocument).map((file) => [file, read(file)]));
  const errors = validateDocumentation(documents, paths);
  if (tools !== generated) errors.push('docs/TOOLS.md: stale schema index; run npm run docs:generate');
  if (errors.length) {
    console.error(errors.join('\n'));
    process.exitCode = 1;
  } else {
    console.log(`Documentation checks passed: ${documents.size} maintained documents, ${schemas.size} tool schemas.`);
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
