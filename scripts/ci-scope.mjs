import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

import { synchronizePackageVersion } from './sync-workspace-lock-versions.mjs';

export const RELEASE_PLEASE_BRANCH =
  'release-please--branches--master--components--nexa-monorepo';

export const RELEASE_METADATA_PATHS = new Set([
  '.release-please-manifest.json',
  'CHANGELOG.md',
  'Cargo.lock',
  'apps/mobile/src-tauri/Cargo.lock',
  'apps/desktop/package.json',
  'apps/desktop/package-lock.json',
  'apps/desktop/src-tauri/Cargo.toml',
  'apps/desktop/src-tauri/tauri.conf.json',
  'crates/core/Cargo.toml',
  'package-lock.json',
  'package.json',
]);

const VERSION_SENTINEL = '0.0.0-nexa-release-metadata';

const JSON_VERSION_PATHS = new Map([
  ['.release-please-manifest.json', [['.']]],
  ['apps/desktop/package.json', [['version']]],
  ['apps/desktop/package-lock.json', [['version'], ['packages', '', 'version']]],
  ['apps/desktop/src-tauri/tauri.conf.json', [['version']]],
  ['package-lock.json', [['version'], ['packages', '', 'version']]],
  ['package.json', [['version']]],
]);

function normalizeJsonVersionFields(changedPath, contents) {
  const parsed = JSON.parse(contents);
  for (const propertyPath of JSON_VERSION_PATHS.get(changedPath) ?? []) {
    let owner = parsed;
    for (const key of propertyPath.slice(0, -1)) {
      owner = owner?.[key];
    }
    const key = propertyPath.at(-1);
    if (!owner || typeof owner[key] !== 'string') {
      throw new Error(`${changedPath} is missing ${propertyPath.join('.')}`);
    }
    owner[key] = VERSION_SENTINEL;
  }
  return JSON.stringify(parsed);
}

function normalizeCargoManifestVersion(contents) {
  const packageHeader = /^\[package\][ \t]*$(?:\r?\n)?/mu.exec(contents);
  if (!packageHeader) {
    throw new Error('Cargo manifest is missing [package]');
  }
  const sectionStart = packageHeader.index + packageHeader[0].length;
  const remainder = contents.slice(sectionStart);
  const nextSection = /^\[/mu.exec(remainder);
  const sectionEnd = sectionStart + (nextSection?.index ?? remainder.length);
  const packageSection = contents.slice(sectionStart, sectionEnd);
  let replacements = 0;
  const normalizedSection = packageSection.replace(
    /^version[ \t]*=[ \t]*"[^"]+"[ \t]*$/gmu,
    () => {
      replacements += 1;
      return `version = "${VERSION_SENTINEL}"`;
    },
  );
  if (replacements !== 1) {
    throw new Error(`Cargo manifest must contain one [package].version; found ${replacements}`);
  }
  return `${contents.slice(0, sectionStart)}${normalizedSection}${contents.slice(sectionEnd)}`;
}

function normalizeWorkspaceLockVersions(contents) {
  return ['nexa-core', 'nexa-desktop'].reduce(
    (normalized, packageName) =>
      synchronizePackageVersion(normalized, packageName, VERSION_SENTINEL),
    contents,
  );
}

export function releaseMetadataContentsMatch(changedPath, baseContents, headContents) {
  try {
    if (changedPath === 'CHANGELOG.md') {
      return true;
    }
    if (JSON_VERSION_PATHS.has(changedPath)) {
      return (
        normalizeJsonVersionFields(changedPath, baseContents) ===
        normalizeJsonVersionFields(changedPath, headContents)
      );
    }
    if (
      changedPath === 'apps/desktop/src-tauri/Cargo.toml' ||
      changedPath === 'crates/core/Cargo.toml'
    ) {
      return (
        normalizeCargoManifestVersion(baseContents) ===
        normalizeCargoManifestVersion(headContents)
      );
    }
    if (changedPath === 'Cargo.lock') {
      return (
        normalizeWorkspaceLockVersions(baseContents) ===
        normalizeWorkspaceLockVersions(headContents)
      );
    }
    if (changedPath === 'apps/mobile/src-tauri/Cargo.lock') {
      return synchronizePackageVersion(baseContents,'nexa-core',VERSION_SENTINEL) === synchronizePackageVersion(headContents,'nexa-core',VERSION_SENTINEL);
    }
  } catch {
    return false;
  }
  return false;
}

export function isReleaseMetadataOnly({
  branch,
  headRepository,
  repository,
  changedPaths,
  readBaseFile,
  readHeadFile,
}) {
  if (
    branch !== RELEASE_PLEASE_BRANCH ||
    headRepository !== repository ||
    changedPaths.length === 0 ||
    !changedPaths.every((changedPath) => RELEASE_METADATA_PATHS.has(changedPath)) ||
    typeof readBaseFile !== 'function' ||
    typeof readHeadFile !== 'function'
  ) {
    return false;
  }
  return changedPaths.every((changedPath) => {
    try {
      return releaseMetadataContentsMatch(
        changedPath,
        readBaseFile(changedPath),
        readHeadFile(changedPath),
      );
    } catch {
      return false;
    }
  });
}

const isCli =
  process.argv[1] &&
  path.resolve(process.argv[1]) === path.resolve(fileURLToPath(import.meta.url));

const DOCUMENTATION_PATH = /^(?:docs\/|README(?:\.[A-Za-z-]+)?\.md$|(?:CONTRIBUTING|CHANGELOG|SECURITY)\.md$)/u;
const FRONTEND_PATH = /^apps\/desktop\/(?:src\/|public\/|e2e\/|tests\/)/u;
const NATIVE_SOURCE_PATH = /^(?:crates\/(?:core|remote|agent-eval|model-catalog)\/src\/|apps\/desktop\/src-tauri\/src\/).+\.rs$/u;

// Mobile imports selected desktop modules. Follow the actual source graph so
// an unrelated desktop page does not rebuild Android, while shared UI does.
export function mobileSourceDependencies(root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')) {
  const visited = new Set();
  const pending = ['apps/mobile/src/main.tsx'];
  const resolve = (base) => ['', '.ts', '.tsx', '.js', '.jsx', '.json', '.css', '/index.ts', '/index.tsx']
    .map(extension => `${base}${extension}`).find(file => fs.existsSync(file) && fs.statSync(file).isFile());
  while (pending.length) {
    const relative = pending.pop();
    if (visited.has(relative)) continue;
    visited.add(relative);
    const file = path.join(root, relative);
    const source = fs.readFileSync(file, 'utf8');
    // Unknown computed imports/globs must not silently skip mobile checks.
    if (/import\.meta\.glob|\bimport\s*\(\s*[^'"\s]/u.test(source)) return null;
    for (const match of source.matchAll(/(?:\bfrom\s*|\bimport\s*\(\s*|\bimport\s*|@import\s*)['"]([^'"]+)['"]/gu)) {
      const specifier = match[1].split('?')[0];
      const base = specifier.startsWith('@/') ? path.join(root, 'apps/desktop/src', specifier.slice(2))
        : specifier.startsWith('.') ? path.resolve(path.dirname(file), specifier) : null;
      if (!base) continue;
      const resolved = resolve(base);
      if (!resolved) return null;
      pending.push(path.relative(root, resolved).split(path.sep).join('/'));
    }
  }
  return visited;
}

export function mobileCiRequired(changedPaths, dependencies = mobileSourceDependencies()) {
  if (changedPaths.length === 0 || dependencies === null) return true;
  return changedPaths.some(changedPath => dependencies.has(changedPath)
    || /^(?:apps\/mobile\/|shared\/|crates\/|\.cargo\/)/u.test(changedPath)
    || /^(?:Cargo\.(?:toml|lock)|rust-toolchain\.toml|package(?:-lock)?\.json)$/u.test(changedPath)
    || /^apps\/desktop\/(?:package(?:-lock)?\.json|tsconfig[^/]*\.json|public\/)/u.test(changedPath)
    || ['scripts/ci-scope.mjs', 'scripts/ci-scope.test.mjs', 'scripts/release-workflow-contract.test.mjs', '.github/workflows/mobile.yml'].includes(changedPath));
}

export function classifyCiScope(changedPaths, releaseMetadataOnly = false) {
  if (releaseMetadataOnly) return 'lightweight';
  if (changedPaths.length === 0) return 'full';
  // The independently versioned Android workflow owns mobile-only builds.
  if (changedPaths.every(changedPath => DOCUMENTATION_PATH.test(changedPath) || changedPath.startsWith('apps/mobile/'))) return 'lightweight';
  if (changedPaths.every(changedPath => DOCUMENTATION_PATH.test(changedPath))) return 'lightweight';
  if (changedPaths.every(changedPath => DOCUMENTATION_PATH.test(changedPath) || FRONTEND_PATH.test(changedPath))) {
    return 'frontend';
  }
  if (changedPaths.every(changedPath => DOCUMENTATION_PATH.test(changedPath) || NATIVE_SOURCE_PATH.test(changedPath))) {
    return 'native';
  }
  // Manifests, shared contracts, workflows, unknown paths and mixed boundaries
  // retain the complete suite. A release candidate always forces full scope.
  return 'full';
}

export function ciResultFailures(needs) {
  const scope = needs.classify?.outputs?.scope;
  const failures = [];
  if (needs.classify?.result !== 'success' || !['full', 'frontend', 'native', 'lightweight'].includes(scope)) {
    return ['CI scope classification did not succeed'];
  }
  const required = new Set(['repository_policy', 'release_metadata']);
  if (scope === 'full' || scope === 'frontend') {
    required.add('frontend');
    required.add('browser_regressions');
  }
  if (scope === 'full' || scope === 'native') {
    for (const job of ['rust_core', 'agent_task_eval', 'linux_desktop', 'windows-check']) required.add(job);
  }
  if (scope === 'full') required.add('office_smoke');
  for (const job of ['repository_policy', 'release_metadata', 'frontend', 'browser_regressions',
    'rust_core', 'agent_task_eval', 'office_smoke', 'linux_desktop', 'windows-check']) {
    const expected = required.has(job) ? 'success' : 'skipped';
    if (needs[job]?.result !== expected) failures.push(`${job}: ${needs[job]?.result ?? 'missing'} (expected ${expected})`);
  }
  return failures;
}

if (isCli && process.argv.includes('--check-results')) {
  const failures = ciResultFailures(JSON.parse(process.env.NEXA_CI_NEEDS ?? '{}'));
  for (const failure of failures) console.error(failure);
  process.exitCode = failures.length > 0 ? 1 : 0;
} else if (isCli) {
  const changedPaths = fs
    .readFileSync(0, 'utf8')
    .split(/\r?\n/u)
    .filter(Boolean);
  const baseRef = process.env.NEXA_CI_BASE_REF ?? 'origin/master';
  const headRef = process.env.NEXA_CI_HEAD_REF ?? 'HEAD';
  const readGitFile = (ref, changedPath) =>
    execFileSync('git', ['show', `${ref}:${changedPath}`], {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
    });
  const result = isReleaseMetadataOnly({
    branch: process.env.NEXA_CI_BRANCH ?? '',
    headRepository: process.env.NEXA_CI_HEAD_REPOSITORY ?? '',
    repository: process.env.GITHUB_REPOSITORY ?? '',
    changedPaths,
    readBaseFile: (changedPath) => readGitFile(baseRef, changedPath),
    readHeadFile: (changedPath) => readGitFile(headRef, changedPath),
  });
  process.stdout.write(process.argv.includes('--github-output')
    ? `release_metadata_only=${result}\nscope=${classifyCiScope(changedPaths, result)}\nmobile_required=${mobileCiRequired(changedPaths)}\n`
    : result ? 'true' : 'false');
}
