import type { CodexAccountSnapshot, CopilotAccountSnapshot } from './api';

export interface AccountQuotaWindow {
  id: string;
  label: string;
  kind?: 'primary' | 'secondary';
  durationMinutes?: number | null;
  usedPercent: number | null;
  resetsAt: string | null;
  unlimited?: boolean;
}

export interface AccountQuota {
  status: 'ready' | 'unsupported' | 'unavailable' | 'signedOut';
  provider?: string | null;
  plan?: string | null;
  windows: AccountQuotaWindow[];
  details?: string[];
}

export interface ExternalAccountUsage {
  status: 'ready' | 'unsupported' | 'unavailable';
  provider: string | null;
  plan: string | null;
  fetchedAt: string | null;
  windows: { label: string; usedPercent: number | null; resetsAt: string | null }[];
  details: string[];
}

export function quotaPercent(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? Math.max(0, Math.min(100, value)) : null;
}

export function codexAccountQuota(snapshot: CodexAccountSnapshot): AccountQuota {
  if (!snapshot.available) return { status: 'unavailable', windows: [] };
  if (!snapshot.account) return { status: 'signedOut', windows: [] };
  const windows = snapshot.rateLimits.flatMap(bucket => (['primary', 'secondary'] as const).flatMap(kind => {
    const window = bucket[kind];
    if (!window) return [];
    const reset = typeof window.resetsAt === 'number' ? new Date(window.resetsAt * 1000) : null;
    return [{ id: `${bucket.id}:${kind}`, label: bucket.name || bucket.id, kind,
      durationMinutes: window.windowDurationMins, usedPercent: quotaPercent(window.usedPercent),
      resetsAt: reset && Number.isFinite(reset.getTime()) ? reset.toISOString() : null }];
  }));
  return { status: windows.length ? 'ready' : snapshot.account.accountType === 'apiKey' ? 'unsupported' : 'unavailable',
    plan: snapshot.account.planType, windows };
}

export function copilotAccountQuota(snapshot: CopilotAccountSnapshot): AccountQuota {
  if (!snapshot.available) return { status: 'unavailable', windows: [] };
  if (!snapshot.authenticated) return { status: 'signedOut', windows: [] };
  const windows = snapshot.quotas.map(window => {
    const remaining = quotaPercent(window.remainingPercent);
    return { id: window.id, label: window.id, unlimited: window.unlimited,
      usedPercent: window.unlimited || remaining === null ? null : 100 - remaining, resetsAt: window.resetDate };
  });
  return { status: windows.length ? 'ready' : 'unavailable', windows };
}

export function externalAccountQuota(snapshot: ExternalAccountUsage): AccountQuota {
  return { status: snapshot.status, provider: snapshot.provider, plan: snapshot.plan,
    details: snapshot.status === 'ready' ? snapshot.details : [],
    windows: snapshot.status === 'ready' ? snapshot.windows.map((window, index) => ({
      ...window, id: `${index}:${window.label}`, usedPercent: quotaPercent(window.usedPercent),
    })) : [] };
}
