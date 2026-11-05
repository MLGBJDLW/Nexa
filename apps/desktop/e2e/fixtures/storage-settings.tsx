import { createRoot } from 'react-dom/client';
import { I18nProvider } from '../../src/i18n';
import { StorageUsageCard } from '../../src/components/settings/StorageUsageCard';
import type { StorageService } from '../../src/lib/api';

let rows = Array.from({ length: 103 }, (_, index) => ({ id: `chat-${index}`, title: `Conversation ${index}`, archivedAt: index < 2 ? '2026-11-01' : null, updatedAt: '2026-11-05' }));
const mutations: { action: string; ids: string[] }[] = [];
Object.assign(window, { storageMutations: mutations });
const service: StorageService = {
  async usage() { return { totalBytes: 1024 * 1024, partial: false, categories:[{id:'chats',bytes:512*1024},{id:'models',bytes:256*1024},{id:'reusable',bytes:256*1024}],
    activeConversations: rows.filter(row => !row.archivedAt).length, archivedConversations: rows.filter(row => row.archivedAt).length,
    database: {allocatedBytes:768*1024,reusableBytes:256*1024,mainFileBytes:768*1024,walBytes:0} }; },
  async page(scope, cursor) { const matching = rows.filter(row => scope === 'all' || (scope === 'archived') === Boolean(row.archivedAt));
    const start = cursor ? matching.findIndex(row => row.id === cursor.id) + 1 : 0;
    const entries = matching.slice(start,start+100); return {entries,next:start+100<matching.length ? entries[entries.length-1] : null}; },
  async archive(ids) { mutations.push({action:'archive',ids}); rows = rows.map(row => ids.includes(row.id) ? {...row,archivedAt:'2026-11-05'} : row); return ids.length; },
  async remove(ids) { mutations.push({action:'remove',ids}); rows = rows.filter(row => !ids.includes(row.id)); return ids.length; },
};
export function renderFixture() { createRoot(document.body.appendChild(document.createElement('div'))).render(<I18nProvider><main className="mx-auto max-w-3xl p-4"><StorageUsageCard service={service}/></main></I18nProvider>); }
