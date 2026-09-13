import { createRoot } from 'react-dom/client';
import { ApprovalDialog } from '../../src/components/chat/ApprovalDialog';
import { I18nProvider } from '../../src/i18n';
let root: ReturnType<typeof createRoot> | undefined;
export function renderApproval(targetKind: string) {
  if (!root) {
    const host = document.createElement('div');
    host.style.cssText = 'position:fixed;inset:0;z-index:9999';
    document.body.append(host); root = createRoot(host);
  }
  root.render(<I18nProvider><ApprovalDialog request={{
    id: 'approval-test', toolName: 'computer_control', targetKind,
    permissionKey: 'trusted-key', targetValue: 'verified-scope', argumentsPreview: '{}',
    riskLevel: 'high', reason: 'Control this verified editor window for the current task.',
  }} /></I18nProvider>);
}
