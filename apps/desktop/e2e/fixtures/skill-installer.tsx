import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { I18nProvider } from '../../src/i18n';
import { OverlayProvider } from '../../src/components/ui/overlay';
import { SkillInstaller } from '../../src/components/settings/SkillInstaller';
import '../../src/index.css';

function Fixture() {
  const [installed, setInstalled] = useState(0);
  return <main className="p-8"><SkillInstaller skills={[]} onInstalled={() => setInstalled(value => value + 1)} />
    <output data-testid="installation-refresh-count">{installed}</output></main>;
}
createRoot(document.getElementById('root')!).render(<I18nProvider><OverlayProvider><Fixture /></OverlayProvider></I18nProvider>);
