import { useState } from 'react';
import { Plus, Trash2 } from 'lucide-react';
import { NexaSelect } from '../../components/ui/overlay';
import type { WorkflowCatalogTemplate, WorkflowCatalogTask } from '../../types/workflows';

type Props = { value: WorkflowCatalogTemplate; onChange: (value: WorkflowCatalogTemplate) => void; tr: (key: string) => string };
const inputClass = 'mt-1 w-full rounded-md border border-border/70 bg-surface-0 px-2 py-1.5 text-xs text-text-primary';
const roles = ['researcher', 'planner', 'writer', 'verifier', 'critic'];

/** Edits the existing recipe; the core graph validator owns run admission. */
export function WorkflowStageEditor({ value, onChange, tr }: Props) {
  const [selected, setSelected] = useState<string | null>(value.tasks[0]?.id ?? null);
  const current = value.tasks.find(task => task.id === selected) ?? value.tasks[0];
  const update = (patch: Partial<WorkflowCatalogTask>) => {
    if (current) onChange({ ...value, tasks: value.tasks.map(task => task.id === current.id ? { ...task, ...patch } : task) });
  };
  const add = () => {
    let index = value.tasks.length + 1;
    while (value.tasks.some(task => task.id === `stage-${index}`)) index++;
    const id = `stage-${index}`;
    onChange({ ...value, tasks: [...value.tasks, { id, roleId: 'researcher', roleLabel: tr('role.researcher'), task: '', expectedOutput: '', deliverableStyle: '', acceptanceCriteria: [], dependsOn: [] }] });
    setSelected(id);
  };
  const remove = () => {
    if (!current || value.tasks.length < 2) return;
    onChange({ ...value, tasks: value.tasks.filter(task => task.id !== current.id).map(task => ({ ...task, dependsOn: task.dependsOn?.filter(id => id !== current.id) })) });
    setSelected(null);
  };
  return <section data-testid="workflow-stage-editor" className="rounded-xl border border-border/70 p-3">
    <header className="mb-3 flex items-center justify-between"><h3 className="text-xs font-semibold text-text-primary">{tr('authoringStages')}</h3><button type="button" onClick={add} className="flex items-center gap-1 text-xs text-accent"><Plus size={13} />{tr('stageAdd')}</button></header>
    <div aria-label={tr('authoringStages')} className="mb-3 flex gap-2 overflow-x-auto pb-2">{value.tasks.map(task => <button key={task.id} type="button" aria-pressed={task.id === current?.id} onClick={() => setSelected(task.id)} className={`min-w-32 max-w-48 shrink-0 rounded-lg border p-2 text-left text-xs ${current?.id === task.id ? 'border-accent bg-accent/8' : 'border-border'}`}><span className="block font-medium text-text-primary">{task.id} · {task.roleLabel}</span><span className="mt-1 block break-words text-text-tertiary">{task.dependsOn?.length ? `← ${task.dependsOn.join(', ')}` : tr('authoringIndependent')}</span></button>)}</div>
    {current && <div className="space-y-3">
      <div className="flex items-end gap-3"><label className="min-w-0 flex-1 text-xs text-text-secondary">{tr('stageRole')}<NexaSelect value={current.roleId} className={inputClass} onChange={event => update({ roleId: event.target.value, roleLabel: tr(`role.${event.target.value}`) })}>{[...new Set([...roles, current.roleId])].map(role => <option key={role} value={role}>{tr(`role.${role}`)}</option>)}</NexaSelect></label><button type="button" aria-label={tr('delete')} onClick={remove} disabled={value.tasks.length < 2} className="rounded-lg p-2 text-danger disabled:opacity-40"><Trash2 size={14} /></button></div>
      <label className="block text-xs text-text-secondary">{tr('stageTask')}<textarea className={`${inputClass} min-h-20`} value={current.task} onChange={event => update({ task: event.target.value })} /></label>
      <label className="block text-xs text-text-secondary">{tr('stageOutput')}<textarea className={inputClass} value={current.expectedOutput} onChange={event => update({ expectedOutput: event.target.value })} /></label>
      <label className="block text-xs text-text-secondary">{tr('recordingSuccessCriteria')}<textarea className={inputClass} value={current.acceptanceCriteria.join('\n')} onChange={event => update({ acceptanceCriteria: event.target.value.split(/\r?\n/) })} /></label>
      <fieldset><legend className="text-xs text-text-secondary">{tr('authoringAfter')}</legend><div className="mt-1 flex flex-wrap gap-3">{value.tasks.filter(task => task.id !== current.id).map(task => <label key={task.id} className="flex items-center gap-1 text-xs text-text-primary"><input type="checkbox" checked={current.dependsOn?.includes(task.id) ?? false} onChange={event => update({ dependsOn: event.target.checked ? [...(current.dependsOn ?? []), task.id] : current.dependsOn?.filter(id => id !== task.id) })} />{task.id}</label>)}</div></fieldset>
      <label className="block text-xs text-text-secondary">{tr('stageParallelism')}<input className={inputClass} type="number" min={1} step={1} value={value.maxParallel} onChange={event => { const maxParallel = Number(event.target.value); if (Number.isSafeInteger(maxParallel) && maxParallel > 0) onChange({ ...value, maxParallel }); }} /></label>
    </div>}
  </section>;
}
