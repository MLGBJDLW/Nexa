export function SettingsToggle({ id, label, checked, onChange, disabled = false }: {
  id?: string; label: string; checked: boolean; onChange: (checked: boolean) => void; disabled?: boolean;
}) {
  return <button id={id} type="button" role="switch" aria-label={label} aria-checked={checked} disabled={disabled} onClick={() => onChange(!checked)}
    className={`relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50 focus-visible:ring-offset-2 disabled:opacity-50 motion-reduce:transition-none ${checked ? 'bg-accent' : 'bg-surface-3'}`}>
    <span aria-hidden="true" className={`h-3.5 w-3.5 rounded-full bg-white shadow-sm transition-transform duration-150 motion-reduce:transition-none ${checked ? 'translate-x-[1.125rem]' : 'translate-x-[0.1875rem]'}`} />
  </button>;
}
