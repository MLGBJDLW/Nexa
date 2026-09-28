export interface ReleaseNote {
  version: string;
  body: string;
}

interface GitHubRelease {
  tag_name?: string;
  body?: string | null;
  draft?: boolean;
  prerelease?: boolean;
}

const RELEASES_URL = 'https://api.github.com/repos/MLGBJDLW/Nexa/releases?per_page=100';
const VERSION = /^(?:nexa-monorepo-v|v)?(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)(?:\+[0-9A-Za-z.-]+)?$/;

export function releaseVersion(value: string): string {
  return value.trim().match(VERSION)?.[1] ?? '';
}

export function compareReleaseVersions(a: string, b: string): number {
  const split = (version: string): [string, string | undefined] => {
    const index = version.indexOf('-');
    return index < 0 ? [version, undefined] : [version.slice(0, index), version.slice(index + 1)];
  };
  const [aCore, aPre] = split(a);
  const [bCore, bPre] = split(b);
  const left = aCore.split('.').map(Number);
  const right = bCore.split('.').map(Number);
  for (let i = 0; i < 3; i += 1) {
    if (left[i] !== right[i]) return left[i] - right[i];
  }
  if (aPre === undefined) return bPre === undefined ? 0 : 1;
  if (bPre === undefined) return -1;
  const aParts = aPre.split('.');
  const bParts = bPre.split('.');
  for (let i = 0; i < Math.max(aParts.length, bParts.length); i += 1) {
    if (aParts[i] === undefined) return -1;
    if (bParts[i] === undefined) return 1;
    if (aParts[i] === bParts[i]) continue;
    const aNumeric = /^\d+$/.test(aParts[i]);
    const bNumeric = /^\d+$/.test(bParts[i]);
    if (aNumeric && bNumeric) return Number(aParts[i]) - Number(bParts[i]);
    if (aNumeric !== bNumeric) return aNumeric ? -1 : 1;
    return aParts[i] < bParts[i] ? -1 : 1;
  }
  return 0;
}

function releaseBody(body: string, version: string): string {
  const trimmed = body.trim();
  const firstLine = trimmed.split('\n', 1)[0];
  const heading = firstLine.match(/^#{1,2}\s+(?:\[([^\]]+)\](?:\([^)]*\))?|([^\s]+))(?:\s+.*)?$/);
  // Legacy manifests and release bodies may already include their version.
  // Remove only a matching leading version heading, never authored sections.
  return heading && releaseVersion(heading[1] ?? heading[2]) === version
    ? trimmed.slice(firstLine.length).trim()
    : trimmed;
}

export function formatReleaseNotesBetween(entries: ReleaseNote[], current: string, target: string): string | undefined {
  const selected = new Map<string, ReleaseNote>();
  for (const entry of entries) {
    const version = releaseVersion(entry.version);
    if (!version || (!target.includes('-') && version.includes('-'))) continue;
    if (compareReleaseVersions(version, current) > 0 && compareReleaseVersions(version, target) <= 0) {
      selected.set(version, { version, body: entry.body });
    }
  }
  // Partial API pages must never replace the target release's own notes.
  if (!selected.has(target)) return undefined;
  return [...selected.values()]
    .sort((a, b) => compareReleaseVersions(b.version, a.version))
    .map(({ version, body }) => `## v${version}\n\n${releaseBody(body, version) || '_No release notes provided._'}`)
    .join('\n\n---\n\n');
}

function bundledNotes(rawJson: Record<string, unknown>): ReleaseNote[] | undefined {
  if (!Array.isArray(rawJson.releaseNotes) || rawJson.releaseNotes.length === 0) return undefined;
  const entries: ReleaseNote[] = [];
  const seen = new Set<string>();
  for (const value of rawJson.releaseNotes) {
    if (!value || typeof value !== 'object' || typeof value.version !== 'string' || typeof value.body !== 'string') return undefined;
    const version = releaseVersion(value.version);
    if (!version || seen.has(version)) return undefined;
    seen.add(version);
    entries.push({ version, body: value.body });
  }
  return entries;
}

/** All update sources share the same version range; bundled history works offline. */
export async function resolveReleaseNotes(
  update: { currentVersion: string; version: string; body?: string; rawJson: Record<string, unknown> },
  fetcher: typeof fetch = fetch,
): Promise<string | undefined> {
  const current = releaseVersion(update.currentVersion);
  const target = releaseVersion(update.version);
  if (!current || !target || compareReleaseVersions(current, target) >= 0) return update.body;
  const bundled = bundledNotes(update.rawJson);
  if (bundled) {
    const notes = formatReleaseNotesBetween(bundled, current, target);
    if (notes) return notes;
  }
  try {
    const entries: ReleaseNote[] = [];
    const signal = AbortSignal.timeout(8000);
    // Follow pagination even when a page includes an older version: releases
    // are not guaranteed to be returned in semantic-version order.
    for (let page = 1; ; page += 1) {
      const response = await fetcher(`${RELEASES_URL}&page=${page}`, {
        headers: { Accept: 'application/vnd.github+json' }, signal,
      });
      if (!response.ok) throw new Error(`Release history request failed (${response.status})`);
      const releases: unknown = await response.json();
      if (!Array.isArray(releases)) throw new Error('Invalid release history');
      for (const value of releases as GitHubRelease[]) {
        if (!value || value.draft || (value.prerelease && !target.includes('-'))) continue;
        const version = releaseVersion(value.tag_name ?? '');
        if (version) entries.push({ version, body: value.body ?? '' });
      }
      if (!response.headers.get('link')?.includes('rel="next"')) break;
    }
    return formatReleaseNotesBetween(entries, current, target) ?? update.body;
  } catch {
    // Checking or installing an update must not depend on release-note access.
    return update.body;
  }
}
