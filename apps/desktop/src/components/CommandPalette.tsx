import { useEffect, useState } from 'react';
import { Command } from 'cmdk';
import { useNavigate } from 'react-router-dom';
import { motion, AnimatePresence } from 'framer-motion';
import { Search, FolderOpen, BookOpen, ScanSearch, Database, Clock } from 'lucide-react';
import { toast } from 'sonner';
import * as api from '../lib/api';
import type { QueryLog } from '../types';

export function CommandPalette() {
  const [open, setOpen] = useState(false);
  const [recentQueries, setRecentQueries] = useState<QueryLog[]>([]);
  const navigate = useNavigate();

  /* ── Ctrl/Cmd+K toggle ───────────────────────────────────────────── */
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'k' && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        setOpen((prev) => !prev);
      }
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, []);

  /* ── Escape to close ─────────────────────────────────────────────── */
  useEffect(() => {
    if (!open) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        setOpen(false);
      }
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, [open]);

  /* ── Load recent queries on open ─────────────────────────────────── */
  useEffect(() => {
    if (!open) return;
    api.getRecentQueries(5).then(setRecentQueries).catch(() => {});
  }, [open]);

  /* ── Helpers ─────────────────────────────────────────────────────── */
  const select = (fn: () => void) => {
    setOpen(false);
    fn();
  };

  const handleScanAll = () => {
    select(() => {
      toast.promise(api.scanAllSources(), {
        loading: '正在扫描所有数据源…',
        success: '扫描完成',
        error: '扫描失败',
      });
    });
  };

  const handleRebuildEmbeddings = () => {
    select(() => {
      toast.promise(api.rebuildEmbeddings(), {
        loading: '正在重建向量索引…',
        success: '向量重建完成',
        error: '向量重建失败',
      });
    });
  };

  /* ── Render ──────────────────────────────────────────────────────── */
  return (
    <AnimatePresence>
      {open && (
        <div className="fixed inset-0 z-50">
          {/* Backdrop */}
          <motion.div
            className="absolute inset-0 bg-black/60 backdrop-blur-sm"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.15 }}
            onClick={() => setOpen(false)}
          />

          {/* Dialog */}
          <motion.div
            className="absolute left-1/2 top-[20%] w-full max-w-lg -translate-x-1/2 px-4"
            initial={{ opacity: 0, scale: 0.96, y: -8 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.96, y: -8 }}
            transition={{ duration: 0.2, ease: [0.16, 1, 0.3, 1] }}
          >
            <Command
              className="overflow-hidden rounded-xl border border-border bg-surface-1 shadow-lg"
              loop
            >
              <Command.Input
                placeholder="输入命令..."
                className="w-full border-b border-border bg-transparent px-4 py-3 text-sm
                  text-text-primary placeholder:text-text-tertiary outline-none"
                autoFocus
              />

              <Command.List className="max-h-72 overflow-y-auto p-2">
                <Command.Empty className="px-4 py-8 text-center text-sm text-text-tertiary">
                  未找到匹配命令
                </Command.Empty>

                {/* 导航 */}
                <Command.Group heading="导航">
                  <Command.Item onSelect={() => select(() => navigate('/'))}>
                    <Search className="h-4 w-4 shrink-0 text-text-tertiary" />
                    搜索
                  </Command.Item>
                  <Command.Item onSelect={() => select(() => navigate('/sources'))}>
                    <FolderOpen className="h-4 w-4 shrink-0 text-text-tertiary" />
                    数据源
                  </Command.Item>
                  <Command.Item onSelect={() => select(() => navigate('/playbooks'))}>
                    <BookOpen className="h-4 w-4 shrink-0 text-text-tertiary" />
                    剧本集
                  </Command.Item>
                </Command.Group>

                <Command.Separator className="mx-2 my-1 h-px bg-border" />

                {/* 操作 */}
                <Command.Group heading="操作">
                  <Command.Item onSelect={handleScanAll}>
                    <ScanSearch className="h-4 w-4 shrink-0 text-text-tertiary" />
                    扫描所有数据源
                  </Command.Item>
                  <Command.Item onSelect={handleRebuildEmbeddings}>
                    <Database className="h-4 w-4 shrink-0 text-text-tertiary" />
                    重建向量索引
                  </Command.Item>
                </Command.Group>

                {/* 最近搜索 */}
                {recentQueries.length > 0 && (
                  <>
                    <Command.Separator className="mx-2 my-1 h-px bg-border" />
                    <Command.Group heading="最近搜索">
                      {recentQueries.map((q) => (
                        <Command.Item
                          key={q.id}
                          value={q.queryText}
                          onSelect={() => select(() => navigate('/'))}
                        >
                          <Clock className="h-4 w-4 shrink-0 text-text-tertiary" />
                          <span className="truncate">{q.queryText}</span>
                        </Command.Item>
                      ))}
                    </Command.Group>
                  </>
                )}
              </Command.List>
            </Command>
          </motion.div>
        </div>
      )}
    </AnimatePresence>
  );
}
