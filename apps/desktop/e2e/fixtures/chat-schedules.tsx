import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router';
import { I18nProvider } from '../../src/i18n';
import { TaskBoard } from '../../src/components/chat/TaskBoard';
import { ChatSidebar } from '../../src/components/chat/ChatSidebar';
import { useConversationSchedules } from '../../src/lib/useConversationSchedules';
import type { WorkflowAutomation } from '../../src/types/workflows';
import type { Conversation } from '../../src/types/conversation';

const task: WorkflowAutomation = { id: 'daily', conversationId: 'scheduled-chat', name: 'Morning research', description: '', workflowTemplateId: 'report_brief', prompt: 'Research', sourceScope: [], enabled: true, status: 'completed', triggerKind: 'schedule', trigger: { kind: 'schedule', cron: '0 8 * * *' }, approvalPolicy: { requireBeforeRun: false, allowedTools: [], riskLevel: 'medium' }, scheduleConfig: { version: 2, timezone: 'Asia/Shanghai', misfirePolicy: 'run_latest', misfireGraceSeconds: 300, overlapPolicy: 'skip', legacyNeedsReview: false, executionPolicy: { workspacePolicy: 'deny_writes', powerMode: 'standard', orchestrationProfile: 'balanced', collaborationMode: 'direct' } }, nextRunAt: '2026-11-04T00:00:00Z', createdAt: '2026-11-02T00:00:00Z', updatedAt: '2026-11-03T00:38:55Z' };
let deferNextRead = false;
Object.assign(window, { holdNextScheduleRead() { deferNextRead = true; window.dispatchEvent(new Event('nexa:schedules-changed')); }, __TAURI_INTERNALS__: { async invoke(command: string, args: any) {
  if (command === 'list_workflow_automations_cmd') {
    const snapshot = [structuredClone(task)];
    if (deferNextRead) { deferNextRead = false; return new Promise(resolve => Object.assign(window, { releaseScheduleRead: () => resolve(snapshot) })); }
    return snapshot;
  }
  if (command === 'set_workflow_automation_enabled_cmd') { task.enabled = args.enabled; return structuredClone(task); }
  return [];
} } });
const conversation = { id: 'scheduled-chat', title: 'Daily research', provider: 'mock', model: 'mock', systemPrompt: '', projectId: null, collectionContext: null, createdAt: task.createdAt, updatedAt: task.updatedAt } as Conversation;
function Fixture() {
  const [revision] = useState('initial');
  const schedules = useConversationSchedules(revision);
  return <main className="relative flex h-screen bg-surface-0 text-text-primary"><aside className="w-72 border-r border-border"><ChatSidebar conversations={[conversation]} activeId={conversation.id} activeProjectId={null} runningConversationIds={new Set()} activeConversationArchived={false} schedulesByConversation={schedules.byConversation} onProjectChange={() => {}} onSelect={() => {}} onNew={() => {}} onArchive={() => {}} onDelete={() => {}} onRename={() => {}} onDeleteBatch={() => {}} onDeleteAll={() => {}} onSelectArchived={() => {}} /></aside><TaskBoard messages={[]} toolCalls={[]} schedules={schedules.byConversation.get(conversation.id)} onSchedulesRefresh={schedules.refresh} /></main>;
}
createRoot(document.getElementById('root') ?? document.body.appendChild(document.createElement('div'))).render(<MemoryRouter><I18nProvider><Fixture /></I18nProvider></MemoryRouter>);
