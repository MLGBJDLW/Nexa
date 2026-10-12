import { useId, useMemo, useState } from 'react';
import { ArrowDown, ArrowUp, Check, ChevronRight, Search } from 'lucide-react';
import { parseChatUi, type UiBlock, type UiCell } from '../../lib/chatUi';
import { useTranslation } from '../../i18n';
import './generativeUi.css';

function DataTable({ block }: { block: Extract<UiBlock, { type: 'table' }> }) {
  const { t } = useTranslation();
  const [query, setQuery] = useState('');
  const [sort, setSort] = useState<{ index: number; descending: boolean } | null>(null);
  const rows = useMemo(() => {
    const filtered = block.rows.filter(row => row.some(value => String(value ?? '').toLocaleLowerCase().includes(query.toLocaleLowerCase())));
    if (sort) filtered.sort((a, b) => {
      const av: UiCell = a[sort.index], bv: UiCell = b[sort.index];
      const comparison = typeof av === 'number' && typeof bv === 'number' ? av - bv : String(av ?? '').localeCompare(String(bv ?? ''), undefined, { numeric: true });
      return comparison * (sort.descending ? -1 : 1);
    });
    return filtered;
  }, [block.rows, query, sort]);
  return <div className="nexa-ui-table">
    {block.title && <h4>{block.title}</h4>}
    <label className="nexa-ui-search"><Search size={14} aria-hidden="true" /><input type="search" aria-label={t('chat.visualFilter')} placeholder={t('chat.visualFilter')} value={query} onChange={event => setQuery(event.target.value)} /></label>
    <div className="nexa-ui-table-scroll"><table><thead><tr>{block.columns.map((column, index) => <th key={index} scope="col" aria-sort={sort?.index === index ? sort.descending ? 'descending' : 'ascending' : 'none'}>
      <button type="button" onClick={() => setSort({ index, descending: sort?.index === index && !sort.descending })}>{column}{sort?.index === index && (sort.descending ? <ArrowDown size={12} /> : <ArrowUp size={12} />)}</button>
    </th>)}</tr></thead><tbody>{rows.map((row, index) => <tr key={index}>{row.map((value, col) => <td key={col}>{value ?? '—'}</td>)}</tr>)}</tbody></table></div>
    {!rows.length && <p className="nexa-ui-muted">{t('chat.visualNoRows')}</p>}
  </div>;
}

function Tabs({ block }: { block: Extract<UiBlock, { type: 'tabs' }> }) {
  const [selected, setSelected] = useState(0); const id = useId();
  const active = Math.min(selected, block.tabs.length - 1);
  return <div className="nexa-ui-tabs"><div role="tablist">{block.tabs.map((tab, index) => <button key={index} type="button" role="tab"
    id={`${id}-tab-${index}`} aria-controls={`${id}-panel-${index}`} aria-selected={index === active} tabIndex={index === active ? 0 : -1}
    onClick={() => setSelected(index)} onKeyDown={event => {
      let next = index;
      if (event.key === 'ArrowRight') next = (index + 1) % block.tabs.length;
      else if (event.key === 'ArrowLeft') next = (index + block.tabs.length - 1) % block.tabs.length;
      else if (event.key === 'Home') next = 0;
      else if (event.key === 'End') next = block.tabs.length - 1;
      else return;
      event.preventDefault(); setSelected(next); document.getElementById(`${id}-tab-${next}`)?.focus();
    }}>{tab.label}</button>)}</div>
    {block.tabs.map((tab, index) => <div key={index} role="tabpanel" id={`${id}-panel-${index}`} aria-labelledby={`${id}-tab-${index}`} hidden={index !== active} tabIndex={0}>
      <Blocks blocks={tab.blocks} />
    </div>)}
  </div>;
}

function Blocks({ blocks }: { blocks: UiBlock[] }) {
  return <div className="nexa-ui-blocks">{blocks.map((block, index) => {
    if (block.type === 'text') return <p key={index} className="nexa-ui-text">{block.text}</p>;
    if (block.type === 'metrics') return <div key={index} className="nexa-ui-metrics">{block.items.map((item, i) => <div key={i} className="nexa-ui-metric"><span>{item.label}</span><strong>{item.value}</strong>{item.detail && <small>{item.detail}</small>}</div>)}</div>;
    if (block.type === 'table') return <DataTable key={index} block={block} />;
    if (block.type === 'tabs') return <Tabs key={index} block={block} />;
    if (block.type === 'steps') return <ol key={index} className="nexa-ui-steps">{block.items.map((item, i) => <li key={i} data-status={item.status ?? 'pending'}>
      <span className="nexa-ui-step-icon" aria-hidden="true">{item.status === 'complete' ? <Check size={13} /> : item.status === 'current' ? <ChevronRight size={13} /> : i + 1}</span>
      <div><strong>{item.title}</strong>{item.detail && <p>{item.detail}</p>}</div>
    </li>)}</ol>;
    const max = Math.max(1, ...block.items.map(item => Math.abs(item.value)));
    const signed = block.items.some(item => item.value < 0);
    return <figure key={index} className="nexa-ui-bars">{block.title && <figcaption>{block.title}</figcaption>}{block.items.map((item, i) => {
      const width = Math.abs(item.value) / max * (signed ? 50 : 100);
      return <div key={i} className="nexa-ui-bar-row"><span>{item.label}</span><div className="nexa-ui-bar-track" aria-hidden="true">
        <i className="nexa-ui-bar-zero" style={{ left: signed ? '50%' : 0 }} />
        <i className="nexa-ui-bar" data-negative={item.value < 0} style={{ left: `${signed ? item.value < 0 ? 50 - width : 50 : 0}%`, width: `${width}%` }} />
      </div><output>{item.value}{block.unit ? ` ${block.unit}` : ''}</output></div>;
    })}</figure>;
  })}</div>;
}

export function GenerativeUi({ source, streaming }: { source: string; streaming: boolean }) {
  const { t } = useTranslation(); const result = useMemo(() => parseChatUi(source, streaming), [source, streaming]);
  return <section className="nexa-ui" data-testid="generative-ui" aria-busy={streaming}>
    {result.document ? <><header><span className="nexa-ui-eyebrow">{t('chat.visualLabel')}</span><h3>{result.document.title}</h3>{result.document.summary && <p>{result.document.summary}</p>}</header><Blocks blocks={result.document.blocks} /></>
      : <p className="nexa-ui-muted" role={streaming ? 'status' : 'alert'}>{streaming ? t('chat.visualBuilding') : t('chat.visualInvalid', { error: result.error })}</p>}
    <details className="nexa-ui-source"><summary>{t('chat.visualSource')}</summary><pre><code>{source}</code></pre></details>
  </section>;
}
