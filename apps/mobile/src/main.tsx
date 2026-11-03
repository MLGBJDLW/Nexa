import { createRoot } from 'react-dom/client';
import { I18nProvider } from '../../desktop/src/i18n';
import { applyTheme, getInitialTheme } from '../../desktop/src/lib/theme';
import './mobile.css';
import { MobileApp } from './MobileApp';
applyTheme(getInitialTheme());
document.documentElement.dataset.remote = 'true';
// Android IME resizing differs across WebView versions. Follow the visible
// viewport so the composer and bottom sheets remain above the keyboard.
const viewport = window.visualViewport;
let viewportFrame = 0;
const syncViewport = () => {
  cancelAnimationFrame(viewportFrame);
  viewportFrame = requestAnimationFrame(() => {
    if (viewport?.scale === 1) document.documentElement.style.setProperty('--mobile-viewport-height', `${viewport.height}px`);
  });
};
viewport?.addEventListener('resize', syncViewport);
syncViewport();
createRoot(document.getElementById('root')!).render(<I18nProvider><MobileApp/></I18nProvider>);
