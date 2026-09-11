import { lazy, Suspense, useState, useMemo, useEffect, type CSSProperties } from 'react';
import { FileSpreadsheet } from 'lucide-react';
import { useTranslation } from '../../i18n';
import type * as api from '../../lib/api';
import { PreviewImage } from './PreviewImage';
type TranslateFn = ReturnType<typeof useTranslation>['t'];
export function createPreviewLabels(t: TranslateFn) {
  return {
    title: t('preview.title'),
    preview: t('preview.preview'),
    structured: t('preview.structured'),
    edit: t('preview.edit'),
    split: t('preview.split'),
    extracted: t('preview.extracted'),
    readOnly: t('preview.readOnly'),
    editable: t('preview.editable'),
    save: t('preview.save'),
    saved: t('preview.saved'),
    discard: t('preview.discard'),
    reload: t('preview.reload'),
    openExternal: t('preview.openExternal'),
    showFolder: t('preview.showFolder'),
    copyPath: t('preview.copyPath'),
    copied: t('preview.copied'),
    close: t('preview.close'),
    resizePanel: t('preview.resizePanel'),
    loading: t('preview.loading'),
    browserOpenFailed: t('browser.openFailed'),
    empty: t('preview.empty'),
    unsupported: t('preview.unsupported'),
    sheets: t('preview.sheets'),
    rows: t('preview.rows'),
    columns: t('preview.columns'),
    formula: t('preview.formula'),
    truncatedPreview: t('preview.truncatedPreview'),
    conflict: t('preview.conflict'),
    saveFailed: t('preview.saveFailed'),
    loadFailed: t('preview.loadFailed'),
    reindexFailed: t('preview.reindexFailed'),
    dirty: t('preview.dirty'),
    lines: t('preview.lines'),
    pages: t('preview.pages'),
    page: t('preview.page'),
    pageBreak: t('preview.pageBreak'),
    zoomIn: t('preview.zoomIn'),
    zoomOut: t('preview.zoomOut'),
    resetZoom: t('preview.resetZoom'),
    source: t('preview.source'),
    encoding: t('preview.encoding'),
    discardPrompt: t('preview.discardPrompt'),
    agentEdit: t('preview.agentEdit'),
    selected: t('preview.selected'),
    chars: t('preview.chars'),
    lineRange: t('preview.lineRange'),
    agentInstructionPlaceholder: t('preview.agentInstructionPlaceholder'),
    askAgent: t('preview.askAgent'),
    copyRequest: t('preview.copyRequest'),
    requestCopied: t('preview.requestCopied'),
    agentRequestSent: t('preview.agentRequestSent'),
    agentAccessRequired: t('preview.agentAccessRequired'),
    saveBeforeAgent: t('preview.saveBeforeAgent'),
    selectionTooLarge: t('preview.selectionTooLarge'),
    selectionMapFailed: t('preview.selectionMapFailed'),
    quickRewrite: t('preview.quickRewrite'),
    quickShorten: t('preview.quickShorten'),
    quickFix: t('preview.quickFix'),
    quickTranslateZh: t('preview.quickTranslateZh'),
  };
}

export function TextPreview({ content }: { content: string }) {
  const lines = content.split('\n');
  return (
    <pre className="min-h-full overflow-auto px-4 py-3 text-xs leading-5 text-text-secondary">
      {lines.map((line, index) => (
        <div key={index} className="grid grid-cols-[3rem_minmax(0,1fr)] gap-3">
          <span className="select-none text-right text-text-tertiary/70">{index + 1}</span>
          <code className="whitespace-pre-wrap break-words font-mono">{line || ' '}</code>
        </div>
      ))}
    </pre>
  );
}

const RichMarkdownPreview = lazy(() => import('./MarkdownPreview'));

export function MarkdownPreview({ content }: { content: string }) {
  return (
    <Suspense fallback={<pre className="whitespace-pre-wrap break-words px-5 py-4 text-sm text-text-primary">{content}</pre>}>
      <RichMarkdownPreview content={content} />
    </Suspense>
  );
}

export type PreviewLabels = ReturnType<typeof createPreviewLabels>;

function isSafeCssColor(value: string | null | undefined): string | undefined {
  if (!value) return undefined;
  if (/^#[0-9a-fA-F]{6}$/.test(value)) return value;
  return undefined;
}

function highlightColor(value: string | null | undefined): string | undefined {
  if (!value) return undefined;
  const normalized = value.toLowerCase();
  const named: Record<string, string> = {
    yellow: 'rgba(250, 204, 21, 0.22)',
    green: 'rgba(34, 197, 94, 0.18)',
    cyan: 'rgba(34, 211, 238, 0.18)',
    magenta: 'rgba(217, 70, 239, 0.2)',
    blue: 'rgba(59, 130, 246, 0.18)',
    red: 'rgba(239, 68, 68, 0.18)',
    darkyellow: 'rgba(202, 138, 4, 0.24)',
    darkgreen: 'rgba(22, 101, 52, 0.28)',
    darkcyan: 'rgba(14, 116, 144, 0.28)',
    darkmagenta: 'rgba(134, 25, 143, 0.28)',
    darkblue: 'rgba(30, 64, 175, 0.28)',
    darkred: 'rgba(153, 27, 27, 0.28)',
    lightgray: 'rgba(148, 163, 184, 0.18)',
    darkgray: 'rgba(71, 85, 105, 0.28)',
  };
  return named[normalized] ?? isSafeCssColor(value);
}

function textAlignClass(alignment: string | null | undefined): string {
  switch (alignment) {
    case 'center':
      return 'text-center';
    case 'right':
      return 'text-right';
    case 'both':
      return 'text-justify';
    default:
      return 'text-left';
  }
}

function runSizeClass(value: string | null | undefined): string {
  switch (value) {
    case 'small':
      return 'text-xs';
    case 'large':
      return 'text-base';
    case 'xlarge':
      return 'text-lg';
    default:
      return '';
  }
}

function DocumentRuns({
  runs,
  onOpenWebLink,
}: {
  runs: api.DocumentPreviewRun[];
  onOpenWebLink: (url: string, title?: string) => void;
}) {
  return (
    <>
      {runs.map((run, index) => {
        const style: CSSProperties = {
          color: isSafeCssColor(run.color),
          backgroundColor: highlightColor(run.backgroundColor),
        };
        const className = [
          run.bold ? 'font-semibold' : '',
          run.italic ? 'italic' : '',
          run.underline || run.hyperlink ? 'underline underline-offset-2' : '',
          run.hyperlink ? 'text-accent' : '',
          runSizeClass(run.fontSize),
        ]
          .filter(Boolean)
          .join(' ');
        const text = run.text || ' ';

        if (run.hyperlink) {
          return (
            <a
              key={`${index}-${run.text}`}
              href={run.hyperlink}
              onClick={(event) => {
                event.preventDefault();
                onOpenWebLink(run.hyperlink!, run.text || undefined);
              }}
              className={className}
              style={style}
            >
              {text}
            </a>
          );
        }

        return (
          <span key={`${index}-${run.text}`} className={className} style={style}>
            {text}
          </span>
        );
      })}
    </>
  );
}

function DocumentBlockView({
  block,
  assetMap,
  labels,
  onOpenWebLink,
}: {
  block: api.DocumentPreviewBlock;
  assetMap: Map<string, api.PreviewAsset>;
  labels: PreviewLabels;
  onOpenWebLink: (url: string, title?: string) => void;
}) {
  switch (block.type) {
    case 'heading': {
      const Tag = (`h${Math.min(Math.max(block.level, 1), 6)}`) as
        | 'h1'
        | 'h2'
        | 'h3'
        | 'h4'
        | 'h5'
        | 'h6';
      const headingClass =
        block.level <= 1
          ? 'mt-7 text-2xl font-semibold leading-tight'
          : block.level === 2
            ? 'mt-6 text-xl font-semibold leading-tight'
            : 'mt-5 text-base font-semibold leading-snug';
      return (
        <Tag className={`${headingClass} first:mt-0 ${textAlignClass(block.alignment)}`}>
          <DocumentRuns runs={block.runs} onOpenWebLink={onOpenWebLink} />
        </Tag>
      );
    }
    case 'paragraph':
      return (
        <p className={`my-3 whitespace-pre-wrap text-sm leading-7 ${textAlignClass(block.alignment)}`}>
          <DocumentRuns runs={block.runs} onOpenWebLink={onOpenWebLink} />
        </p>
      );
    case 'list': {
      const ListTag = block.ordered ? 'ol' : 'ul';
      return (
        <ListTag
          className={`my-3 space-y-1 text-sm leading-7 ${
            block.ordered ? 'list-decimal' : 'list-disc'
          }`}
          style={{ paddingLeft: `${1.5 + block.level * 1.25}rem` }}
        >
          {block.items.map((item, index) => (
            <li key={index}>
              <DocumentRuns runs={item.runs} onOpenWebLink={onOpenWebLink} />
            </li>
          ))}
        </ListTag>
      );
    }
    case 'table':
      return (
        <div className="my-4 overflow-x-auto rounded-md border border-border">
          <table className="min-w-full border-collapse text-sm">
            <tbody>
              {block.rows.map((row, rowIndex) => (
                <tr key={rowIndex} className="border-b border-border last:border-b-0">
                  {row.cells.map((cell, cellIndex) => (
                    <td
                      key={cellIndex}
                      className="min-w-32 border-r border-border bg-surface-0 px-3 py-2 align-top last:border-r-0"
                    >
                      {cell.blocks.length > 0 ? (
                        cell.blocks.map((child, childIndex) => (
                          <DocumentBlockView
                            key={childIndex}
                            block={child}
                            assetMap={assetMap}
                            labels={labels}
                            onOpenWebLink={onOpenWebLink}
                          />
                        ))
                      ) : (
                        <span className="text-text-tertiary"> </span>
                      )}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    case 'image': {
      const asset = assetMap.get(block.assetId);
      if (!asset) {
        return (
          <div className="my-4 rounded-md border border-border bg-surface-0 px-3 py-2 text-xs text-text-tertiary">
            {block.alt ?? block.assetId}
          </div>
        );
      }
      return (
        <figure className="my-5">
          <PreviewImage
            src={asset.path}
            alt={block.alt ?? asset.id}
            className="max-h-[520px] max-w-full rounded-md border border-border bg-surface-0 object-contain"
          />
          {(block.alt || asset.mimeType) && (
            <figcaption className="mt-1 text-[11px] text-text-tertiary">
              {block.alt ?? asset.mimeType}
            </figcaption>
          )}
        </figure>
      );
    }
    case 'pageBreak':
      return (
        <div className="my-7 flex items-center gap-3 text-[11px] uppercase tracking-wide text-text-tertiary">
          <span className="h-px flex-1 bg-border" />
          <span>{labels.pageBreak}</span>
          <span className="h-px flex-1 bg-border" />
        </div>
      );
    case 'unsupported':
      return (
        <div className="my-3 rounded-md border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning">
          {block.message}
        </div>
      );
    default:
      return null;
  }
}

function StructuredDocumentPreview({
  preview,
  labels,
  onMouseUp,
  onOpenWebLink,
}: {
  preview: api.DocumentStructuredPreview;
  labels: PreviewLabels;
  onMouseUp: () => void;
  onOpenWebLink: (url: string, title?: string) => void;
}) {
  const assetMap = useMemo(
    () => new Map(preview.assets.map((asset) => [asset.id, asset])),
    [preview.assets],
  );

  return (
    <div
      data-testid="file-preview-structured-document"
      className="h-full overflow-auto bg-surface-0 px-4 py-5"
      onMouseUp={onMouseUp}
    >
      <article className="mx-auto min-h-full max-w-[900px] rounded-md border border-border/70 bg-surface-1 px-6 py-6 text-text-primary shadow-[0_18px_45px_rgba(0,0,0,0.18)] sm:px-8 sm:py-7">
        {preview.blocks.map((block, index) => (
          <DocumentBlockView
            key={index}
            block={block}
            assetMap={assetMap}
            labels={labels}
            onOpenWebLink={onOpenWebLink}
          />
        ))}
      </article>
    </div>
  );
}

function columnName(column: number): string {
  let value = column + 1;
  let label = '';
  while (value > 0) {
    value -= 1;
    label = String.fromCharCode(65 + (value % 26)) + label;
    value = Math.floor(value / 26);
  }
  return label;
}

function WorkbookPreview({
  preview,
  labels,
  onMouseUp,
}: {
  preview: api.WorkbookStructuredPreview;
  labels: PreviewLabels;
  onMouseUp: () => void;
}) {
  const [selectedSheetIndex, setSelectedSheetIndex] = useState(0);

  useEffect(() => {
    setSelectedSheetIndex(0);
  }, [preview]);

  const sheet = preview.sheets[Math.min(selectedSheetIndex, Math.max(preview.sheets.length - 1, 0))];
  const cellMap = useMemo(() => {
    const map = new Map<string, api.WorkbookPreviewCell>();
    for (const cell of sheet?.cells ?? []) {
      map.set(`${cell.row}:${cell.column}`, cell);
    }
    return map;
  }, [sheet]);
  const mergeInfo = useMemo(() => {
    const starts = new Map<string, { rowSpan: number; colSpan: number }>();
    const covered = new Set<string>();
    for (const range of sheet?.mergedRanges ?? []) {
      const rowSpan = range.endRow - range.startRow + 1;
      const colSpan = range.endColumn - range.startColumn + 1;
      starts.set(`${range.startRow}:${range.startColumn}`, { rowSpan, colSpan });
      for (let row = range.startRow; row <= range.endRow; row += 1) {
        for (let col = range.startColumn; col <= range.endColumn; col += 1) {
          if (row !== range.startRow || col !== range.startColumn) {
            covered.add(`${row}:${col}`);
          }
        }
      }
    }
    return { starts, covered };
  }, [sheet]);

  if (!sheet) {
    return (
      <div className="flex h-full items-center justify-center px-6 text-center text-sm text-text-tertiary">
        {labels.empty}
      </div>
    );
  }

  const rowCount = Math.max(sheet.previewRowCount, 1);
  const columnCount = Math.max(sheet.previewColumnCount, 1);
  const columns = Array.from({ length: columnCount }, (_, index) => index);
  const rows = Array.from({ length: rowCount }, (_, index) => index);
  const gridStyle: CSSProperties = {
    gridTemplateColumns: `3rem repeat(${columnCount}, minmax(7rem, 1fr))`,
    gridTemplateRows: `2rem repeat(${rowCount}, minmax(2rem, auto))`,
  };

  return (
    <div data-testid="file-preview-workbook" className="flex h-full min-h-0 flex-col bg-surface-0">
      <div className="shrink-0 border-b border-border bg-surface-1/95 px-4 py-2">
        <div className="flex min-w-0 items-center gap-2">
          <FileSpreadsheet size={14} className="shrink-0 text-accent" />
          <span className="shrink-0 text-xs font-medium text-text-primary">
            {preview.sheets.length} {labels.sheets}
          </span>
          {preview.truncated && (
            <span className="rounded-full border border-warning/25 bg-warning/10 px-2 py-0.5 text-[10px] font-medium text-warning">
              {labels.truncatedPreview}
            </span>
          )}
          <div className="min-w-0 flex-1 overflow-x-auto">
            <div className="flex gap-1">
              {preview.sheets.map((candidate, index) => (
                <button
                  key={`${candidate.index}-${candidate.name}`}
                  type="button"
                  onClick={() => setSelectedSheetIndex(index)}
                  className={`h-7 shrink-0 rounded-md px-2.5 text-[11px] font-medium transition-colors ${
                    index === selectedSheetIndex
                      ? 'bg-accent text-white'
                      : 'text-text-secondary hover:bg-surface-3 hover:text-text-primary'
                  }`}
                >
                  {candidate.name}
                </button>
              ))}
            </div>
          </div>
          <span className="hidden shrink-0 text-[11px] text-text-tertiary sm:inline">
            {sheet.rowCount} {labels.rows} · {sheet.columnCount} {labels.columns}
          </span>
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-auto p-4" onMouseUp={onMouseUp}>
        <div className="grid min-w-max rounded-md border border-border bg-surface-1 text-xs" style={gridStyle}>
          <div className="sticky left-0 top-0 z-30 border-b border-r border-border bg-surface-2" />
          {columns.map((column) => (
            <div
              key={`col-${column}`}
              className="sticky top-0 z-20 flex h-8 items-center justify-center border-b border-r border-border bg-surface-2 font-medium text-text-tertiary"
              style={{ gridColumn: column + 2, gridRow: 1 }}
            >
              {columnName(column)}
            </div>
          ))}
          {rows.map((row) => (
            <div
              key={`row-${row}`}
              className="sticky left-0 z-10 flex h-8 items-center justify-end border-b border-r border-border bg-surface-2 px-2 font-medium text-text-tertiary"
              style={{ gridColumn: 1, gridRow: row + 2 }}
            >
              {row + 1}
            </div>
          ))}
          {rows.flatMap((row) =>
            columns.map((column) => {
              if (mergeInfo.covered.has(`${row}:${column}`)) return null;
              const cell = cellMap.get(`${row}:${column}`);
              const merged = mergeInfo.starts.get(`${row}:${column}`);
              const style: CSSProperties = {
                gridColumn: `${column + 2} / span ${merged?.colSpan ?? 1}`,
                gridRow: `${row + 2} / span ${merged?.rowSpan ?? 1}`,
              };
              return (
                <div
                  key={`${row}-${column}`}
                  className={`min-h-8 overflow-hidden border-b border-r border-border px-2 py-1.5 leading-5 ${
                    cell?.formula ? 'bg-accent/5' : 'bg-surface-1'
                  }`}
                  style={style}
                  title={cell?.formula ? `${labels.formula}: ${cell.formula}` : cell?.value}
                >
                  {cell ? (
                    <div className="flex min-w-0 items-start gap-1.5">
                      {cell.formula && (
                        <span className="mt-0.5 shrink-0 rounded border border-accent/30 px-1 text-[9px] font-semibold uppercase text-accent">
                          fx
                        </span>
                      )}
                      <span className="min-w-0 whitespace-pre-wrap break-words text-text-primary">
                        {cell.value}
                      </span>
                    </div>
                  ) : (
                    <span className="text-text-tertiary"> </span>
                  )}
                </div>
              );
            }),
          )}
        </div>
      </div>
    </div>
  );
}

export function StructuredPreviewRenderer({
  preview,
  labels,
  onMouseUp,
  onOpenWebLink,
}: {
  preview: api.StructuredPreview;
  labels: PreviewLabels;
  onMouseUp: () => void;
  onOpenWebLink: (url: string, title?: string) => void;
}) {
  if (preview.type === 'workbook') {
    return <WorkbookPreview preview={preview} labels={labels} onMouseUp={onMouseUp} />;
  }
  return (
    <StructuredDocumentPreview
      preview={preview}
      labels={labels}
      onMouseUp={onMouseUp}
      onOpenWebLink={onOpenWebLink}
    />
  );
}

