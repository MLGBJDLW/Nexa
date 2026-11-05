import assert from 'node:assert/strict';
import test from 'node:test';
import { changelogEntries, desktopReleaseNotes, releaseHistory, run, updateVersionNotes, withMergedPullRequests } from './release-notes.mjs';

const original = '# Changelog\n\n## [1.2.0](https://example.test/compare/v1.1.0...v1.2.0)\n\nManual release context.\n\n## [1.1.0](https://example.test/1.1.0)\n\nOlder release.\n';
const first = '## What\'s Changed\n* First PR #101\n* Second PR #102';

test('successive PR maintenance retains the complete baseline range and older releases', () => {
  const two = updateVersionNotes(original, '1.2.0', first);
  const three = updateVersionNotes(two, '1.2.0', first + '\n* Third PR #103');
  assert.match(three, /Manual release context/);
  for (const id of [101, 102, 103]) assert.equal(three.split(`#${id}`).length - 1, 1);
  assert.equal(changelogEntries(three)[1].body, changelogEntries(original)[1].body);
  assert.equal(updateVersionNotes(three, '1.2.0', first + '\n* Third PR #103'), three);
});

test('reruns preserve notes outside the managed section without duplication', () => {
  const body = withMergedPullRequests('User-written release notes.', first) + '\nManual postscript.';
  const updated = withMergedPullRequests(body, first);
  assert.match(updated, /User-written release notes/);
  assert.match(updated, /Manual postscript/);
  assert.equal(updated.split('#101').length - 1, 1);
  assert.throws(() => withMergedPullRequests('<!-- nexa:merged-prs:start -->', first));
});

test('updater history contains complete unicode notes and every version, with exact target validation', () => {
  const body = '完整说明。'.repeat(2000) + '\nFinal PR #103';
  const history = releaseHistory(original, '1.2.0', body);
  assert.equal(history[0].body, body);
  assert.equal(history[1].version, '1.1.0');
  assert.equal(history[1].body, 'Older release.');
  assert.throws(() => releaseHistory(original, '9.9.9', body));
  assert.throws(() => changelogEntries(original + '\n## 1.2.0\nDuplicate'));
});

test('current tooling prepares an older immutable draft using only its own candidate metadata', () => {
  const sha = 'a'.repeat(40);
  const writes = new Map();
  const candidateFiles = new Map([
    ['CHANGELOG.md', original],
    ['.release-please-manifest.json', '{".":"1.2.0"}'],
  ]);
  const dependencies = {
    readFile(path) { assert.ok(candidateFiles.has(path)); return candidateFiles.get(path); },
    writeFile(path, value) { writes.set(path, value); },
    command(program, args, input) {
      if (program === 'git') {
        if (args[0] === 'merge-base') {
          assert.deepEqual(args, ['merge-base', '--is-ancestor', 'nexa-monorepo-v1.1.0', sha]);
          return '';
        }
        assert.ok(['HEAD', 'nexa-monorepo-v1.2.0^{commit}'].includes(args[1]));
        return sha;
      }
      if (args[0] === 'api') {
        assert.deepEqual(JSON.parse(input), { tag_name: 'nexa-monorepo-v1.2.0', previous_tag_name: 'nexa-monorepo-v1.1.0', target_commitish: sha });
        return JSON.stringify({ body: first });
      }
      assert.equal(args[0], 'release');
      return JSON.stringify({ isDraft: true, body: 'Original draft notes.\n\n### Mobile 0.3.0 workflow\n\nPhone archive and dictation.\n\n### Desktop interaction\n\nDesktop improvements.' });
    },
  };
  run('publish', { GITHUB_REPOSITORY: 'owner/repo', RELEASE_TAG: 'nexa-monorepo-v1.2.0', TARGET_SHA: sha }, dependencies);
  assert.deepEqual([...writes.keys()], ['release-notes.md', 'release-history.json']);
  assert.match(writes.get('release-notes.md'), /Original draft notes/);
  assert.match(writes.get('release-notes.md'), /#101/);
  assert.match(writes.get('release-notes.md'), /#102/);
  assert.doesNotMatch(writes.get('release-notes.md'), /Mobile 0\.3\.0|Phone archive and dictation/);
  assert.match(writes.get('release-notes.md'), /Desktop improvements/);
  assert.equal(JSON.parse(writes.get('release-history.json'))[1].version, '1.1.0');
});

test('mobile publication includes exactly its current version and rejects a mismatched tag', () => {
  const writes = new Map();
  const dependencies = {
    readFile(path) {
      if (path === 'apps/mobile/package.json') return '{"version":"0.3.0"}';
      if (path === 'apps/mobile/CHANGELOG.md') return '# Nexa Mobile\n\n## 0.3.0\n\nCurrent phone fixes.\n\n## 0.2.0\n\nOld phone notes.\n\n## 0.1.0\n\nInitial phone notes.';
      throw new Error(`Unexpected file: ${path}`);
    },
    writeFile(path, value) { writes.set(path, value); },
    command() { throw new Error('Mobile notes need no GitHub API call'); },
  };
  run('mobile', { RELEASE_TAG: 'nexa-mobile-v0.3.0' }, dependencies);
  assert.equal(writes.get('mobile-release-notes.md'), 'Current phone fixes.\n');
  assert.throws(() => run('mobile', { RELEASE_TAG: 'nexa-mobile-v0.2.0' }, dependencies), /tag|version/i);
});

test('desktop notes retain shared control features while omitting mobile-only sections and PRs', () => {
  const sections = '### Mobile and desktop continuity\nShared phone control.\n### Mobile 0.3.0 workflow\nPhone-only UI.\n### Desktop interaction\nDesktop UI.';
  assert.match(desktopReleaseNotes(sections), /Shared phone control/);
  assert.doesNotMatch(desktopReleaseNotes(sections), /Phone-only UI/);
  const lastSection = withMergedPullRequests('Desktop content.\n\n### Mobile 0.3.0\nPhone-only content.', first);
  assert.doesNotThrow(() => withMergedPullRequests(desktopReleaseNotes(lastSection), first));
  const writes = new Map(); const sha = 'a'.repeat(40);
  run('publish', {GITHUB_REPOSITORY:'owner/repo',RELEASE_TAG:'nexa-monorepo-v1.2.0',TARGET_SHA:sha}, {
    readFile(path) { return path === 'CHANGELOG.md' ? original : '{".":"1.2.0"}'; },
    writeFile(path,value) { writes.set(path,value); },
    command(program,args) {
      if(program==='git') return args[0]==='rev-parse' ? sha : '';
      if(args[0]==='release') return JSON.stringify({isDraft:true,body:sections});
      if(args.includes('--method')) return JSON.stringify({body:'## Changed\n* Phone UI https://github.com/owner/repo/pull/1\n* Shared control https://github.com/owner/repo/pull/10'});
      return args[1].includes('/pulls/1/') ? 'apps/mobile/src/mobile.css' : 'crates/core/src/tools/phone_control_tool.rs\napps/mobile/src/mobile.css';
    },
  });
  const body=writes.get('release-notes.md'); assert.doesNotMatch(body,/Phone UI/); assert.match(body,/pull\/10/);
});
