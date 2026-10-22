import type { ReactNode } from 'react';

/** Compact settings share alignment without squeezing narrow windows. */
export function SettingsRow({ label, description, children, htmlFor }: {
  label: string; description?: string; children: ReactNode; htmlFor?: string;
}) {
  return <div className="flex min-w-0 items-center justify-between gap-3 py-2 sm:gap-6" data-settings-row>
    <div className="min-w-0 flex-1">
      <label htmlFor={htmlFor} className="text-sm font-medium text-text-primary">{label}</label>
      {description && <p className="mt-0.5 text-xs leading-relaxed text-text-tertiary">{description}</p>}
    </div>
    <div className="flex min-w-0 w-[42%] max-w-56 shrink-0 flex-col items-stretch [&>button[role=switch]]:self-end">{children}</div>
  </div>;
}

export const settingsSelectClass = 'w-full min-w-0 rounded-md border border-border bg-surface-2 px-2.5 py-1.5 text-sm text-text-primary focus:outline-none focus:ring-2 focus:ring-accent/40';
