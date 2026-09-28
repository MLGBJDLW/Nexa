import assert from 'node:assert/strict';
import test from 'node:test';
import { changelogEntries, releaseHistory, updateVersionNotes, withMergedPullRequests } from './release-notes.mjs';

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
  assert.throws(() => releaseHistory(original, '9.9.9', body));
  assert.throws(() => changelogEntries(original + '\n## 1.2.0\nDuplicate'));
});
