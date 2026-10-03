import { createRoot } from 'react-dom/client';
import { I18nProvider } from '../../src/i18n';
import { McpOAuthDisclosure } from '../../src/components/settings/McpOAuthPanel';
import '../../src/index.css';

createRoot(document.getElementById('root')!).render(<I18nProvider><main className="mx-auto mt-10 max-w-2xl rounded-xl border border-border bg-surface-1 text-text-primary"><h1 className="p-4 text-lg">Remote MCP connector</h1><McpOAuthDisclosure serverId="oauth-fixture" enabled /></main></I18nProvider>);
