import { createRoot, type Root } from 'react-dom/client';
import { ThinkingBlock } from '../../src/components/chat/ThinkingBlock';
import { I18nProvider } from '../../src/i18n';

let root: Root | undefined;
export function renderTrace(text: string, cardHeight: number) {
  if (!root) {
    const host = document.createElement('div');
    host.style.cssText = 'position:fixed;inset:20px;background:var(--color-surface-1);z-index:9999';
    document.body.append(host);
    root = createRoot(host);
  }
  root.render(<I18nProvider><ThinkingBlock content={text} isStreaming sections={[{
    text,
    node: cardHeight ? <div data-testid="trace-tool-card" style={{ height: cardHeight }}>Tool progress</div> : null,
  }]} /></I18nProvider>);
}
