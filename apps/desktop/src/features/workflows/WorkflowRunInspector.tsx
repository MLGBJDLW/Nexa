import { useEffect, useState } from 'react';
import { CheckCircle2, Circle, GitBranch, Loader2, ShieldCheck, XCircle } from 'lucide-react';
import { useTranslation } from '../../i18n';
import { getAgentTaskRunPlan, getAgentTaskRunSummary } from '../../lib/api';
import { taskRunCanAcceptStop } from '../../lib/streaming/runReconciliation';
import type { AgentTaskRunListItem } from '../../types/conversation';

interface Node {
  id: string; title: string; phase: string; status: string; dependencies: string[];
  attempts: number; allowedTools: string[];
  deliverable?: { claims: string[]; evidence: string[]; filesTouched: string[]; tests: string[]; uncertainties: string[] } | null;
}
interface Workflow {
  version: number;
  projectVerificationPendingMutation?: boolean;
  nodes: Node[];
  verificationGates: Array<{ id: string; kind: string; required: boolean; passed: boolean | null; detail?: string | null }>;
  completionContract: { requireVerificationGates: boolean; requireInteractionGates: boolean; requireEvidenceLedger: boolean };
  checkpoint: { revision: number };
}

function workflowFromPlan(plan: Record<string, unknown> | null): Workflow | null {
  const workflow = plan?.workflowIr as Workflow | undefined;
  return workflow?.version === 1 && Array.isArray(workflow.nodes) && Array.isArray(workflow.verificationGates)
    && workflow.nodes.every(node => typeof node.id === 'string' && Array.isArray(node.dependencies))
    && workflow.completionContract && workflow.checkpoint ? workflow : null;
}

/** Read the same IR that gates completion, never infer success from UI events. */
export function WorkflowRunInspector({ runId, active, onTaskUpdate }: { runId: string; active: boolean; onTaskUpdate: (task: AgentTaskRunListItem) => void }) {
  const { t } = useTranslation();
  const [workflow, setWorkflow] = useState<Workflow | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  useEffect(() => {
    let alive = true;
    let pending = false;
    let poll = active;
    let timer: ReturnType<typeof setTimeout> | undefined;
    setWorkflow(null); setSelected(null); setLoading(true); setError(null);
    const load = async () => {
      if (!alive || pending || document.hidden) return;
      pending = true;
      clearTimeout(timer);
      try {
        const [plan, task] = await Promise.all([getAgentTaskRunPlan(runId), getAgentTaskRunSummary(runId)]);
        const value = workflowFromPlan(plan);
        if (alive) { setWorkflow(value); setError(null); poll = taskRunCanAcceptStop(task.run); onTaskUpdate(task); }
      } catch (reason) { if (alive) setError(String(reason)); }
      finally { pending = false; if (alive) { setLoading(false); if (poll) timer = setTimeout(() => void load(), 5_000); } }
    };
    const refresh = () => void load();
    void load();
    document.addEventListener('visibilitychange', refresh);
    return () => { alive = false; clearTimeout(timer); document.removeEventListener('visibilitychange', refresh); };
  }, [runId, active, onTaskUpdate]);
  if (!loading && !workflow && !error) return null;
  const current = workflow?.nodes.find(node => node.id === selected)
    ?? workflow?.nodes.find(node => ['running', 'failed', 'blocked'].includes(node.status)) ?? workflow?.nodes[0];
  return <section data-testid="workflow-run-inspector" aria-label={t('workflows.runTimeline')} className="rounded-lg border border-border/70 bg-surface-1/70 p-4">
    <header className="mb-3 flex items-center justify-between gap-2"><h2 className="flex items-center gap-2 text-sm font-semibold text-text-primary"><GitBranch size={16} className="text-accent" />{t('workflows.runTimeline')}</h2>{workflow && <span className="text-xs text-text-tertiary">rev {workflow.checkpoint.revision}</span>}{loading && <Loader2 size={14} className="animate-spin" />}</header>
    {error && <p role="alert" className="text-sm text-danger">{error}</p>}
    {workflow && <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(0,1.4fr)_minmax(0,1fr)]">
      <nav aria-label={t('workflows.authoringStages')} className="max-h-96 space-y-2 overflow-y-auto">{workflow.nodes.map(node => <button key={node.id} type="button" aria-current={current?.id === node.id ? 'step' : undefined} onClick={() => setSelected(node.id)} className={`block w-full rounded-lg border p-3 text-left text-xs ${current?.id === node.id ? 'border-accent/60 bg-accent/8' : 'border-border/70 bg-surface-0'}`}>
        <span className="flex items-center gap-2 text-text-primary">{node.status === 'succeeded' ? <CheckCircle2 size={14} className="shrink-0 text-success" /> : ['failed', 'blocked'].includes(node.status) ? <XCircle size={14} className="shrink-0 text-danger" /> : <Circle size={14} className="shrink-0 text-text-tertiary" />}<span className="break-words">{node.title}</span></span>
        <span className="mt-1 block break-words text-text-tertiary">{node.phase} · {node.status}{node.dependencies.length ? ` · ${t('workflows.authoringAfter')} ${node.dependencies.join(', ')}` : ''}</span>
      </button>)}</nav>
      {current && <div className="min-w-0 space-y-3 text-xs"><h3 className="font-medium text-text-primary">{current.title}</h3>
        {current.dependencies.length > 0 && <div className="flex flex-wrap gap-1">{current.dependencies.map(id => <button type="button" key={id} onClick={() => setSelected(id)} className="rounded bg-surface-2 px-2 py-1 text-accent">← {id}</button>)}</div>}
        <p className="break-words text-text-tertiary">{current.allowedTools?.join(' · ')}</p>
        {current.deliverable ? <div className="max-h-80 space-y-3 overflow-y-auto">{([
          ['claims', 'taskCenter.summaryLabel'], ['evidence', 'taskCenter.citations'], ['filesTouched', 'taskCenter.artifactPaths'], ['tests', 'workflows.verificationGates'], ['uncertainties', 'taskCenter.openQuestions'],
        ] as const).map(([field, key]) => current.deliverable![field]?.length > 0 && <div key={field}><h4 className="mb-1 font-medium text-text-primary">{t(key)}</h4><ul className="space-y-1 text-text-secondary">{current.deliverable![field].map((text, index) => <li className="break-words" key={index}>{text}</li>)}</ul></div>)}</div> : <p className="text-text-tertiary">{t('taskCenter.noArtifacts')}</p>}
      </div>}
      <aside className="min-w-0"><h3 className="mb-2 flex items-center gap-2 text-xs font-semibold text-text-primary"><ShieldCheck size={14} />{t('workflows.verificationGates')}</h3><div className="max-h-96 space-y-2 overflow-y-auto">{workflow.verificationGates.map(gate => {
        const interaction = gate.kind.startsWith('browser') || gate.kind.startsWith('desktop');
        const projectCheckPending = workflow.projectVerificationPendingMutation && ['tests', 'lint', 'typecheck', 'build'].includes(gate.kind);
        const enforced = !projectCheckPending && gate.required && (workflow.completionContract.requireVerificationGates || (interaction && workflow.completionContract.requireInteractionGates));
        return <article key={gate.id} className="rounded-lg border border-border/70 bg-surface-0 p-2 text-xs"><div className="flex flex-wrap items-center justify-between gap-1"><span className="font-medium text-text-primary">{gate.kind}</span><span className={gate.passed === false ? 'text-danger' : gate.passed ? 'text-success' : 'text-text-tertiary'}>{t(gate.passed === true ? 'taskCenter.completed' : gate.passed === false ? 'taskCenter.failed' : 'taskCenter.queued')}</span></div><p className="mt-1 text-text-tertiary">{t(enforced ? 'workflows.gateRequired' : 'workflows.gateAdvisory')}</p>{gate.detail && <p className="mt-1 break-words text-text-secondary">{gate.detail}</p>}</article>;
      })}</div></aside>
    </div>}
  </section>;
}
