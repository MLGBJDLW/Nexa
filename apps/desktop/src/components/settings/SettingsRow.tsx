import type { ReactNode } from 'react';

/** Compact settings share alignment without squeezing narrow windows. */
export function SettingsRow({ label, description, children, htmlFor }: {
  label: string; description?: string; children: ReactNode; htmlFor?: string;
}) {
  return <div className="flex min-w-0 flex-col gap-2 py-2 sm:flex-row sm:items-center sm:justify-between sm:gap-6">
    <div className="min-w-0 flex-1">
      <label htmlFor={htmlFor} className="text-sm font-medium text-text-primary">{label}</label>
      {description && <p className="mt-0.5 text-xs leading-relaxed text-text-tertiary">{description}</p>}
    </div>
    <div className="min-w-0 sm:w-56 sm:shrink-0">{children}</div>
  </div>;
}

export const settingsSelectClass = 'w-full min-w-0 rounded-md border border-border bg-surface-2 px-2.5 py-1.5 text-sm text-text-primary focus:outline-none focus:ring-2 focus:ring-accent/40';
