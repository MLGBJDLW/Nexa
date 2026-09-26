import { createRoot } from 'react-dom/client';
import { I18nProvider } from '../../src/i18n';
import { OverlayProvider } from '../../src/components/ui/overlay';
import { VectorStoreSection } from '../../src/components/settings/VectorStoreSection';
import '../../src/index.css';
createRoot(document.getElementById('root')!).render(<I18nProvider><OverlayProvider><div className="mx-auto max-w-3xl p-4 text-text-primary"><VectorStoreSection /></div></OverlayProvider></I18nProvider>);
