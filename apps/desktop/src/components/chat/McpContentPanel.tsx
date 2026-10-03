import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from '../../i18n';
import { Button } from '../ui/Button';
import { Input } from '../ui/Input';
import { McpResultView } from './McpResultView';
import { extractMcpResult } from '../../lib/mcpResult';
import type { ArtifactPayload } from '../../types/conversation';

interface Server { id: string; name: string; enabled: boolean }
interface Catalog {
  authorityEpoch: number; complete: boolean; diagnostics: string | null;
  contentDiagnostics?: string[]; promptsComplete?: boolean;
  resources: Array<{ uri: string; name: string }>;
  resourceTemplates: Array<{ uriTemplate: string; name: string }>;
  prompts: Array<{ name: string; description?: string; arguments?: Array<{ name: string; description?: string; required?: boolean }> }>;
}
interface ContentResult { content: string; isError: boolean; artifacts: ArtifactPayload }

export function McpContentDisclosure({ serverId }: { serverId: string }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  return <details className="mx-4 mb-3 rounded border border-border p-2" onToggle={event => setOpen(event.currentTarget.open)}>
    <summary className="cursor-pointer text-xs text-text-secondary">{t('chat.mcpContentTitle')}</summary>
    {open && <div className="mt-3"><McpContentPanel serverId={serverId} /></div>}
  </details>;
}

export function McpContentPanel({ serverId: initialServerId, onInsert }: { serverId?: string; onInsert?: (text: string) => void }) {
  const { t } = useTranslation();
  const [servers, setServers] = useState<Server[]>([]);
  const [serverId, setServerId] = useState(initialServerId ?? '');
  const [catalog, setCatalog] = useState<Catalog | null>(null);
  const [mode, setMode] = useState<'resource' | 'prompt'>('resource');
  const [uri, setUri] = useState('');
  const [uriTemplate, setUriTemplate] = useState('');
  const [prompt, setPrompt] = useState('');
  const [args, setArgs] = useState<Record<string, string>>({});
  const [result, setResult] = useState<ContentResult | null>(null);
  const [source, setSource] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const sequence = useRef(0);
  useEffect(() => {
    let active = true;
    invoke<Server[]>('list_mcp_servers_cmd').then(items => { if (active) setServers(items.filter(item => item.enabled)); }).catch(cause => { if (active) setError(String(cause)); });
    return () => { active = false; sequence.current += 1; };
  }, []);
  const load = useCallback(async () => {
    const request = ++sequence.current;
    setCatalog(null); setResult(null); setUri(''); setUriTemplate(''); setPrompt(''); setArgs({}); setError(null);
    if (!serverId) { setBusy(false); return; }
    setBusy(true);
    try {
      const next = await invoke<Catalog>('get_mcp_content_catalog_cmd', { serverId });
      if (request === sequence.current) setCatalog(next);
    } catch (cause) { if (request === sequence.current) setError(String(cause)); }
    finally { if (request === sequence.current) setBusy(false); }
  }, [serverId]);
  useEffect(() => { void load(); return () => { sequence.current += 1; }; }, [load]);
  const selectedPrompt = catalog?.prompts.find(item => item.name === prompt);
  const templateVariables = [...new Set([...uriTemplate.matchAll(/\{([^{}]+)\}/g)].flatMap(match =>
    match[1].replace(/^[+#./;?&]/, '').split(',').map(variable => variable.replace(/(?::\d+|\*)$/, '')),
  ))];
  const read = async () => {
    if (!catalog) return;
    const request = ++sequence.current; setBusy(true); setError(null); setResult(null);
    const target = mode === 'resource' ? uriTemplate || uri : prompt;
    try {
      const next = await invoke<ContentResult>('read_mcp_content_cmd', { serverId, authorityEpoch: catalog.authorityEpoch,
        request: mode === 'resource'
          ? uriTemplate ? { action:'read_resource_template',uri_template:uriTemplate,arguments:args } : { action:'read_resource',uri }
          : { action:'get_prompt',name:prompt,arguments:args },
      });
      if (request === sequence.current) {
        setResult(next); setSource(`MCP · ${servers.find(item => item.id === serverId)?.name ?? serverId} · ${target}`);
      }
    } catch (cause) { if (request === sequence.current) setError(String(cause)); }
    finally { if (request === sequence.current) setBusy(false); }
  };
  const view = result ? extractMcpResult(result.artifacts) : null;
  const hasText = view ? view.contentBlocks.some(block => block.type === 'text' || (block.type === 'resource' && typeof block.resource.text === 'string')) : !!result?.content;
  const selectClass = 'w-full rounded-md border border-border bg-surface-2 p-2 text-xs';
  return <section className="space-y-3" data-testid="mcp-content-panel">
    <p className="text-xs leading-relaxed text-text-secondary">{t('chat.mcpContentHint')}</p>
    {!initialServerId && <select disabled={busy} aria-label={t('chat.mcpConnector')} className={selectClass} value={serverId} onChange={event => setServerId(event.target.value)}>
      <option value="">{t('chat.mcpChooseConnector')}</option>{servers.map(server => <option key={server.id} value={server.id}>{server.name}</option>)}
    </select>}
    <Button size="sm" variant="ghost" disabled={!serverId || busy} onClick={() => void load()}>{t('chat.mcpRefreshCatalog')}</Button>
    {error && <p role="alert" className="break-words text-xs text-danger">{error}</p>}
    {catalog?.diagnostics && <p role="status" className="text-xs text-warning">{catalog.diagnostics}</p>}
    {catalog?.contentDiagnostics?.map(diagnostic => <p key={diagnostic} role="status" className="text-xs text-warning">{diagnostic}</p>)}
    {catalog && <>
      <select disabled={busy} aria-label={t('chat.mcpContentKind')} className={selectClass} value={mode} onChange={event => { setMode(event.target.value as typeof mode); setArgs({}); setResult(null); }}>
        <option value="resource">{t('chat.mcpResources')}</option><option value="prompt">{t('chat.mcpPrompts')}</option>
      </select>
      {mode === 'resource' ? <>
        <select disabled={busy} aria-label={t('chat.mcpResources')} className={selectClass} value={uriTemplate || (catalog.resources.some(item => item.uri === uri) ? uri : '')} onChange={event => {
          const target = event.target.value;
          const isTemplate = catalog.resourceTemplates.some(item => item.uriTemplate === target);
          setUriTemplate(isTemplate ? target : ''); setUri(isTemplate ? '' : target); setArgs({}); setResult(null);
        }}>
          <option value="">{t('chat.mcpChooseResource')}</option>
          {catalog.resources.map(item => <option key={item.uri} value={item.uri}>{item.name} · {item.uri}</option>)}
          {catalog.resourceTemplates.map(item => <option key={item.uriTemplate} value={item.uriTemplate}>{item.name} · {item.uriTemplate}</option>)}
        </select>
        <Input disabled={busy || !!uriTemplate} aria-label={t('chat.mcpResourceUri')} placeholder={t('chat.mcpResourceUri')} value={uriTemplate || uri} onChange={event => { setUri(event.target.value); setResult(null); }} />
        {templateVariables.map(variable => <label key={variable} className="block text-xs">{variable}
          <Input disabled={busy} value={args[variable] ?? ''} onChange={event => { setArgs(values => ({ ...values, [variable]: event.target.value })); setResult(null); }} />
        </label>)}
      </> : <>
        <select disabled={busy} aria-label={t('chat.mcpPrompts')} className={selectClass} value={prompt} onChange={event => { setPrompt(event.target.value); setArgs({}); setResult(null); }}>
          <option value="">{t('chat.mcpChoosePrompt')}</option>{catalog.prompts.map(item => <option key={item.name} value={item.name}>{item.name}</option>)}
        </select>
        {selectedPrompt?.description && <p className="text-xs text-text-tertiary">{selectedPrompt.description}</p>}
        {selectedPrompt?.arguments?.map(argument => <label key={argument.name} className="block text-xs">{argument.name}{argument.required ? ' *' : ''}
          <Input disabled={busy} value={args[argument.name] ?? ''} required={argument.required} title={argument.description} onChange={event => { setArgs(values => ({ ...values,[argument.name]:event.target.value })); setResult(null); }} />
        </label>)}
      </>}
      <Button size="sm" loading={busy} disabled={busy || !catalog.complete || (mode === 'resource' ? !uri && !uriTemplate : !prompt || selectedPrompt?.arguments?.some(argument => argument.required && !args[argument.name]))} onClick={() => void read()}>{t('chat.mcpReadContent')}</Button>
    </>}
    {result && <div className="space-y-3 rounded-lg border border-border p-3">
      <p className="break-all text-xs text-text-tertiary">{source}</p>
      {view ? <McpResultView result={view} /> : <pre className="max-h-72 overflow-auto whitespace-pre-wrap break-words text-xs">{result.content}</pre>}
      {onInsert && <Button size="sm" disabled={result.isError || !hasText || busy} onClick={() => onInsert(`${source}\n${result.content}`)}>{t('chat.mcpInsertText')}</Button>}
    </div>}
  </section>;
}
