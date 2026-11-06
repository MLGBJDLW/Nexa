import { useCallback, useEffect, useRef, useState } from 'react';
import { CalendarClock, Check, ChevronDown, History, Pause, Pencil, Play, Plus, Trash2, X } from 'lucide-react';
import { useTranslation } from '../../i18n';
import { MobileDialog } from './MobileChatShell';
import type { RemoteChatClient } from './remoteClient';
import type { WorkflowAutomation } from '../../types/workflows';

type Run = { id: string; status: string; scheduledFor?: string; createdAt: string; summary?: string };
type Editor = { id?: string; expectedRevision?: number; requestId?: string; name: string; prompt: string; cron: string; timezone: string; allowedTools: string[]; requireApproval: boolean };
const blankEditor = (): Editor => ({
  requestId: crypto.randomUUID(), name: '', prompt: '', cron: '0 9 * * *',
  timezone: Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC', allowedTools: [], requireApproval: false,
});

export function MobileSchedules({ client, conversationId, ensureChat, onClose }: {
  client: RemoteChatClient; conversationId: string; ensureChat: () => Promise<string>; onClose(): void;
}) {
  const { t, locale } = useTranslation();
  const [tasks, setTasks] = useState<WorkflowAutomation[]>([]);
  const [editor, setEditor] = useState<Editor | null>(null);
  const [removing, setRemoving] = useState<WorkflowAutomation | null>(null);
  const [history, setHistory] = useState<{ id: string; runs: Run[] } | null>(null);
  const [preview, setPreview] = useState<{ cron: string; timezone: string; dates: string[]; error?: string } | null>(null);
  const [hasMore, setHasMore] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const request = useRef(0);
  const lock = useRef(false);
  const rpc = useCallback(<T,>(id: string, input: Record<string, unknown>) =>
    client.rpc<T>('chat.schedules', {conversationId: id, input}), [client]);
  const refresh = useCallback(async (id: string) => {
    const generation = ++request.current;
    if (!id) { setTasks([]); return; }
    const result = await rpc<{ tasks: WorkflowAutomation[] }>(id, {action:'list'});
    if (generation === request.current) setTasks(result.tasks);
  }, [rpc]);
  useEffect(() => {
    void refresh(conversationId).catch(e => setError(String(e)));
    return () => { request.current++; };
  }, [conversationId, refresh]);
  const cron = editor?.cron.trim();
  const zone = editor?.timezone.trim();
  useEffect(() => {
    if (!cron || !zone) { setPreview(null); return; }
    let active = true;
    const timer = setTimeout(() => {
      void rpc<{ occurrences: { scheduledFor: string }[] }>(conversationId, {action:'preview', cron, timezone:zone, limit:5})
        .then(result => { if (active) setPreview({cron, timezone:zone, dates:result.occurrences.map(item=>item.scheduledFor)}); })
        .catch(e => { if (active) setPreview({cron, timezone:zone, dates:[], error:String(e)}); });
    }, 300);
    return () => { active = false; clearTimeout(timer); };
  }, [cron, zone, conversationId, rpc]);
  const currentPreview = preview && preview.cron === cron && preview.timezone === zone ? preview : null;
  const act = async (run: () => Promise<void>) => {
    if (lock.current) return;
    lock.current = true; setBusy(true); setError('');
    try { await run(); } catch (e) { setError(String(e)); }
    finally { lock.current = false; setBusy(false); }
  };
  const save = async () => {
    if (!editor || !currentPreview?.dates.length) return;
    const id = conversationId || await ensureChat();
    await rpc(id, { action: editor.id ? 'update' : 'create', ...editor });
    setEditor(null); setHistory(null); await refresh(id);
  };
  const mutate = async (task: WorkflowAutomation, action: string) => {
    await rpc(conversationId, {action, id: task.id, expectedRevision: task.definitionRevision});
    setRemoving(null); setHistory(null); await refresh(conversationId);
  };
  const showHistory = async (id: string, before?: string) => {
    const { runs } = await rpc<{ runs: Run[] }>(conversationId, {action:'history', id, before, limit:25});
    setHistory(previous => ({id, runs: before && previous?.id === id ? [...previous.runs, ...runs] : runs}));
    setHasMore(runs.length === 25);
  };
  const time = (value?: string | null, zone?: string) => value
    ? new Date(value).toLocaleString(locale, {timeZone: zone, dateStyle:'short', timeStyle:'short'}) : '—';
  const edit = (task: WorkflowAutomation) => {
    if (task.trigger.kind !== 'schedule') return;
    setEditor({ id:task.id, expectedRevision:task.definitionRevision, name:task.name, prompt:task.prompt,
      cron:task.trigger.cron, timezone:task.scheduleConfig.timezone,
      allowedTools:task.approvalPolicy.allowedTools, requireApproval:task.approvalPolicy.requireBeforeRun });
  };
  return <MobileDialog title={t('chat.scheduledTasks')} onClose={onClose}>
    <div className="mobile-schedules">
      {error && <p className="mobile-error" role="alert">{error}</p>}
      {editor ? <form className="mobile-settings-form" onSubmit={event => { event.preventDefault(); void act(save); }}>
        <fieldset disabled={busy}>
          {(['name','prompt','cron','timezone'] as const).map(field => <label key={field}>{t(`workflows.${field}`)}
            {field === 'prompt' ? <textarea rows={3} required value={editor.prompt} onChange={e => setEditor({...editor,prompt:e.target.value})}/>
              : <input required value={editor[field]} onChange={e => setEditor({...editor,[field]:e.target.value})}/>}
          </label>)}
          <p className="mobile-section-hint">{t('workflows.cronHelp')}</p>
          {currentPreview?.error && <p className="mobile-error" role="alert">{currentPreview.error}</p>}
          {currentPreview && currentPreview.dates.length > 0 && <div aria-live="polite"><strong>{t('workflows.nextOccurrences')}</strong>
            <ul>{currentPreview.dates.map(date=><li key={date}>{time(date,currentPreview.timezone)}</li>)}</ul>
          </div>}
          <fieldset><legend>{t('workflows.allowedTools')}</legend>
            {['web_search','fetch_url'].map(tool => <label key={tool} className="mobile-schedule-check">
              <input type="checkbox" checked={editor.allowedTools.includes(tool)} onChange={e=>setEditor({...editor,
                allowedTools:e.target.checked ? [...editor.allowedTools,tool] : editor.allowedTools.filter(name=>name!==tool)})}/>{tool}
            </label>)}
          </fieldset>
          <label className="mobile-schedule-check"><input type="checkbox" checked={editor.requireApproval} onChange={e=>setEditor({...editor,requireApproval:e.target.checked})}/>{t('workflows.approvalRequired')}</label>
        </fieldset>
        <div className="mobile-confirm-actions"><button type="button" disabled={busy} onClick={()=>setEditor(null)}>{t('common.cancel')}</button><button className="mobile-primary" disabled={busy || !currentPreview?.dates.length}>{t('workflows.saveAutomation')}</button></div>
      </form> : <>
        <button className="mobile-drawer-new" disabled={busy} onClick={()=>setEditor(blankEditor())}><Plus size={16}/>{t('workflows.createAutomation')}</button>
        {tasks.map(task => <section key={task.id}>
          <strong><CalendarClock size={16}/>{task.name}</strong>
          <p>{task.prompt}</p>
          <p>{t('workflows.allowedTools')} · {task.approvalPolicy.allowedTools.join(', ') || t('common.none')}</p>
          <p>{task.trigger.kind === 'schedule' && task.trigger.cron} · {task.scheduleConfig.timezone}</p>
          <p>{task.enabled ? `${t('workflows.nextRun')} · ${time(task.nextRunAt, task.scheduleConfig.timezone)}` : t('chat.schedulePaused')}</p>
          <div className="mobile-schedule-controls">
            <button disabled={busy} aria-label={t('common.edit')} onClick={()=>edit(task)}><Pencil size={17}/></button>
            <button disabled={busy} aria-label={t(task.enabled?'chat.schedulePause':'chat.scheduleResume')} onClick={()=>void act(()=>mutate(task,task.enabled?'pause':'resume'))}>{task.enabled?<Pause size={17}/>:<Play size={17}/>}</button>
            <button disabled={busy} aria-label={t('workflows.authoringHistory')} onClick={()=>history?.id===task.id?setHistory(null):void act(()=>showHistory(task.id))}><History size={17}/></button>
            <button disabled={busy} aria-label={t('common.delete')} onClick={()=>setRemoving(task)}><Trash2 size={17}/></button>
          </div>
          {history?.id===task.id && <ul>{history.runs.map(run=><li key={run.id}>{time(run.scheduledFor??run.createdAt,task.scheduleConfig.timezone)} · {run.status}
            {run.summary && <p>{run.summary}</p>}{run.status==='waiting_approval' && <div className="mobile-confirm-actions">
              {[false,true].map(allow=><button key={String(allow)} disabled={busy} onClick={()=>void act(async()=>{
                await client.rpc('chat.scheduleDecision',{conversationId,runId:run.id,allow}); await showHistory(task.id); await refresh(conversationId);
              })}>{allow?<Check size={15}/>:<X size={15}/>} {t(allow?'workflows.approve':'workflows.deny')}</button>)}
            </div>}</li>)}{!history.runs.length && <li>—</li>}
            {hasMore && <li><button disabled={busy} onClick={()=>void act(()=>showHistory(task.id,history.runs[history.runs.length-1]?.id))}><ChevronDown size={16}/>{t('remote.moreChats')}</button></li>}</ul>}
        </section>)}
        <button disabled={busy} onClick={()=>void act(()=>refresh(conversationId))}>{t('workflows.refresh')}</button>
      </>}
      <MobileDialog open={!!removing} title={t('common.delete')} onClose={()=>{if(!busy)setRemoving(null);}}>
        <p>{removing?.name}</p>{error && <p role="alert">{error}</p>}<div className="mobile-confirm-actions">
          <button disabled={busy} onClick={()=>setRemoving(null)}>{t('common.cancel')}</button>
          <button disabled={busy} onClick={()=>{ if(removing)void act(()=>mutate(removing,'delete')); }}>{t('common.delete')}</button>
        </div>
      </MobileDialog>
    </div>
  </MobileDialog>;
}
