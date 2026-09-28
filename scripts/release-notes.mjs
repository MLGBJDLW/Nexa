import { readFileSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const BEGIN = '<!-- nexa:merged-prs:start -->';
const END = '<!-- nexa:merged-prs:end -->';

export function changelogEntries(markdown) {
  const headings = [...markdown.matchAll(/^## \[?(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)(?:\]|\s|$).*$/gm)];
  const seen = new Set();
  return headings.map((heading, index) => {
    const version = heading[1];
    if (seen.has(version)) throw new Error(`Duplicate changelog version ${version}`);
    seen.add(version);
    const start = heading.index;
    const end = headings[index + 1]?.index ?? markdown.length;
    return { version, start, end, body: markdown.slice(start, end).trim() };
  });
}

export function withMergedPullRequests(body, generated) {
  const start = body.indexOf(BEGIN);
  const end = body.indexOf(END);
  if ((start < 0) !== (end < 0) || (start >= 0 && end < start)) {
    throw new Error('Unbalanced managed release-note markers');
  }
  const preserved = start < 0 ? body : body.slice(0, start) + body.slice(end + END.length);
  // Rebuild from the entire tag-to-SHA range on every run. Never append only
  // the triggering PR, and never replace author-written content outside it.
  const notes = generated.trim().replace(/^## /gm, '### ');
  if (!notes) throw new Error('GitHub returned empty cumulative release notes');
  return `${preserved.trim()}\n\n${BEGIN}\n${notes}\n${END}\n`;
}

export function updateVersionNotes(changelog, version, generated) {
  const entry = changelogEntries(changelog).find(entry => entry.version === version);
  if (!entry) throw new Error(`Missing changelog entry for ${version}`);
  return changelog.slice(0, entry.start) + withMergedPullRequests(entry.body, generated)
    + '\n' + changelog.slice(entry.end);
}

export function releaseHistory(changelog, version, currentBody) {
  const entries = changelogEntries(changelog);
  if (entries[0]?.version !== version) throw new Error(`Changelog head does not match release ${version}`);
  return entries.map((entry, index) => ({ version: entry.version, body: index === 0 ? currentBody : entry.body }));
}

function command(program, args, input) {
  return execFileSync(program, args, { encoding: 'utf8', input, maxBuffer: 16 * 1024 * 1024, stdio: ['pipe', 'pipe', 'pipe'] }).trim();
}

export function run(mode, env = process.env, dependencies = {}) {
  const execute = dependencies.command ?? command;
  const read = dependencies.readFile ?? readFileSync;
  const write = dependencies.writeFile ?? writeFileSync;
  const repository = env.GITHUB_REPOSITORY;
  if (!repository || !/^[\w.-]+\/[\w.-]+$/.test(repository)) throw new Error('GITHUB_REPOSITORY is required');
  const changelog = read('CHANGELOG.md', 'utf8');
  const version = JSON.parse(read('.release-please-manifest.json', 'utf8'))['.'];
  const entries = changelogEntries(changelog);
  if (entries[0]?.version !== version || !entries[1]) throw new Error('Release changelog range is unresolved');
  const tag = `nexa-monorepo-v${version}`;
  const previousTag = `nexa-monorepo-v${entries[1].version}`;
  const targetSha = mode === 'maintain'
    ? execute('git', ['merge-base', 'HEAD', 'origin/master'])
    : env.TARGET_SHA;
  if (!/^[a-f0-9]{40}$/.test(targetSha ?? '')) throw new Error('Immutable target SHA is required');
  execute('git', ['merge-base', '--is-ancestor', previousTag, targetSha]);
  if (mode === 'publish') {
    if (env.RELEASE_TAG !== tag || execute('git', ['rev-parse', 'HEAD']) !== targetSha
        || execute('git', ['rev-parse', `${tag}^{commit}`]) !== targetSha) {
      throw new Error('Notes, version and build must resolve to the same immutable release');
    }
  } else if (mode !== 'maintain') {
    throw new Error(`Unknown release-note mode ${mode}`);
  }
  // This endpoint generates text without creating/publishing a release:
  // https://docs.github.com/en/rest/releases/releases#generate-release-notes-content-for-a-release
  const generated = JSON.parse(execute('gh', ['api', '--method', 'POST',
    `repos/${repository}/releases/generate-notes`, '--input', '-'], JSON.stringify({
    tag_name: tag, previous_tag_name: previousTag, target_commitish: targetSha,
  }))).body;
  if (typeof generated !== 'string') throw new Error('Missing generated release-note body');
  if (mode === 'maintain') {
    if (!/^\d+$/.test(env.RELEASE_PR_NUMBER ?? '')) throw new Error('Release PR number is required');
    const pr = JSON.parse(execute('gh', ['pr', 'view', env.RELEASE_PR_NUMBER, '--repo', repository, '--json', 'body']));
    write('CHANGELOG.md', updateVersionNotes(changelog, version, generated));
    write(env.RELEASE_PR_BODY_FILE, withMergedPullRequests(pr.body, generated));
  } else {
    const release = JSON.parse(execute('gh', ['release', 'view', tag, '--repo', repository, '--json', 'body,isDraft']));
    if (!release.isDraft) throw new Error('Release notes can only be prepared for a draft');
    const body = withMergedPullRequests(release.body, generated);
    write('release-notes.md', body);
    write('release-history.json', JSON.stringify(releaseHistory(changelog, version, body), null, 2) + '\n');
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { run(process.argv[2]); } catch (error) {
    // CLI failures may include HTTP metadata; keep credentials and subprocess
    // output out of the release log.
    console.error(`Release-note preparation failed: ${error instanceof Error ? error.message.split('\n')[0] : 'unknown error'}`);
    process.exitCode = 1;
  }
}
