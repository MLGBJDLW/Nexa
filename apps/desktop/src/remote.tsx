import { createRoot } from 'react-dom/client';
import { I18nProvider } from './i18n';
import { MobileApp } from './features/remote/MobileApp';
import '@fontsource-variable/inter';
import './index.css';
import './features/remote/remote.css';
import { applyTheme, getInitialTheme } from './lib/theme';
import 'katex/dist/katex.min.css';
applyTheme(getInitialTheme());
document.documentElement.dataset.remote = 'true';
createRoot(document.getElementById('root')!).render(
  <I18nProvider>
    <MobileApp />
  </I18nProvider>,
);
