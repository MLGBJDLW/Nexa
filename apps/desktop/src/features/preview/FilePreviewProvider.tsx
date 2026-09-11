import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
} from 'react';
import { useLocation, useNavigate } from 'react-router';
import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { open as openExternal } from '@tauri-apps/plugin-shell';
import { AnimatePresence, motion, useReducedMotion } from 'framer-motion';
import { toast } from 'sonner';
import {
  BotMessageSquare,
  Check,
  Copy,
  ExternalLink,
  Eye,
  FileCode2,
  FileSpreadsheet,
  FileText,
  FolderOpen,
  Image as ImageIcon,
  Languages,
  ListTree,
  Loader2,
  Minus,
  PanelRightClose,
  Plus,
  RotateCcw,
  Save,
  Scissors,
  SplitSquareHorizontal,
  Sparkles,
  SquarePen,
  TextCursorInput,
  TriangleAlert,
  X,
} from 'lucide-react';
import { useTranslation } from '../../i18n';
import * as api from '../../lib/api';
import { isWebUrl } from '../../lib/sourceDisplay';
import { useResizablePanel } from '../../lib/useResizablePanel';
import { openNexaBrowser } from '../browser';
import { requestNexaBrowser } from '../browser/openNexaBrowser';
import { FilePreviewContext } from './filePreviewContext';
import { useAgentPreviewRequests, type AgentPreviewRequest } from './useAgentPreviewRequests';

import { createPreviewLabels, TextPreview, MarkdownPreview, StructuredPreviewRenderer, type PreviewLabels } from './StructuredPreview';

type PreviewMode = 'preview' | 'text' | 'edit' | 'split';

const isMediaPreview = (preview: api.FilePreview) => ['image', 'audio', 'video'].includes(preview.kind);

function MediaPreview({ preview, ready, failed }: { preview: api.FilePreview; ready: () => void; failed: () => void }) {
  const source = `${convertFileSrc(preview.path)}?preview=${encodeURIComponent(preview.hash)}`;
  return <div className="flex h-full min-h-60 items-center justify-center bg-surface-0 p-4" data-testid="file-preview-media">
    {preview.kind === 'image' ? <img src={source} alt={preview.displayName} className="max-h-full max-w-full object-contain" onLoad={ready} onError={failed} />
      : preview.kind === 'audio' ? <audio src={source} controls preload="metadata" className="w-full" onLoadedMetadata={ready} onError={failed} />
        : <video src={source} controls playsInline preload="metadata" className="max-h-full max-w-full" onLoadedMetadata={ready} onError={failed} />}
  </div>;
}

const INSTANT_TRANSITION = { duration: 0 };
const FILE_PREVIEW_WIDTH_KEY = 'file-preview-panel-width';
const FILE_PREVIEW_MIN_WIDTH = 560;
const FILE_PREVIEW_MAX_WIDTH = 1180;
const MAX_AGENT_SELECTION_CHARS = 24_000;

type TextSelectionState = {
  start: number;
  end: number;
  origin: 'editor' | 'preview';
};

type TextSelectionSummary = TextSelectionState & {
  text: string;
  startLine: number;
  endLine: number;
  charCount: number;
  lineCount: number;
};

function basename(path: string): string {
  const normalized = path.replace(/[\\/]+$/, '');
  const lastSep = Math.max(normalized.lastIndexOf('/'), normalized.lastIndexOf('\\'));
  return lastSep === -1 ? normalized : normalized.slice(lastSep + 1);
}

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return '';
  if (bytes < 1024) return `${bytes} B`;
  const units = ['KB', 'MB', 'GB'];
  let value = bytes / 1024;
  for (const unit of units) {
    if (value < 1024 || unit === 'GB') {
      return `${value.toFixed(value < 10 ? 1 : 0)} ${unit}`;
    }
    value /= 1024;
  }
  return `${bytes} B`;
}

function formatTimestamp(value: string | null | undefined, locale: string): string {
  if (!value) return '';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return '';
  return new Intl.DateTimeFormat(locale, {
    month: 'short',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  }).format(date);
}

function lineNumberAt(content: string, index: number): number {
  const safeIndex = Math.max(0, Math.min(index, content.length));
  let line = 1;
  for (let i = 0; i < safeIndex; i += 1) {
    if (content.charCodeAt(i) === 10) line += 1;
  }
  return line;
}

function getSelectionSummary(
  content: string,
  selection: TextSelectionState | null,
): TextSelectionSummary | null {
  if (!selection) return null;
  const start = Math.max(0, Math.min(selection.start, content.length));
  const end = Math.max(0, Math.min(selection.end, content.length));
  if (end <= start) return null;

  const text = content.slice(start, end);
  if (!text.trim()) return null;

  const startLine = lineNumberAt(content, start);
  const endLine = lineNumberAt(content, Math.max(start, end - 1));

  return {
    ...selection,
    start,
    end,
    text,
    startLine,
    endLine,
    charCount: text.length,
    lineCount: endLine - startLine + 1,
  };
}

function codeFenceFor(text: string): string {
  let fence = '```';
  while (text.includes(fence)) {
    fence += '`';
  }
  return fence;
}

function normalizeRenderedSelection(text: string): string {
  return text.replace(/\r\n?/g, '\n');
}

function isOfficeDocumentPreview(preview: api.FilePreview): boolean {
  return ['.docx', '.pptx', '.xlsx'].includes(preview.extension.toLowerCase());
}

function hasStructuredPreview(preview: api.FilePreview | null): boolean {
  return Boolean(preview?.structuredPreview);
}

function defaultModeForPreview(preview: api.FilePreview): PreviewMode {
  if (preview.structuredPreview) return 'preview';
  if (preview.editable) return 'edit';
  return 'preview';
}

type TranslateFn = ReturnType<typeof useTranslation>['t'];

function buildAgentEditPrompt({
  t,
  preview,
  selection,
  instruction,
}: {
  t: TranslateFn;
  preview: api.FilePreview;
  selection: TextSelectionSummary;
  instruction: string;
}): string {
  const fallbackInstruction = t('preview.defaultAgentInstruction');
  const finalInstruction = instruction.trim() || fallbackInstruction;
  const lineRange =
    selection.startLine === selection.endLine
      ? `${selection.startLine}`
      : `${selection.startLine}-${selection.endLine}`;
  const fence = codeFenceFor(selection.text);
  const officeDocument = isOfficeDocumentPreview(preview);
  const promptKey = officeDocument
    ? 'preview.agentPromptOffice'
    : preview.editable
      ? 'preview.agentPromptEditable'
      : 'preview.agentPromptReadOnly';

  return t(promptKey as Parameters<TranslateFn>[0], {
    path: preview.path,
    displayName: preview.displayName,
    sourceName: preview.sourceName,
    lineRange,
    start: selection.start,
    end: selection.end,
    hash: preview.hash,
    instruction: finalInstruction,
    fence,
    text: selection.text,
  });
}

function OfficeRenderedPreview({
  rendered,
  labels,
}: {
  rendered: api.RenderedPreview;
  labels: PreviewLabels;
}) {
  const [zoom, setZoom] = useState(1);

  useEffect(() => {
    setZoom(1);
  }, [rendered]);

  const zoomPercent = Math.round(zoom * 100);
  const pageWidth = Math.round(900 * zoom);
  const pageSummary = rendered.truncated
    ? `${rendered.pageCount}+ ${labels.pages}`
    : `${rendered.pageCount} ${labels.pages}`;

  return (
    <div data-testid="file-preview-rendered-content" className="flex h-full min-h-0 flex-col bg-surface-0">
      <div className="shrink-0 border-b border-border bg-surface-1/95 px-4 py-2 backdrop-blur">
        <div className="flex items-center gap-2">
          <div className="flex min-w-0 items-center gap-2 text-xs text-text-tertiary">
            <ImageIcon size={14} className="text-accent" />
            <span className="whitespace-nowrap">{pageSummary}</span>
            <span className="hidden whitespace-nowrap sm:inline">{rendered.dpi} DPI</span>
          </div>
          <div className="flex-1" />
          <div className="flex items-center rounded-md border border-border bg-surface-2 p-0.5">
            <button
              type="button"
              onClick={() => setZoom((value) => Math.max(0.5, Number((value - 0.1).toFixed(2))))}
              className="inline-flex h-7 w-7 items-center justify-center rounded text-text-secondary transition-colors hover:bg-surface-3 hover:text-text-primary"
              title={labels.zoomOut}
              aria-label={labels.zoomOut}
            >
              <Minus size={14} />
            </button>
            <button
              type="button"
              onClick={() => setZoom(1)}
              className="h-7 min-w-12 rounded px-2 text-[11px] font-medium text-text-secondary transition-colors hover:bg-surface-3 hover:text-text-primary"
              title={labels.resetZoom}
              aria-label={labels.resetZoom}
            >
              {zoomPercent}%
            </button>
            <button
              type="button"
              onClick={() => setZoom((value) => Math.min(1.8, Number((value + 0.1).toFixed(2))))}
              className="inline-flex h-7 w-7 items-center justify-center rounded text-text-secondary transition-colors hover:bg-surface-3 hover:text-text-primary"
              title={labels.zoomIn}
              aria-label={labels.zoomIn}
            >
              <Plus size={14} />
            </button>
          </div>
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-auto px-4 py-5">
        <div className="mx-auto flex max-w-full flex-col items-center gap-5">
          {rendered.pages.map((page) => (
            <figure
              key={`${page.page}-${page.path}`}
              data-testid="file-preview-rendered-page"
              className="m-0"
              style={{
                width: pageWidth,
                maxWidth: zoom <= 1 ? '100%' : undefined,
              }}
            >
              <figcaption className="mb-1 flex items-center justify-between px-1 text-[11px] text-text-tertiary">
                <span>
                  {labels.page} {page.page}
                </span>
              </figcaption>
              <div className="overflow-hidden rounded-md border border-border/70 bg-white shadow-[0_18px_45px_rgba(0,0,0,0.28)]">
                <img
                  src={convertFileSrc(page.path)}
                  alt={`${labels.page} ${page.page}`}
                  draggable={false}
                  className="block w-full select-none"
                />
              </div>
            </figure>
          ))}
        </div>
      </div>
    </div>
  );
}

function ModeButton({
  active,
  icon,
  label,
  onClick,
}: {
  active: boolean;
  icon: ReactNode;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`inline-flex h-8 items-center gap-1.5 rounded-md px-2.5 text-xs font-medium transition-colors ${
        active
          ? 'bg-accent text-white'
          : 'text-text-secondary hover:bg-surface-3 hover:text-text-primary'
      }`}
    >
      {icon}
      <span>{label}</span>
    </button>
  );
}

export function FilePreviewProvider({ children }: { children: ReactNode }) {
  const { locale, t } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const labels = useMemo(() => createPreviewLabels(t), [t]);
  const shouldReduceMotion = useReducedMotion();
  const [open, setOpen] = useState(false);
  const [activePath, setActivePath] = useState<string | null>(null);
  const [preview, setPreview] = useState<api.FilePreview | null>(null);
  const [draft, setDraft] = useState('');
  const [textSelection, setTextSelection] = useState<TextSelectionState | null>(null);
  const [agentInstruction, setAgentInstruction] = useState('');
  const [copiedAgentRequest, setCopiedAgentRequest] = useState(false);
  const [mode, setMode] = useState<PreviewMode>('preview');
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copiedPath, setCopiedPath] = useState(false);
  const {
    size: previewPanelWidth,
    setSize: setPreviewPanelWidth,
    startResize: startPreviewPanelResize,
    isResizing: isPreviewPanelResizing,
  } = useResizablePanel({
    storageKey: FILE_PREVIEW_WIDTH_KEY,
    defaultSize: 860,
    minSize: FILE_PREVIEW_MIN_WIDTH,
    maxSize: FILE_PREVIEW_MAX_WIDTH,
    direction: -1,
  });
  const dirty = Boolean(preview?.editable && draft !== (preview.content ?? ''));
  const dirtyRef = useRef(false);
  const loadGeneration = useRef(0);
  const agentPreviewRequest = useRef<string | null>(null);
  const htmlRequest = useRef<AbortController | null>(null);
  const textPreview = useRef<HTMLTextAreaElement>(null);
  const mediaReady = useRef<{ path: string; resolve: () => void; reject: (error: Error) => void } | null>(null);

  useEffect(() => {
    dirtyRef.current = dirty;
  }, [dirty]);

  useEffect(() => () => htmlRequest.current?.abort(), []);
  const openHtml = useCallback(async (path: string, conversationId?: string | null, resourcePaths: string[] = []) => {
    htmlRequest.current?.abort();
    const controller = new AbortController(); htmlRequest.current = controller;
    const currentChat = /^\/chat\/([^/]+)$/.exec(location.pathname)?.[1];
    const owner = conversationId ?? currentChat ?? 'nexa-global-browser-workspace';
    const prepared = await invoke<{ previewId: string; path: string; url: string; reused: boolean }>('prepare_html_preview_cmd', { path, conversationId: owner, resourcePaths });
    try {
      if (controller.signal.aborted) throw new Error('The HTML preview was cancelled.');
      const opened = requestNexaBrowser(prepared.url, owner, controller.signal);
      if (conversationId && currentChat !== conversationId) navigate(`/chat/${encodeURIComponent(conversationId)}`);
      else if (!currentChat && location.pathname.startsWith('/chat')) navigate('/');
      setOpen(false);
      await opened;
      return { path: prepared.path, kind: 'html', displayMode: 'browser', warning: null };
    } catch (error) {
      if (!prepared.reused) await invoke('release_html_preview_cmd', { previewId: prepared.previewId }).catch(() => {});
      throw error;
    } finally { if (htmlRequest.current === controller) htmlRequest.current = null; }
  }, [location.pathname, navigate]);

  const loadFile = useCallback(
    async (
      path: string,
      options: { preferredMode?: PreviewMode } = {},
    ) => {
      const generation = ++loadGeneration.current;
      setLoading(true);
      setError(null);
      setActivePath(path);
      try {
        const next = await api.previewFile(path);
        if (generation !== loadGeneration.current) return null;
        setPreview(next);
        setDraft(next.content ?? '');
        setTextSelection(null);
        setAgentInstruction('');
        setCopiedAgentRequest(false);
        setMode(options.preferredMode ?? defaultModeForPreview(next));
        setActivePath(next.path);
        return next;
      } catch (err) {
        if (generation !== loadGeneration.current) return null;
        const message = err instanceof Error ? err.message : String(err);
        setPreview(null);
        setDraft('');
        setTextSelection(null);
        setAgentInstruction('');
        setError(message);
        toast.error(`${labels.loadFailed}: ${message}`);
        return null;
      } finally {
        if (generation === loadGeneration.current) setLoading(false);
      }
    },
    [labels.loadFailed],
  );

  useAgentPreviewRequests(async (request: AgentPreviewRequest) => {
    if (dirtyRef.current) throw new Error('The current preview has unsaved edits. Save or discard them before opening another file.');
    agentPreviewRequest.current = request.requestId;
    if (/\.html?$/i.test(request.path) && !request.line) {
      const receipt = await openHtml(request.path, request.conversationId, request.resourcePaths);
      if (agentPreviewRequest.current !== request.requestId) throw new Error('The preview request was cancelled.');
      agentPreviewRequest.current = null;
      return receipt;
    }
    setOpen(true);
    const next = await loadFile(request.path, { preferredMode: request.line ? 'text' : 'preview' });
    if (!next || agentPreviewRequest.current !== request.requestId) throw new Error('The preview failed or was replaced by another request.');
    if (next.content == null && !isMediaPreview(next) && !next.structuredPreview && !next.renderedPreview?.pages.length) throw new Error('Nexa cannot preview this file. No external application was opened.');
    if (isMediaPreview(next)) await new Promise<void>((resolve, reject) => { mediaReady.current = { path: next.path, resolve, reject }; });
    await new Promise<void>(resolve => {
      let frame = 0;
      const finish = () => { cancelAnimationFrame(frame); clearTimeout(timer); resolve(); };
      const timer = setTimeout(finish, 150);
      frame = requestAnimationFrame(finish);
    });
    if (agentPreviewRequest.current !== request.requestId) throw new Error('The preview request was cancelled.');
    if (request.line && next.content != null) {
      const deadline = performance.now() + 5_000;
      while (!textPreview.current || textPreview.current.value !== next.content) {
        if (agentPreviewRequest.current !== request.requestId || performance.now() > deadline) throw new Error('The text preview did not become ready.');
        await new Promise(resolve => setTimeout(resolve, 25));
      }
      const lines = next.content.split('\n');
      const line = Math.min(request.line, lines.length);
      const start = lines.slice(0, line - 1).reduce((sum, text) => sum + text.length + 1, 0);
      setTextSelection({ start, end: start + (lines[line - 1]?.length ?? 0), origin: 'preview' });
      if (textPreview.current) {
        textPreview.current.focus(); textPreview.current.setSelectionRange(start, start + (lines[line - 1]?.length ?? 0));
        textPreview.current.scrollTop = Math.max(0, line - 3) * Number.parseFloat(getComputedStyle(textPreview.current).lineHeight || '20');
      }
    }
    agentPreviewRequest.current = null;
    return { path: next.path, kind: next.kind, displayMode: request.line ? 'text' : next.structuredPreview || next.renderedPreview ? 'structured' : 'preview', warning: next.warning ?? null };
  }, requestId => {
    if (agentPreviewRequest.current !== requestId) return;
    agentPreviewRequest.current = null; loadGeneration.current++; setLoading(false);
    htmlRequest.current?.abort();
    mediaReady.current?.reject(new Error('The media preview was cancelled.')); mediaReady.current = null;
    if (!dirtyRef.current) setOpen(false);
  });

  const openFilePreview = useCallback((path: string) => {
    if (dirtyRef.current && !window.confirm(labels.discardPrompt)) {
      return;
    }
    agentPreviewRequest.current = null;
    htmlRequest.current?.abort();
    mediaReady.current?.reject(new Error('The media preview was replaced.')); mediaReady.current = null;
    if (/\.html?$/i.test(path)) { void openHtml(path).catch(error => toast.error(`${labels.loadFailed}: ${String(error)}`)); }
    else { setOpen(true); void loadFile(path); }
  }, [labels.discardPrompt, labels.loadFailed, loadFile, openHtml]);

  const openWebLink = useCallback((url: string, title?: string) => {
    const trimmed = url.trim();
    if (!isWebUrl(trimmed)) {
      void openExternal(trimmed).catch((reason) => {
        console.error('[link] external open failed', reason);
        toast.error(labels.browserOpenFailed);
      });
      return;
    }
    if (dirtyRef.current && !window.confirm(labels.discardPrompt)) {
      return;
    }
    if (!openNexaBrowser(trimmed, title)) {
      toast.error(labels.browserOpenFailed);
      return;
    }
    setOpen(false);
  }, [labels.browserOpenFailed, labels.discardPrompt]);

  const close = useCallback(() => {
    if (dirty && !window.confirm(labels.discardPrompt)) {
      return;
    }
    loadGeneration.current++;
    agentPreviewRequest.current = null;
    mediaReady.current?.reject(new Error('The media preview was closed.')); mediaReady.current = null;
    setLoading(false);
    setOpen(false);
  }, [dirty, labels.discardPrompt]);


  const handlePreviewPanelResizeKey = useCallback((event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'ArrowLeft') {
      event.preventDefault();
      setPreviewPanelWidth(previewPanelWidth + 16);
    } else if (event.key === 'ArrowRight') {
      event.preventDefault();
      setPreviewPanelWidth(previewPanelWidth - 16);
    } else if (event.key === 'Home') {
      event.preventDefault();
      setPreviewPanelWidth(FILE_PREVIEW_MIN_WIDTH);
    } else if (event.key === 'End') {
      event.preventDefault();
      setPreviewPanelWidth(FILE_PREVIEW_MAX_WIDTH);
    }
  }, [previewPanelWidth, setPreviewPanelWidth]);

  const save = useCallback(async () => {
    if (!preview?.editable || !dirty) return;
    setSaving(true);
    setError(null);
    try {
      const result = await api.saveTextFile(preview.path, draft, preview.hash);
      setPreview(result.preview);
      setDraft(result.preview.content ?? '');
      toast.success(labels.saved);
      if (result.reindexStatus === 'error') {
        toast.warning(`${labels.reindexFailed}: ${result.reindexDetail ?? ''}`);
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
      toast.error(`${labels.saveFailed}: ${message}`);
    } finally {
      setSaving(false);
    }
  }, [dirty, draft, labels.reindexFailed, labels.saveFailed, labels.saved, preview]);

  const selectedText = useMemo(
    () => getSelectionSummary(draft, textSelection),
    [draft, textSelection],
  );

  const quickActions = useMemo(
    () => [
      {
        id: 'rewrite',
        label: labels.quickRewrite,
        instruction: t('preview.quickRewriteInstruction'),
        icon: <Sparkles size={13} />,
      },
      {
        id: 'shorten',
        label: labels.quickShorten,
        instruction: t('preview.quickShortenInstruction'),
        icon: <Scissors size={13} />,
      },
      {
        id: 'fix',
        label: labels.quickFix,
        instruction: t('preview.quickFixInstruction'),
        icon: <TextCursorInput size={13} />,
      },
      {
        id: 'translate-zh',
        label: labels.quickTranslateZh,
        instruction: t('preview.quickTranslateZhInstruction'),
        icon: <Languages size={13} />,
      },
    ],
    [labels.quickFix, labels.quickRewrite, labels.quickShorten, labels.quickTranslateZh, t],
  );

  const updateSelectionFromEditor = useCallback((target: HTMLTextAreaElement) => {
    const start = Math.min(target.selectionStart, target.selectionEnd);
    const end = Math.max(target.selectionStart, target.selectionEnd);
    if (end <= start) {
      setTextSelection(null);
      setCopiedAgentRequest(false);
      return;
    }
    setTextSelection({ start, end, origin: 'editor' });
    setCopiedAgentRequest(false);
  }, []);

  const captureRenderedSelection = useCallback(() => {
    if (!preview || !draft) return;
    const raw = window.getSelection()?.toString() ?? '';
    const selected = normalizeRenderedSelection(raw);
    if (!selected.trim()) return;

    const start = draft.indexOf(selected);
    if (start < 0) {
      setTextSelection(null);
      setCopiedAgentRequest(false);
      toast.info(labels.selectionMapFailed);
      return;
    }

    setTextSelection({ start, end: start + selected.length, origin: 'preview' });
    setCopiedAgentRequest(false);
  }, [draft, labels.selectionMapFailed, preview]);

  const updateDraft = useCallback((value: string) => {
    setDraft(value);
    setTextSelection(null);
    setCopiedAgentRequest(false);
  }, []);

  const buildCurrentAgentPrompt = useCallback(() => {
    if (!preview || !selectedText) return '';
    return buildAgentEditPrompt({
      t,
      preview,
      selection: selectedText,
      instruction: agentInstruction,
    });
  }, [agentInstruction, preview, selectedText, t]);

  const copyAgentRequest = useCallback(async () => {
    const prompt = buildCurrentAgentPrompt();
    if (!prompt) return;
    await navigator.clipboard.writeText(prompt);
    setCopiedAgentRequest(true);
    setTimeout(() => setCopiedAgentRequest(false), 1600);
    toast.success(labels.requestCopied);
  }, [buildCurrentAgentPrompt, labels.requestCopied]);

  const sendSelectionToAgent = useCallback(() => {
    if (!preview || !selectedText || dirty || !(preview.agentEditAllowed ?? Boolean(preview.sourceId))) return;
    const prompt = buildCurrentAgentPrompt();
    if (!prompt) return;
    navigate('/chat', {
      state: {
        initialMessage: prompt,
        sourceIds: preview.sourceId ? [preview.sourceId] : [],
      },
    });
    setOpen(false);
    toast.success(labels.agentRequestSent);
  }, [buildCurrentAgentPrompt, dirty, labels.agentRequestSent, navigate, preview, selectedText]);

  useEffect(() => {
    if (!textSelection) return;
    if (textSelection.start >= draft.length || textSelection.end > draft.length) {
      setTextSelection(null);
    }
  }, [draft.length, textSelection]);

  useEffect(() => {
    if (!open) return;
    const handler = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 's') {
        event.preventDefault();
        void save();
      }
      if (event.key === 'Escape') {
        close();
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [close, open, save]);

  const contextValue = useMemo(
    () => ({ openFilePreview, openWebLink }),
    [openFilePreview, openWebLink],
  );
  const content = preview?.content ?? '';
  const hasStructured = hasStructuredPreview(preview);
  const hasRenderedPreview = Boolean(preview?.renderedPreview?.pages?.length);
  const canShowPreview = Boolean(hasStructured || hasRenderedPreview || preview?.content != null || (preview && isMediaPreview(preview)));
  const supportsSplitPreview = Boolean(
    preview?.editable && preview.kind === 'markdown',
  );
  const metadataBits = preview
    ? [
        formatBytes(preview.sizeBytes),
        preview.renderedPreview?.pageCount
          ? `${preview.renderedPreview.pageCount}${preview.renderedPreview.truncated ? '+' : ''} ${labels.pages}`
          : '',
        preview.lineCount > 0 ? `${preview.lineCount} ${labels.lines}` : '',
        formatTimestamp(preview.modifiedAt, locale),
        preview.encoding ? `${labels.encoding}: ${preview.encoding}` : '',
      ].filter(Boolean)
    : [];

  return (
    <FilePreviewContext.Provider value={contextValue}>
      {children}
      <AnimatePresence>
        {open && (
          <>
            <motion.div
              key="file-preview-backdrop"
              initial={shouldReduceMotion ? false : { opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              transition={shouldReduceMotion ? INSTANT_TRANSITION : { duration: 0.15 }}
              data-testid="file-preview-backdrop"
              className="fixed inset-0 z-50 bg-black/35 backdrop-blur-[1px]"
              onClick={close}
              aria-hidden="true"
            />
            <motion.aside
              key="file-preview-panel"
              initial={shouldReduceMotion ? false : { x: '100%', opacity: 0.8 }}
              animate={{ x: 0, opacity: 1 }}
              exit={shouldReduceMotion ? { opacity: 0 } : { x: '100%', opacity: 0.8 }}
              transition={shouldReduceMotion || isPreviewPanelResizing ? INSTANT_TRANSITION : { duration: 0.24, ease: [0.16, 1, 0.3, 1] }}
              className="fixed inset-y-0 right-0 z-[51] flex max-w-full flex-col border-l border-border bg-surface-1 shadow-2xl"
              style={{ width: previewPanelWidth }}
              role="dialog"
              aria-modal="true"
              aria-label={labels.title}
            >
            <div
              role="separator"
              aria-orientation="vertical"
              aria-valuemin={FILE_PREVIEW_MIN_WIDTH}
              aria-valuemax={FILE_PREVIEW_MAX_WIDTH}
              aria-valuenow={previewPanelWidth}
              tabIndex={0}
              onPointerDown={startPreviewPanelResize}
              onKeyDown={handlePreviewPanelResizeKey}
              className="absolute left-0 top-0 h-full w-2 -translate-x-1 cursor-col-resize touch-none
                bg-transparent outline-none transition-colors hover:bg-accent/25 focus-visible:bg-accent/35"
              title={labels.resizePanel}
            />
            <header className="shrink-0 border-b border-border bg-surface-1/95 px-4 py-3 backdrop-blur">
              <div className="flex items-start gap-3">
                <div className="mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-md border border-border bg-surface-2 text-accent">
                  {preview?.kind === 'code' ? <FileCode2 size={18} /> : <FileText size={18} />}
                </div>
                <div className="min-w-0 flex-1">
                  <div className="flex min-w-0 items-center gap-2">
                    <h2 className="truncate text-sm font-semibold text-text-primary">
                      {preview?.displayName ?? basename(activePath ?? labels.title)}
                    </h2>
                    {dirty && (
                      <span className="shrink-0 rounded-full border border-warning/30 bg-warning/10 px-2 py-0.5 text-[10px] font-medium text-warning">
                        {labels.dirty}
                      </span>
                    )}
                    {preview && (
                      <span className={`shrink-0 rounded-full border px-2 py-0.5 text-[10px] font-medium ${
                        preview.editable
                          ? 'border-success/20 bg-success/10 text-success'
                          : 'border-border bg-surface-2 text-text-tertiary'
                      }`}>
                        {preview.editable ? labels.editable : labels.readOnly}
                      </span>
                    )}
                  </div>
                  <p className="mt-1 truncate text-[11px] text-text-tertiary" title={preview?.path ?? activePath ?? ''}>
                    {preview?.path ?? activePath}
                  </p>
                  {preview && (
                    <p className="mt-1 truncate text-[11px] text-text-tertiary">
                      {labels.source}: {preview.sourceName}
                      {metadataBits.length > 0 ? ` · ${metadataBits.join(' · ')}` : ''}
                    </p>
                  )}
                </div>
                <button
                  type="button"
                  onClick={close}
                  className="rounded-md p-2 text-text-tertiary transition-colors hover:bg-surface-2 hover:text-text-primary"
                  title={labels.close}
                  aria-label={labels.close}
                >
                  <PanelRightClose size={18} />
                </button>
              </div>

              <div className="mt-3 flex flex-wrap items-center gap-2">
                <div className="flex rounded-md border border-border bg-surface-2 p-0.5">
                  <ModeButton
                    active={mode === 'preview'}
                    icon={
                      preview?.structuredPreview?.type === 'workbook'
                        ? <FileSpreadsheet size={14} />
                        : hasStructured
                          ? <ListTree size={14} />
                          : <Eye size={14} />
                    }
                    label={
                      hasStructured
                        ? labels.structured
                        : preview?.kind === 'document'
                          ? labels.extracted
                          : labels.preview
                    }
                    onClick={() => {
                      setMode('preview');
                    }}
                  />
                  {(hasStructured || hasRenderedPreview) && preview?.content && (
                    <ModeButton
                      active={mode === 'text'}
                      icon={<FileText size={14} />}
                      label={labels.extracted}
                      onClick={() => setMode('text')}
                    />
                  )}
                  {preview?.editable && (
                    <>
                      <ModeButton
                        active={mode === 'edit'}
                        icon={<SquarePen size={14} />}
                        label={labels.edit}
                        onClick={() => setMode('edit')}
                      />
                      {supportsSplitPreview && (
                        <ModeButton
                          active={mode === 'split'}
                          icon={<SplitSquareHorizontal size={14} />}
                          label={labels.split}
                          onClick={() => setMode('split')}
                        />
                      )}
                    </>
                  )}
                </div>

                <div className="flex-1" />

                {preview?.editable && (
                  <>
                    <button
                      type="button"
                      disabled={!dirty || saving}
                      onClick={() => {
                        setDraft(preview.content ?? '');
                        setTextSelection(null);
                        setAgentInstruction('');
                        setCopiedAgentRequest(false);
                      }}
                      className="inline-flex h-8 items-center gap-1.5 rounded-md px-2.5 text-xs font-medium text-text-secondary transition-colors hover:bg-surface-2 hover:text-text-primary disabled:pointer-events-none disabled:opacity-40"
                    >
                      <RotateCcw size={14} />
                      {labels.discard}
                    </button>
                    <button
                      type="button"
                      disabled={!dirty || saving}
                      onClick={save}
                      className="inline-flex h-8 items-center gap-1.5 rounded-md bg-accent px-3 text-xs font-medium text-white transition-colors hover:bg-accent-hover disabled:pointer-events-none disabled:opacity-40"
                    >
                      {saving ? <Loader2 size={14} className="animate-spin" /> : <Save size={14} />}
                      {labels.save}
                    </button>
                  </>
                )}

                {preview && (
                  <>
                    <button
                      type="button"
                      onClick={() => {
                        void api.openFileInDefaultApp(preview.path);
                      }}
                      className="inline-flex h-8 items-center justify-center rounded-md px-2 text-text-tertiary transition-colors hover:bg-surface-2 hover:text-text-primary"
                      title={labels.openExternal}
                      aria-label={labels.openExternal}
                    >
                      <ExternalLink size={15} />
                    </button>
                    <button
                      type="button"
                      onClick={() => {
                        void api.showInFileExplorer(preview.path);
                      }}
                      className="inline-flex h-8 items-center justify-center rounded-md px-2 text-text-tertiary transition-colors hover:bg-surface-2 hover:text-text-primary"
                      title={labels.showFolder}
                      aria-label={labels.showFolder}
                    >
                      <FolderOpen size={15} />
                    </button>
                    <button
                      type="button"
                      onClick={async () => {
                        await navigator.clipboard.writeText(preview.path);
                        setCopiedPath(true);
                        setTimeout(() => setCopiedPath(false), 1600);
                      }}
                      className="inline-flex h-8 items-center justify-center rounded-md px-2 text-text-tertiary transition-colors hover:bg-surface-2 hover:text-text-primary"
                      title={labels.copyPath}
                      aria-label={labels.copyPath}
                    >
                      {copiedPath ? <Check size={15} className="text-success" /> : <Copy size={15} />}
                    </button>
                    <button
                      type="button"
                      onClick={() => {
                        if (preview) {
                          void loadFile(preview.path, { preferredMode: mode });
                        }
                      }}
                      className="inline-flex h-8 items-center justify-center rounded-md px-2 text-text-tertiary transition-colors hover:bg-surface-2 hover:text-text-primary"
                      title={labels.reload}
                      aria-label={labels.reload}
                    >
                      <RotateCcw size={15} />
                    </button>
                  </>
                )}
              </div>
            </header>

            {(preview?.warning || error) && (
              <div className="shrink-0 border-b border-warning/20 bg-warning/10 px-4 py-2">
                <div className="flex items-start gap-2 text-xs text-warning">
                  <TriangleAlert size={14} className="mt-0.5 shrink-0" />
                  <p className="min-w-0 whitespace-pre-wrap break-words">{error ?? preview?.warning}</p>
                </div>
              </div>
            )}

            <div className="min-h-0 flex-1 overflow-hidden bg-surface-0">
              {loading ? (
                <div className="flex h-full items-center justify-center gap-2 text-sm text-text-tertiary">
                  <Loader2 size={16} className="animate-spin" />
                  {labels.loading}
                </div>
              ) : !preview ? (
                <div className="flex h-full flex-col items-center justify-center gap-3 px-6 text-center text-sm text-text-tertiary">
                  <FileText size={28} />
                  <p>{error ?? labels.unsupported}</p>
                  <button
                    type="button"
                    onClick={close}
                    className="inline-flex h-8 items-center gap-1.5 rounded-md px-3 text-xs font-medium text-text-secondary transition-colors hover:bg-surface-2 hover:text-text-primary"
                  >
                    <X size={14} />
                    {labels.close}
                  </button>
                </div>
              ) : mode === 'text' && preview.content != null ? (
                <textarea ref={textPreview} data-testid="file-preview-text-view" aria-label={`${labels.extracted}: ${preview.displayName}`} readOnly value={content} onSelect={event => updateSelectionFromEditor(event.currentTarget)} className="h-full w-full resize-none border-0 bg-surface-0 px-4 py-3 font-mono text-xs leading-5 text-text-primary outline-none" />
              ) : isMediaPreview(preview) && error ? (
                <div role="alert" className="flex h-full items-center justify-center p-6 text-sm text-text-secondary">{error}</div>
              ) : isMediaPreview(preview) ? (
                <MediaPreview key={`${preview.path}:${loadGeneration.current}`} preview={preview} ready={() => { if (mediaReady.current?.path === preview.path) { mediaReady.current.resolve(); mediaReady.current = null; } }} failed={() => { setError(labels.unsupported); mediaReady.current?.reject(new Error(labels.unsupported)); mediaReady.current = null; toast.error(labels.unsupported); }} />
              ) : mode === 'edit' && preview.editable ? (
                <textarea
                  data-testid="file-preview-editor"
                  value={draft}
                  onChange={(event) => updateDraft(event.target.value)}
                  onSelect={(event) => updateSelectionFromEditor(event.currentTarget)}
                  onKeyUp={(event) => updateSelectionFromEditor(event.currentTarget)}
                  onMouseUp={(event) => updateSelectionFromEditor(event.currentTarget)}
                  spellCheck={false}
                  className="h-full w-full resize-none border-0 bg-surface-0 px-4 py-3 font-mono text-xs leading-5 text-text-primary outline-none placeholder:text-text-tertiary"
                />
              ) : mode === 'split' && preview.editable && supportsSplitPreview ? (
                <div className="grid h-full grid-cols-1 md:grid-cols-2">
                  <textarea
                    data-testid="file-preview-editor"
                    value={draft}
                    onChange={(event) => updateDraft(event.target.value)}
                    onSelect={(event) => updateSelectionFromEditor(event.currentTarget)}
                    onKeyUp={(event) => updateSelectionFromEditor(event.currentTarget)}
                    onMouseUp={(event) => updateSelectionFromEditor(event.currentTarget)}
                    spellCheck={false}
                    className="h-full w-full resize-none border-0 border-r border-border bg-surface-0 px-4 py-3 font-mono text-xs leading-5 text-text-primary outline-none placeholder:text-text-tertiary md:border-r"
                  />
                  <div className="h-full overflow-auto bg-surface-1">
                    <MarkdownPreview content={draft} />
                  </div>
                </div>
              ) : canShowPreview ? (
                mode === 'preview' && preview.structuredPreview ? (
                  <StructuredPreviewRenderer
                    preview={preview.structuredPreview}
                    labels={labels}
                    onMouseUp={captureRenderedSelection}
                    onOpenWebLink={openWebLink}
                  />
                ) : hasRenderedPreview && mode === 'preview' && preview.renderedPreview ? (
                  <OfficeRenderedPreview rendered={preview.renderedPreview} labels={labels} />
                ) : preview.content ? (
                <div
                  data-testid="file-preview-readable-content"
                  className="h-full overflow-auto"
                  onMouseUp={captureRenderedSelection}
                >
                  {preview.kind === 'markdown' ? (
                    <MarkdownPreview content={preview.editable ? draft : content} />
                  ) : preview.kind === 'html' ? (
                    <div className="flex h-full items-center justify-center p-6"><button type="button" disabled={dirty} title={dirty ? labels.save : t('browser.title')} className="rounded-lg border border-border px-4 py-2 text-sm disabled:opacity-40" onClick={() => void openHtml(preview.path).catch(error => toast.error(String(error)))}>{t('browser.title')}</button></div>
                  ) : (
                    <TextPreview content={preview.editable ? draft : content} />
                  )}
                </div>
                ) : (
                  <div className="flex h-full items-center justify-center px-6 text-center text-sm text-text-tertiary">
                    {labels.empty}
                  </div>
                )
              ) : (
                <div className="flex h-full items-center justify-center px-6 text-center text-sm text-text-tertiary">
                  {preview.kind === 'binary' ? labels.unsupported : labels.empty}
                </div>
              )}
            </div>

            <AnimatePresence>
              {preview && selectedText && (
                <motion.div
                  key="agent-selection-panel"
                  initial={shouldReduceMotion ? false : { y: 16, opacity: 0 }}
                  animate={{ y: 0, opacity: 1 }}
                  exit={shouldReduceMotion ? { opacity: 0 } : { y: 16, opacity: 0 }}
                  transition={shouldReduceMotion ? INSTANT_TRANSITION : { duration: 0.18, ease: [0.16, 1, 0.3, 1] }}
                  data-testid="file-preview-agent-panel"
                  className="shrink-0 border-t border-border bg-surface-1/95 px-4 py-3 shadow-[0_-12px_28px_rgba(0,0,0,0.16)] backdrop-blur"
                >
                  <div className="flex items-start gap-3">
                    <div className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-md border border-accent/25 bg-accent/10 text-accent">
                      <BotMessageSquare size={16} />
                    </div>
                    <div className="min-w-0 flex-1">
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="text-xs font-semibold text-text-primary">{labels.agentEdit}</span>
                        <span className="rounded-full border border-border bg-surface-2 px-2 py-0.5 text-[10px] font-medium text-text-tertiary">
                          {labels.selected} {selectedText.charCount} {labels.chars} · {labels.lineRange}{' '}
                          {selectedText.startLine === selectedText.endLine
                            ? selectedText.startLine
                            : `${selectedText.startLine}-${selectedText.endLine}`}
                        </span>
                      </div>

                      {(dirty || selectedText.charCount > MAX_AGENT_SELECTION_CHARS) && (
                        <p className={`mt-1 text-[11px] ${dirty ? 'text-warning' : 'text-text-tertiary'}`}>
                          {dirty ? labels.saveBeforeAgent : labels.selectionTooLarge}
                        </p>
                      )}
                      {!(preview.agentEditAllowed ?? Boolean(preview.sourceId)) && (
                        <p className="mt-1 text-[11px] text-warning">{labels.agentAccessRequired}</p>
                      )}

                      <div className="mt-2 flex flex-wrap gap-1.5">
                        {quickActions.map((action) => (
                          <button
                            key={action.id}
                            type="button"
                            onClick={() => {
                              setAgentInstruction(action.instruction);
                              setCopiedAgentRequest(false);
                            }}
                            className="inline-flex h-7 items-center gap-1.5 rounded-md border border-border bg-surface-2 px-2 text-[11px] font-medium text-text-secondary transition-colors hover:border-accent/40 hover:bg-accent/10 hover:text-text-primary"
                          >
                            {action.icon}
                            <span>{action.label}</span>
                          </button>
                        ))}
                      </div>

                      <div className="mt-2 flex flex-col gap-2 sm:flex-row">
                        <input
                          data-testid="file-preview-agent-instruction"
                          value={agentInstruction}
                          onChange={(event) => {
                            setAgentInstruction(event.target.value);
                            setCopiedAgentRequest(false);
                          }}
                          onKeyDown={(event) => {
                            if (event.key === 'Enter' && !event.nativeEvent.isComposing) {
                              event.preventDefault();
                              sendSelectionToAgent();
                            }
                          }}
                          placeholder={labels.agentInstructionPlaceholder}
                          className="h-9 min-w-0 flex-1 rounded-md border border-border bg-surface-0 px-3 text-xs text-text-primary outline-none transition-colors placeholder:text-text-tertiary focus:border-accent/60"
                        />
                        <div className="flex shrink-0 gap-2">
                          {dirty && (
                            <button
                              type="button"
                              disabled={saving}
                              onClick={save}
                              className="inline-flex h-9 items-center gap-1.5 rounded-md border border-border bg-surface-2 px-3 text-xs font-medium text-text-secondary transition-colors hover:bg-surface-3 hover:text-text-primary disabled:pointer-events-none disabled:opacity-40"
                            >
                              {saving ? <Loader2 size={14} className="animate-spin" /> : <Save size={14} />}
                              {labels.save}
                            </button>
                          )}
                          <button
                            type="button"
                            disabled={dirty}
                            onClick={copyAgentRequest}
                            data-testid="file-preview-agent-copy"
                            className="inline-flex h-9 items-center justify-center rounded-md border border-border bg-surface-2 px-3 text-text-secondary transition-colors hover:bg-surface-3 hover:text-text-primary disabled:pointer-events-none disabled:opacity-40"
                            title={labels.copyRequest}
                            aria-label={labels.copyRequest}
                          >
                            {copiedAgentRequest ? <Check size={15} className="text-success" /> : <Copy size={15} />}
                          </button>
                          <button
                            type="button"
                            disabled={dirty || !(preview.agentEditAllowed ?? Boolean(preview.sourceId))}
                            onClick={sendSelectionToAgent}
                            data-testid="file-preview-agent-send"
                            className="inline-flex h-9 items-center gap-1.5 rounded-md bg-accent px-3 text-xs font-medium text-white transition-colors hover:bg-accent-hover disabled:pointer-events-none disabled:opacity-40"
                          >
                            <BotMessageSquare size={14} />
                            {labels.askAgent}
                          </button>
                        </div>
                      </div>
                    </div>
                  </div>
                </motion.div>
              )}
            </AnimatePresence>
            </motion.aside>
          </>
        )}
      </AnimatePresence>
    </FilePreviewContext.Provider>
  );
}
