import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from '../../i18n';
import { Button } from '../ui/Button';
import { Input } from '../ui/Input';

interface ReviewFile { path:string; patch:string; truncated:boolean; binary:boolean }
interface Finding { id:string; anchor:{revision:string;path:string;side:string;line:number}; priority:number; title:string; body:string; status:string; stale:boolean }
interface Review { id:string; mode:string; baseRef:string; baseSha:string; workspaceRoot:string; createdAt:string; snapshot:{revision:string;headSha:string;baseline:string;files:ReviewFile[];truncated:boolean}; findings:Finding[]; pullRequest: null | {url:string;title:string;headSha:string;state:string;draft:boolean;reviewDecision:string|null;observedAt:string;matchesLocalHead:boolean;partial:boolean;checks:Array<{name:string;state:string;url:string|null}>;unresolvedThreads:Array<{id:string;path:string;line:number|null;outdated:boolean;body:string;url:string|null}>} }
interface SavedReview { id:string; mode:string; baseRef:string; createdAt:string }
interface Packet { marker:string; text:string }
const selectStyle = 'w-full rounded border border-border bg-surface-1 p-2 text-xs';
function diffLines(patch:string) {
  let old = 0; let next = 0; let active = false;
  return patch.split('\n').map(text => {
    const header = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(text);
    if (header) { old=Number(header[1]); next=Number(header[2]); active=true; return {text,old:null,next:null}; }
    const row:{text:string;old:number|null;next:number|null}={text,old:null,next:null};
    if (active && text.startsWith('+')) row.next=next++;
    else if (active && text.startsWith('-')) row.old=old++;
    else if (active && text.startsWith(' ')) { row.old=old++; row.next=next++; }
    else if (!text.startsWith('\\')) active=false;
    return row;
  });
}
function SafeLink({ url, children }: {url:string|null;children:React.ReactNode}) {
  if (!url || !/^https:\/\//i.test(url)) return <span>{children}</span>;
  return <a className="text-accent hover:underline" href={url} target="_blank" rel="noreferrer">{children}</a>;
}
export function CodeReviewPanel({ conversationId, onInsert }: {conversationId:string;onInsert:(packet:Packet)=>void}) {
  const { t }=useTranslation();
  const modeLabel=(value:string)=>t(value==='branch'?'chat.reviewMode_branch':value==='staged'?'chat.reviewMode_staged':value==='unstaged'?'chat.reviewMode_unstaged':'chat.reviewMode_worktree');
  const statusLabel=(value:string)=>t(value==='accepted'?'chat.reviewStatus_accepted':value==='resolved'?'chat.reviewStatus_resolved':value==='dismissed'?'chat.reviewStatus_dismissed':'chat.reviewStatus_open');
  const [review,setReview]=useState<Review|null>(null); const [saved,setSaved]=useState<SavedReview[]>([]);
  const [mode,setMode]=useState('worktree'); const [base,setBase]=useState('HEAD'); const [path,setPath]=useState('');
  const [line,setLine]=useState('1'); const [side,setSide]=useState('new'); const [priority,setPriority]=useState('2');
  const [title,setTitle]=useState(''); const [body,setBody]=useState(''); const [url,setUrl]=useState('');
  const [selected,setSelected]=useState<string[]>([]); const [busy,setBusy]=useState(false); const [error,setError]=useState<string|null>(null);
  const sequence=useRef(0);
  const apply=(value:Review|null) => { setReview(value); setSelected([]); setPath(current=>value?.snapshot.files.some(file=>file.path===current)?current:value?.snapshot.files[0]?.path??''); setUrl(value?.pullRequest?.url??''); };
  useEffect(()=>{
    const current=++sequence.current; setBusy(true); setError(null); apply(null); setSaved([]);
    void Promise.all([invoke<Review|null>('code_review_cmd',{conversationId,request:{action:'get'}}),invoke<SavedReview[]>('code_review_cmd',{conversationId,request:{action:'list'}})]).then(([value,history])=>{if(current===sequence.current){apply(value);setSaved(history);}}).catch(cause=>{if(current===sequence.current)setError(String(cause));}).finally(()=>{if(current===sequence.current)setBusy(false);});
    return ()=>{sequence.current++;};
  },[conversationId]);
  const run=async(request:Record<string,unknown>)=>{
    const current=++sequence.current; setBusy(true); setError(null);
    try {
      const value=await invoke<Review|Packet|null>('code_review_cmd',{conversationId,request});
      if(current!==sequence.current)return;
      if(request.action==='feedback'){onInsert(value as Packet);return;}
      apply(value as Review|null);
      if(request.action==='add'){setTitle('');setBody('');}
      const history=await invoke<SavedReview[]>('code_review_cmd',{conversationId,request:{action:'list'}}); if(current===sequence.current)setSaved(history);
    }catch(cause){if(current===sequence.current)setError(String(cause));}finally{if(current===sequence.current)setBusy(false);}
  };
  const file=review?.snapshot.files.find(entry=>entry.path===path);
  const feedback=(ids:string[])=>{if(review)void run({action:'feedback',review_id:review.id,revision:review.snapshot.revision,ids});};
  const status=(finding:Finding,value:string)=>{if(review)void run({action:'disposition',review_id:review.id,revision:review.snapshot.revision,finding_id:finding.id,status:value});};
  return <div data-testid="code-review-panel" className="space-y-4 text-xs">
    <p className="leading-5 text-text-secondary">{t('chat.reviewHelp')}</p>
    {error&&<p role="alert" className="break-words text-danger">{error}</p>}
    <div className="flex flex-wrap items-end gap-2">
      <label className="min-w-36 flex-1 space-y-1">{t('chat.reviewComparison')}<select aria-label={t('chat.reviewComparison')} className={selectStyle} disabled={busy} value={mode} onChange={event=>setMode(event.target.value)}>{['worktree','staged','unstaged','branch'].map(value=><option key={value} value={value}>{modeLabel(value)}</option>)}</select></label>
      {mode==='branch'&&<label className="min-w-36 flex-1 space-y-1">{t('chat.worktreeStart')}<Input aria-label={t('chat.worktreeStart')} disabled={busy} value={base} onChange={event=>setBase(event.target.value)}/></label>}
      <Button disabled={busy||(mode==='branch'&&!base.trim())} onClick={()=>void run({action:'start',mode,base})}>{t('chat.reviewStart')}</Button>
      <Button variant="secondary" disabled={busy} onClick={()=>void run({action:'get'})}>{t('chat.reviewRefresh')}</Button>
    </div>
    {saved.length>0&&<label className="block space-y-1">{t('chat.reviewSaved')}<select className={selectStyle} aria-label={t('chat.reviewSaved')} disabled={busy} value={review?.id??''} onChange={event=>void run({action:'select',id:event.target.value})}><option value="" disabled>—</option>{saved.map(item=><option key={item.id} value={item.id}>{modeLabel(item.mode)} · {item.baseRef} · {new Date(item.createdAt).toLocaleString()}</option>)}</select></label>}
    {review&&<>
      <div className="space-y-1 rounded border border-border p-2 text-text-secondary">
        <p className="break-all select-text">{review.workspaceRoot}</p>
        <p>{t('chat.reviewComparison')}: {modeLabel(review.mode)} · {review.baseRef}</p>
        <p className="break-all font-mono select-text">{t('chat.reviewRevision')}: {review.snapshot.revision}</p>
        <p className="break-all font-mono select-text">HEAD: {review.snapshot.headSha}</p>
        <p className="break-all font-mono select-text">{t('chat.reviewBaseline')}: {review.snapshot.baseline}</p>
        {review.snapshot.truncated&&<p className="text-warning">{t('chat.reviewPartial')}</p>}
      </div>
      <div className="flex flex-wrap gap-2"><Button disabled={busy} onClick={()=>feedback([])}>{t('chat.reviewAsk')}</Button><Button disabled={busy||!selected.length} onClick={()=>feedback(selected)}>{t('chat.reviewRepair')} ({selected.length})</Button></div>
      <div className="grid min-w-0 gap-4 md:grid-cols-2">
        <section className="min-w-0 space-y-2">
          <select aria-label={t('chat.reviewFile')} className={selectStyle} disabled={busy} value={path} onChange={event=>{setPath(event.target.value);setLine('1');}}><option value="" disabled>{t('chat.reviewFile')}</option>{review.snapshot.files.map(entry=><option key={entry.path} value={entry.path}>{entry.path}</option>)}</select>
          {file&&<div className="max-h-80 overflow-auto rounded border border-border bg-surface-1 font-mono leading-5" aria-label={t('chat.reviewDiff')}>
            {diffLines(file.patch).map((row,index)=><div key={index} className={`flex w-max min-w-full ${row.next!==null&&row.old===null?'bg-success/10':row.old!==null&&row.next===null?'bg-danger/10':''}`}>
              <button type="button" disabled={busy||file.truncated||file.binary||row.old===null} aria-label={`old ${row.old??''}`} className="w-10 shrink-0 text-text-tertiary hover:bg-surface-3 disabled:cursor-default" onClick={()=>{setSide('old');setLine(String(row.old));}}>{row.old}</button>
              <button type="button" disabled={busy||file.truncated||file.binary||row.next===null} aria-label={`new ${row.next??''}`} className="w-10 shrink-0 text-text-tertiary hover:bg-surface-3 disabled:cursor-default" onClick={()=>{setSide('new');setLine(String(row.next));}}>{row.next}</button>
              <span className="whitespace-pre px-2 select-text">{row.text}</span>
            </div>)}
          </div>}
          {file&&(file.truncated||file.binary)&&<p className="text-warning">{t('chat.reviewPartial')}</p>}
          {file&&!file.truncated&&!file.binary&&<div className="space-y-2 rounded border border-border p-2">
            <div className="flex gap-2"><select aria-label={t('chat.reviewSide')} disabled={busy} className={selectStyle} value={side} onChange={event=>setSide(event.target.value)}><option value="new">{t('chat.reviewNew')}</option><option value="old">{t('chat.reviewOld')}</option></select><Input aria-label={t('chat.reviewLine')} type="number" min="1" value={line} disabled={busy} onChange={event=>setLine(event.target.value)}/><select aria-label={t('chat.reviewPriority')} className={selectStyle} disabled={busy} value={priority} onChange={event=>setPriority(event.target.value)}>{[0,1,2,3].map(value=><option key={value} value={value}>P{value}</option>)}</select></div>
            <Input aria-label={t('chat.reviewFindingTitle')} placeholder={t('chat.reviewFindingTitle')} value={title} maxLength={240} disabled={busy} onChange={event=>setTitle(event.target.value)}/>
            <textarea aria-label={t('chat.reviewFindingBody')} placeholder={t('chat.reviewFindingBody')} className={`${selectStyle} min-h-20 resize-y`} value={body} maxLength={8000} disabled={busy} onChange={event=>setBody(event.target.value)}/>
            <Button disabled={busy||!title.trim()||!body.trim()||!Number.isSafeInteger(Number(line))||Number(line)<1} onClick={()=>void run({action:'add',review_id:review.id,finding:{anchor:{revision:review.snapshot.revision,path,side,line:Number(line)},priority:Number(priority),title,body}})}>{t('chat.reviewAdd')}</Button>
          </div>}
        </section>
        <section className="min-w-0 space-y-2" aria-label={t('chat.reviewFindings')}>
          <p className="font-medium">{t('chat.reviewFindings')} ({review.findings.length})</p>
          {review.findings.map(finding=><article key={finding.id} className="space-y-2 rounded border border-border p-3">
            <label className="flex items-start gap-2"><input type="checkbox" className="mt-0.5" aria-label={finding.title} disabled={busy||finding.stale||!['open','accepted'].includes(finding.status)} checked={selected.includes(finding.id)} onChange={event=>setSelected(ids=>event.target.checked?[...ids,finding.id]:ids.filter(id=>id!==finding.id))}/><span className="break-words font-medium">P{finding.priority} · {finding.title}</span></label>
            <p className="break-all text-text-tertiary">{finding.anchor.path}:{finding.anchor.line} ({finding.anchor.side}) · {statusLabel(finding.status)}{finding.stale&&<span className="text-warning"> · {t('chat.reviewStale')}</span>}</p>
            <p className="whitespace-pre-wrap break-words text-text-secondary">{finding.body}</p>
            <div className="flex flex-wrap gap-2"><Button variant="secondary" disabled={busy||finding.stale} onClick={()=>status(finding,'accepted')}>{t('chat.reviewAccept')}</Button><Button variant="secondary" disabled={busy} onClick={()=>status(finding,'resolved')}>{t('chat.reviewResolve')}</Button><Button variant="secondary" disabled={busy} onClick={()=>status(finding,'dismissed')}>{t('chat.reviewDismiss')}</Button></div>
          </article>)}
        </section>
      </div>
      <section className="space-y-2 border-t border-border pt-3">
        <p className="font-medium">GitHub PR</p><p className="text-text-secondary">{t('chat.reviewPrHelp')}</p>
        <div className="flex gap-2"><Input aria-label="GitHub PR URL" placeholder="https://github.com/owner/repo/pull/123" value={url} disabled={busy} onChange={event=>setUrl(event.target.value)}/><Button disabled={busy||!url.trim()} onClick={()=>void run({action:'attach_pr',review_id:review.id,url})}>{t('chat.reviewPrRead')}</Button></div>
        {review.pullRequest&&<div className="space-y-2 rounded border border-border p-3">
          <SafeLink url={review.pullRequest.url}>{review.pullRequest.title}</SafeLink><p className="break-all">{review.pullRequest.state}{review.pullRequest.draft?' · draft':''} · {review.pullRequest.reviewDecision??'—'}</p>
          <p className="break-all font-mono select-text">HEAD: {review.pullRequest.headSha}</p><p className="text-text-tertiary">{t('chat.reviewObserved')}: {new Date(review.pullRequest.observedAt).toLocaleString()}</p>
          {!review.pullRequest.matchesLocalHead&&<p className="text-warning">{t('chat.reviewHeadMismatch')}</p>}{review.pullRequest.partial&&<p className="text-warning">{t('chat.reviewPartial')}</p>}
          <details><summary className="cursor-pointer">{t('chat.reviewChecks')} ({review.pullRequest.checks.length})</summary><ul className="mt-2 space-y-1">{review.pullRequest.checks.map((check,index)=><li key={index} className="break-words"><SafeLink url={check.url}>{check.name}</SafeLink> · {check.state}</li>)}</ul></details>
          <details><summary className="cursor-pointer">{t('chat.reviewThreads')} ({review.pullRequest.unresolvedThreads.length})</summary><div className="mt-2 space-y-3">{review.pullRequest.unresolvedThreads.map(thread=><article key={thread.id}><SafeLink url={thread.url}>{thread.path}:{thread.line??'—'}</SafeLink>{thread.outdated&&<span className="text-warning"> · {t('chat.reviewStale')}</span>}<p className="whitespace-pre-wrap break-words text-text-secondary">{thread.body}</p></article>)}</div></details>
        </div>}
      </section>
    </>}
    {busy&&<p role="status">{t('common.loading')}</p>}
  </div>;
}
