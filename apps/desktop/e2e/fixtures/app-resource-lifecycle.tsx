import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { ThemeProvider, resolveDesktopThemeMedia, useTheme } from '../../src/lib/ThemeProvider';
import { ThemeBackdrop } from '../../src/components/ThemeBackdrop';

function Appearance() {
  const { activeThemeId, activeCustomTheme } = useTheme();
  return <>
    <ThemeBackdrop theme={activeCustomTheme} resolve={resolveDesktopThemeMedia} />
    <output data-testid="active-appearance">{activeThemeId}</output>
  </>;
}

const root = createRoot(document.getElementById('root')!);
root.render(<StrictMode><ThemeProvider><Appearance /></ThemeProvider></StrictMode>);
Object.assign(window, { unmountAppearance: () => root.unmount() });
