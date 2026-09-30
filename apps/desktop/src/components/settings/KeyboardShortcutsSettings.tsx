import { useState } from 'react';
import { useTranslation } from '../../i18n';
import { DEFAULT_PALETTE_SHORTCUT, formatPaletteShortcut, setPaletteShortcut, shortcutFromEvent, usePaletteShortcut } from '../../lib/shortcuts';

export function KeyboardShortcutsSettings() {
  const { t } = useTranslation();
  const shortcut = usePaletteShortcut();
  const [invalid, setInvalid] = useState(false);
  return <div className="space-y-3 border-t border-border pt-5" data-testid="keyboard-shortcut-settings">
    <label htmlFor="palette-shortcut" className="block text-sm font-medium text-text-primary">{t('shortcuts.commandPalette')}</label>
    <p id="palette-shortcut-help" className="text-xs text-text-tertiary">{t('shortcuts.customizeHint')}</p>
    <div className="flex items-center gap-2">
      <input id="palette-shortcut" data-testid="palette-shortcut-input" aria-describedby="palette-shortcut-help" aria-invalid={invalid} readOnly value={formatPaletteShortcut(shortcut)} className="min-w-0 rounded-lg border border-border bg-surface-2 px-3 py-2 text-sm outline-accent" onKeyDown={event => {
        if (event.key === 'Tab' || event.key === 'Escape') return;
        event.preventDefault();
        event.stopPropagation();
        if (event.nativeEvent.isComposing || ['Control', 'Meta', 'Shift', 'Alt'].includes(event.key)) return;
        setInvalid(!setPaletteShortcut(shortcutFromEvent(event)));
      }} />
      <button type="button" className="rounded px-3 py-2 text-xs text-text-secondary hover:bg-surface-2" onClick={() => { setPaletteShortcut(DEFAULT_PALETTE_SHORTCUT); setInvalid(false); }}>{t('shortcuts.reset')}</button>
    </div>
    {invalid && <p role="alert" className="text-xs text-danger">{t('shortcuts.invalid')}</p>}
  </div>;
}
