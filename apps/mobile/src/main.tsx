import { createRoot } from 'react-dom/client';
import { I18nProvider } from '../../desktop/src/i18n';
import { applyTheme, getInitialTheme } from '../../desktop/src/lib/theme';
import './mobile.css';
import { MobileApp } from './MobileApp';
applyTheme(getInitialTheme());
document.documentElement.dataset.remote = 'true';
createRoot(document.getElementById('root')!).render(<I18nProvider><MobileApp/></I18nProvider>);
