import { useMemo, useRef, useState } from 'react';
import { Loader2, Play, Save, X } from 'lucide-react';
import { NexaSelect } from '../../components/ui/overlay';
import * as api from '../../lib/api';
import type { WorkflowCatalogTemplate } from '../../lib/api';
import type { Source } from '../../types';
import type { SaveWorkflowAutomationInput, WorkflowAuthoringPreview, WorkflowAutomation, WorkflowInputs, WorkflowRecipe, WorkflowRunOptions } from '../../types/workflows';

type WorkflowT = (key: string, params?: Record<string, string | number>) => string;
type Props = {
  template: WorkflowCatalogTemplate;
  automation?: WorkflowAutomation;
  mode: 'create' | 'edit' | 'run';
  sources: Source[];
  tr: WorkflowT;
  onClose: () => void;
  onSaved: (workflow: WorkflowAutomation) => Promise<void>;
  onRun: (workflow: WorkflowAutomation, options: WorkflowRunOptions) => Promise<void>;
};

function defaultInputs(): WorkflowInputs {
  return { goal: '', context: '', constraints: [], deliverable: { format: 'markdown', instructions: '' }, replayValues: [] };
}

const inputClass = 'w-full rounded-lg border border-border/70 bg-surface-0 px-3 py-2 text-sm text-text-primary outline-none focus:border-accent';
const actionClass = 'inline-flex h-9 items-center justify-center gap-2 rounded-lg border border-border px-3 text-sm disabled:opacity-45';

export function WorkflowAuthoringPanel({ template, automation, mode, sources, tr, onClose, onSaved, onRun }: Props) {
  const existingRecipe = automation?.recipe?.version === 1 ? automation.recipe : null;
  const [name, setName] = useState(automation?.name ?? template.label);
  const [inputs, setInputs] = useState<WorkflowInputs>(() => structuredClone(existingRecipe?.inputs ?? defaultInputs()));
  const [constraints, setConstraints] = useState(inputs.constraints.join('\n'));
  const [instructions, setInstructions] = useState(existingRecipe?.customInstructions ?? '');
  const [sourceScope, setSourceScope] = useState(automation?.sourceScope ?? []);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [preview, setPreview] = useState<WorkflowAuthoringPreview | null>(null);
  const requestId = useRef(crypto.randomUUID());
  const runOnly = mode === 'run';
  const recipe = useMemo<WorkflowRecipe>(() => ({
    version: 1,
    kind: existingRecipe?.kind ?? (instructions.trim() ? 'custom' : 'template'),
    templateSnapshot: existingRecipe?.templateSnapshot ?? { ...template, version: template.version ?? 1 },
    inputs: { ...inputs, constraints: constraints.split(/\r?\n/).filter(line => line.trim()) },
    customInstructions: instructions,
    recording: existingRecipe?.recording ?? null,
  }), [existingRecipe, constraints, inputs, instructions, template]);
  const candidate = (copy = false): SaveWorkflowAutomationInput => ({
    id: copy ? null : automation?.id,
    name, description: automation?.description ?? inputs.goal,
    workflowTemplateId: template.id, prompt: automation?.prompt ?? '', recipe,
    trigger: automation?.trigger ?? { kind: 'manual' }, sourceScope,
    approvalPolicy: automation?.approvalPolicy ?? { requireBeforeRun: true, allowedTools: [], riskLevel: 'medium' },
    scheduleConfig: automation?.scheduleConfig, enabled: automation?.enabled ?? true,
    expectedRevision: copy ? undefined : automation?.definitionRevision,
  });
  const updateInputs = (patch: Partial<WorkflowInputs>) => { setInputs(current => ({ ...current, ...patch })); setPreview(null); setError(null); };

  const inspect = async () => {
    setBusy(true); setError(null);
    try {
      setPreview(runOnly && automation ? await api.previewSavedWorkflowRun(automation.id, recipe.inputs) : await api.previewWorkflowAuthoring(candidate()));
    } catch (reason) { setError(String(reason)); } finally { setBusy(false); }
  };
  const submit = async (run: boolean, copy = false) => {
    if (busy) return;
    setBusy(true); setError(null);
    try {
      let saved = automation;
      if (!runOnly) {
        await api.previewWorkflowAuthoring(candidate(copy));
        saved = await api.saveWorkflowAutomation(candidate(copy));
      }
      if (!saved) throw new Error(tr('namePromptRequired'));
      if (run) {
        const checked = await api.previewSavedWorkflowRun(saved.id, recipe.inputs);
        await onRun(saved, { inputs: recipe.inputs, expectedRevision: saved.definitionRevision, previewDigest: checked.contentDigest, clientRequestId: requestId.current });
        onClose();
      } else { await onSaved(saved); }
    } catch (reason) { setError(String(reason)); } finally { setBusy(false); }
  };
  const supported = !automation?.recipe || automation.recipe.version === 1;

  return <div className="fixed inset-0 z-60 flex items-center justify-center bg-black/50 p-4 backdrop-blur-sm">
    <section role="dialog" aria-modal="true" aria-labelledby="workflow-authoring-title" data-testid="workflow-authoring" className="flex max-h-[92vh] w-full max-w-4xl flex-col overflow-hidden rounded-2xl border border-border bg-surface-1 shadow-2xl">
      <header className="flex items-start justify-between border-b border-border px-5 py-4">
        <div><h2 id="workflow-authoring-title" className="font-semibold text-text-primary">{runOnly ? tr('authoringRunInputs') : tr('authoringTitle')}</h2><p className="mt-1 text-xs text-text-secondary">{template.label}{automation?.definitionRevision ? ` · rev ${automation.definitionRevision}` : ''}</p></div>
        <button type="button" className="rounded-lg p-2 text-text-secondary hover:bg-surface-2" aria-label={tr('authoringClose')} onClick={onClose} disabled={busy}><X size={18} /></button>
      </header>
      <div className="overflow-y-auto p-5">
        {!supported ? <p role="alert" className="text-sm text-danger">{tr('authoringUnsupported')}</p> : <fieldset disabled={busy} className="grid gap-5 md:grid-cols-[1fr_240px]">
          <div className="space-y-3">
            {!runOnly && <label className="block text-xs text-text-secondary">{tr('name')}<input className={`${inputClass} mt-1`} value={name} onChange={event => { setName(event.target.value); setPreview(null); }} /></label>}
            <label className="block text-xs text-text-secondary">{tr('recordingObjective')}<textarea className={`${inputClass} mt-1 min-h-20`} value={inputs.goal} onChange={event => updateInputs({ goal: event.target.value })} /></label>
            <label className="block text-xs text-text-secondary">{tr('recordingContext')}<textarea className={`${inputClass} mt-1 min-h-28`} value={inputs.context} onChange={event => updateInputs({ context: event.target.value })} /></label>
            <label className="block text-xs text-text-secondary">{tr('authoringConstraints')}<textarea className={`${inputClass} mt-1 min-h-20`} value={constraints} onChange={event => { setConstraints(event.target.value); setPreview(null); }} /></label>
            <div className="grid gap-3 sm:grid-cols-[150px_1fr]">
              <label className="block text-xs text-text-secondary">{tr('authoringFormat')}<NexaSelect className={`${inputClass} mt-1`} value={inputs.deliverable.format} onChange={event => updateInputs({ deliverable: { ...inputs.deliverable, format: event.target.value as WorkflowInputs['deliverable']['format'] } })}>{(['answer', 'markdown', 'table', 'checklist'] as const).map(format => <option key={format} value={format}>{tr(`authoringFormat_${format}`)}</option>)}</NexaSelect></label>
              <label className="block text-xs text-text-secondary">{tr('authoringDeliverable')}<input className={`${inputClass} mt-1`} value={inputs.deliverable.instructions} onChange={event => updateInputs({ deliverable: { ...inputs.deliverable, instructions: event.target.value } })} /></label>
            </div>
            {recipe.recording && <label className="block text-xs text-text-secondary">{tr('recordingReplayValues')}<textarea className={`${inputClass} mt-1`} value={inputs.replayValues.join('\n')} onChange={event => updateInputs({ replayValues: event.target.value.split(/\r?\n/) })} /></label>}
            {!runOnly && <label className="block text-xs text-text-secondary">{tr('authoringInstructions')}<textarea className={`${inputClass} mt-1 min-h-20`} value={instructions} onChange={event => { setInstructions(event.target.value); setPreview(null); }} /></label>}
          </div>
          <aside className="space-y-4">
            <div className="rounded-xl border border-border bg-surface-0 p-3"><h3 className="text-xs font-semibold text-text-primary">{tr('authoringStages')}</h3><ol className="mt-2 space-y-2">{recipe.recording
              ? recipe.recording.steps.map((step, index) => <li key={step.id} className="rounded-lg border border-border/70 bg-surface-1 p-2 text-xs text-text-primary">{index + 1}. {step.text}</li>)
              : recipe.templateSnapshot.tasks.map(task => <li key={task.id} className="rounded-lg border border-border/70 bg-surface-1 p-2 text-xs"><span className="font-medium text-text-primary">{task.roleLabel}</span><p className="mt-1 text-text-tertiary">{task.dependsOn?.length ? `${tr('authoringAfter')} ${task.dependsOn.join(', ')}` : tr('authoringIndependent')}</p></li>)}</ol></div>
            <div className="space-y-2 text-xs text-text-secondary"><h3 className="font-semibold text-text-primary">{tr('sourceScope')}</h3>{sources.map(source => <label key={source.id} className="flex items-start gap-2 break-all"><input type="checkbox" className="mt-0.5" disabled={runOnly} checked={sourceScope.includes(source.id)} onChange={event => { setSourceScope(current => event.target.checked ? [...current, source.id] : current.filter(id => id !== source.id)); setPreview(null); }} />{source.rootPath}</label>)}</div>
            {runOnly && <p className="rounded-lg bg-accent/8 p-3 text-xs leading-5 text-text-secondary">{tr('authoringRunOnlyHint')}</p>}
          </aside>
        </fieldset>}
        {error && <p role="alert" className="mt-4 rounded-lg border border-danger/30 bg-danger/8 p-3 text-sm text-danger">{error}</p>}
        {preview && <details open className="mt-4 rounded-xl border border-border p-3"><summary className="cursor-pointer text-sm font-medium text-text-primary">{tr('authoringPreview')}</summary><pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap text-xs leading-5 text-text-secondary">{preview.prompt}</pre></details>}
      </div>
      <footer className="flex flex-wrap justify-end gap-2 border-t border-border p-4">
        <button type="button" className={actionClass} disabled={busy || !supported} onClick={() => void inspect()}>{tr('authoringPreview')}</button>
        {!runOnly && <button type="button" className={actionClass} disabled={busy || !supported} onClick={() => void submit(false)}><Save size={14} />{automation ? tr('saveAutomation') : tr('authoringSaveCopy')}</button>}
        {!runOnly && automation && <button type="button" className={actionClass} disabled={busy || !supported} onClick={() => void submit(false, true)}>{tr('authoringSaveCopy')}</button>}
        <button type="button" className={`${actionClass} bg-accent text-white`} disabled={busy || !supported || !inputs.goal.trim()} onClick={() => void submit(true)}>{busy ? <Loader2 size={14} className="animate-spin" /> : <Play size={14} />}{runOnly ? tr('run') : tr('authoringSaveRun')}</button>
      </footer>
    </section>
  </div>;
}
