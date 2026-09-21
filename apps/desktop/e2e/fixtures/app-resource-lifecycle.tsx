import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { ThemeProvider, useTheme } from '../../src/lib/ThemeProvider';

function Appearance() {
  const { activeThemeId } = useTheme();
  return <output data-testid="active-appearance">{activeThemeId}</output>;
}

const root = createRoot(document.getElementById('root')!);
root.render(<StrictMode><ThemeProvider><Appearance /></ThemeProvider></StrictMode>);
Object.assign(window, { unmountAppearance: () => root.unmount() });
