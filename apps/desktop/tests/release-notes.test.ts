import { compareReleaseVersions, resolveReleaseNotes } from '../src/lib/releaseNotes';

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

async function main() {
  assert(compareReleaseVersions('1.0.0-rc.10', '1.0.0-rc.2') > 0, 'numeric prerelease ordering');
  assert(compareReleaseVersions('1.0.0-alpha-z', '1.0.0-alpha-a') > 0, 'hyphens inside prerelease identifiers');
  assert(compareReleaseVersions('1.0.0', '1.0.0-rc.10') > 0, 'stable follows prerelease');
  const update = { currentVersion: '0.1.0', version: '0.1.80', body: 'latest fallback', rawJson: {} };
  const releaseNotes = Array.from({ length: 81 }, (_, i) => ({ version: `0.1.${i}`, body: `release ${i}` }));
  const offline = await resolveReleaseNotes({ ...update, rawJson: { releaseNotes } }, async () => {
    throw new Error('bundled history must not access the network');
  });
  assert(offline !== undefined, 'bundled notes are available');
  assert(offline.match(/^## v0\.1\./gm)?.length === 80 && offline.includes('release 80'), 'all 80 skipped releases are included');
  assert(!offline.includes('release 0\n'), 'installed release is excluded');

  let pages = 0;
  const ranged = await resolveReleaseNotes(update, async (input) => {
    pages += 1;
    assert(String(input).endsWith(`page=${pages}`), 'each subsequent page is requested');
    return new Response(JSON.stringify(pages === 1 ? [
      { tag_name: 'v0.1.80', body: 'target' },
      { tag_name: 'v0.1.0', body: 'installed' },
      { tag_name: 'v0.1.79', body: 'draft', draft: true },
      { tag_name: 'v0.1.78-rc.1', body: 'preview', prerelease: true },
    ] : [{ tag_name: 'v0.1.1', body: 'older intermediate' }]), {
      headers: pages === 1 ? { link: '<https://api.github.com/unused>; rel="next"' } : {},
    });
  });
  assert(ranged !== undefined, 'paged notes are available');
  assert(pages === 2 && ranged?.includes('older intermediate'), 'unsorted pages cannot prematurely stop range discovery');
  assert(!ranged.includes('draft') && !ranged.includes('preview') && !ranged.includes('installed'), 'stable release range excludes drafts and previews');
  const fallback = await resolveReleaseNotes(update, async () => new Response('[]', { status: 503 }));
  assert(fallback === update.body, 'failed history lookup keeps the target notes');
  const missingTarget = await resolveReleaseNotes(update, async () => new Response(JSON.stringify([{ tag_name: 'v0.1.1', body: 'incomplete list' }])));
  assert(missingTarget === update.body, 'partial history never replaces the target notes');
  console.log('release-note range contracts passed');
}

void main().catch(error => { throw error; });
