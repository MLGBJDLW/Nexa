import { useState, useEffect, useCallback } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { BookOpen, Plus, Trash2, X, Pencil, FileText, Calendar } from 'lucide-react';
import { toast } from 'sonner';
import * as api from '../lib/api';
import type { Playbook, PlaybookCitation } from '../types';
import { Button } from '../components/ui/Button';
import { Input } from '../components/ui/Input';
import { Badge } from '../components/ui/Badge';
import { Modal } from '../components/ui/Modal';
import { Skeleton, CardSkeleton } from '../components/ui/Skeleton';
import { EmptyState } from '../components/ui/EmptyState';
import { ConfirmDialog } from '../components/ui/ConfirmDialog';

/* ------------------------------------------------------------------ */
/*  Helpers                                                            */
/* ------------------------------------------------------------------ */

function formatDate(iso: string): string {
  return new Date(iso).toLocaleDateString('zh-CN', {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  });
}

const listItemVariants = {
  hidden: { opacity: 0, x: -12 },
  visible: (i: number) => ({
    opacity: 1,
    x: 0,
    transition: { delay: i * 0.04, duration: 0.25, ease: [0.16, 1, 0.3, 1] as const },
  }),
  exit: { opacity: 0, x: -12, transition: { duration: 0.15 } },
};

const detailVariants = {
  hidden: { opacity: 0, x: 20 },
  visible: { opacity: 1, x: 0, transition: { duration: 0.3, ease: [0.16, 1, 0.3, 1] as const } },
  exit: { opacity: 0, x: 20, transition: { duration: 0.15 } },
};

/* ------------------------------------------------------------------ */
/*  Component                                                          */
/* ------------------------------------------------------------------ */

export function PlaybooksPage() {
  /* ── Data state ─────────────────────────────────────────────────── */
  const [playbooks, setPlaybooks] = useState<Playbook[]>([]);
  const [loading, setLoading] = useState(true);

  /* ── Create modal ───────────────────────────────────────────────── */
  const [showCreateModal, setShowCreateModal] = useState(false);
  const [formTitle, setFormTitle] = useState('');
  const [formDesc, setFormDesc] = useState('');
  const [formQuery, setFormQuery] = useState('');
  const [creating, setCreating] = useState(false);

  /* ── Detail view ────────────────────────────────────────────────── */
  const [selectedPlaybook, setSelectedPlaybook] = useState<Playbook | null>(null);
  const [citations, setCitations] = useState<PlaybookCitation[]>([]);
  const [loadingCitations, setLoadingCitations] = useState(false);

  /* ── Delete confirmation ────────────────────────────────────────── */
  const [deleteTarget, setDeleteTarget] = useState<Playbook | null>(null);
  const [deleting, setDeleting] = useState(false);

  /* ── Inline edit ────────────────────────────────────────────────── */
  const [editMode, setEditMode] = useState(false);
  const [editTitle, setEditTitle] = useState('');
  const [editDesc, setEditDesc] = useState('');
  const [saving, setSaving] = useState(false);

  /* ── Remove citation confirm ────────────────────────────────────── */
  const [removeCitTarget, setRemoveCitTarget] = useState<string | null>(null);

  /* ================================================================ */
  /*  Data loading                                                     */
  /* ================================================================ */

  const loadPlaybooks = useCallback(async () => {
    try {
      const list = await api.listPlaybooks();
      setPlaybooks(list);
    } catch (e) {
      toast.error(`加载剧本列表失败: ${String(e)}`);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadPlaybooks();
  }, [loadPlaybooks]);

  /* ================================================================ */
  /*  Handlers                                                         */
  /* ================================================================ */

  const handleCreate = async () => {
    if (!formTitle.trim()) return;
    setCreating(true);
    try {
      await api.createPlaybook(formTitle.trim(), formDesc.trim(), formQuery.trim());
      setFormTitle('');
      setFormDesc('');
      setFormQuery('');
      setShowCreateModal(false);
      toast.success('剧本已创建');
      await loadPlaybooks();
    } catch (e) {
      toast.error(`创建失败: ${String(e)}`);
    } finally {
      setCreating(false);
    }
  };

  const handleDelete = async () => {
    if (!deleteTarget) return;
    setDeleting(true);
    try {
      await api.deletePlaybook(deleteTarget.id);
      if (selectedPlaybook?.id === deleteTarget.id) {
        setSelectedPlaybook(null);
        setCitations([]);
      }
      toast.success('剧本已删除');
      await loadPlaybooks();
    } catch (e) {
      toast.error(`删除失败: ${String(e)}`);
    } finally {
      setDeleting(false);
      setDeleteTarget(null);
    }
  };

  const handleSelect = async (playbook: Playbook) => {
    setSelectedPlaybook(playbook);
    setEditMode(false);
    setLoadingCitations(true);
    try {
      const cits = await api.listCitations(playbook.id);
      setCitations(cits);
    } catch (e) {
      toast.error(`加载引用失败: ${String(e)}`);
    } finally {
      setLoadingCitations(false);
    }
  };

  const handleRemoveCitation = async () => {
    if (!removeCitTarget) return;
    try {
      await api.removeCitation(removeCitTarget);
      setCitations((prev) => prev.filter((c) => c.id !== removeCitTarget));
      toast.success('引用已移除');
    } catch (e) {
      toast.error(`移除引用失败: ${String(e)}`);
    } finally {
      setRemoveCitTarget(null);
    }
  };

  const startEdit = () => {
    if (!selectedPlaybook) return;
    setEditTitle(selectedPlaybook.title);
    setEditDesc(selectedPlaybook.description);
    setEditMode(true);
  };

  const handleSaveEdit = async () => {
    if (!selectedPlaybook || !editTitle.trim()) return;
    setSaving(true);
    try {
      const updated = await api.updatePlaybook(
        selectedPlaybook.id,
        editTitle.trim(),
        editDesc.trim(),
      );
      setSelectedPlaybook(updated);
      setEditMode(false);
      toast.success('剧本已更新');
      await loadPlaybooks();
    } catch (e) {
      toast.error(`更新失败: ${String(e)}`);
    } finally {
      setSaving(false);
    }
  };

  /* ================================================================ */
  /*  Loading skeleton                                                 */
  /* ================================================================ */

  if (loading) {
    return (
      <div className="mx-auto max-w-6xl p-6">
        <div className="flex items-center justify-between mb-6">
          <Skeleton className="h-7 w-24" />
          <Skeleton className="h-9 w-28 rounded-md" />
        </div>
        <div className="flex gap-6">
          <div className="w-[340px] shrink-0 space-y-2">
            {Array.from({ length: 4 }).map((_, i) => (
              <CardSkeleton key={i} />
            ))}
          </div>
          <div className="flex-1">
            <CardSkeleton />
          </div>
        </div>
      </div>
    );
  }

  /* ================================================================ */
  /*  Render                                                           */
  /* ================================================================ */

  return (
    <div className="mx-auto max-w-6xl p-6">
      {/* ── Header ──────────────────────────────────────────────── */}
      <div className="flex items-center justify-between mb-6">
        <h1 className="text-lg font-semibold text-text-primary">剧本集</h1>
        <Button
          variant="primary"
          size="sm"
          icon={<Plus size={15} />}
          onClick={() => setShowCreateModal(true)}
        >
          新建剧本
        </Button>
      </div>

      {/* ── Split panel ─────────────────────────────────────────── */}
      <div className="flex gap-6 items-start">
        {/* ── Left: list ──────────────────────────────────────── */}
        <div className="w-[340px] shrink-0 space-y-1.5 overflow-y-auto max-h-[calc(100vh-160px)] pr-1">
          {playbooks.length === 0 ? (
            <EmptyState
              icon={<BookOpen size={32} />}
              title="尚无剧本"
              description="创建你的第一个剧本来收集和整理搜索中的关键引用"
              action={{ label: '新建剧本', onClick: () => setShowCreateModal(true) }}
            />
          ) : (
            <AnimatePresence mode="popLayout">
              {playbooks.map((pb, i) => {
                const isActive = selectedPlaybook?.id === pb.id;
                return (
                  <motion.button
                    key={pb.id}
                    custom={i}
                    variants={listItemVariants}
                    initial="hidden"
                    animate="visible"
                    exit="exit"
                    layout
                    onClick={() => handleSelect(pb)}
                    className={`
                      w-full rounded-lg border p-3 text-left transition-colors cursor-pointer
                      ${isActive
                        ? 'border-accent bg-accent/8 ring-1 ring-accent/20'
                        : 'border-border bg-surface-1 hover:bg-surface-2 hover:border-border-hover'
                      }
                    `}
                  >
                    <div className="flex items-start gap-2.5">
                      <div className={`shrink-0 mt-0.5 ${isActive ? 'text-accent' : 'text-text-tertiary'}`}>
                        <BookOpen size={16} />
                      </div>
                      <div className="min-w-0 flex-1">
                        <p className="text-sm font-medium text-text-primary truncate">
                          {pb.title}
                        </p>
                        {pb.description && (
                          <p className="mt-0.5 text-xs text-text-tertiary truncate leading-relaxed">
                            {pb.description}
                          </p>
                        )}
                        <div className="mt-2 flex items-center gap-2">
                          <Badge variant="info">
                            {pb.citations?.length ?? 0} 引用
                          </Badge>
                          <span className="text-[11px] text-text-tertiary flex items-center gap-1">
                            <Calendar size={11} />
                            {formatDate(pb.createdAt)}
                          </span>
                        </div>
                      </div>
                    </div>
                  </motion.button>
                );
              })}
            </AnimatePresence>
          )}
        </div>

        {/* ── Right: detail ───────────────────────────────────── */}
        <div className="flex-1 min-w-0 min-h-[400px]">
          <AnimatePresence mode="wait">
            {selectedPlaybook ? (
              <motion.div
                key={selectedPlaybook.id}
                variants={detailVariants}
                initial="hidden"
                animate="visible"
                exit="exit"
                className="rounded-lg border border-border bg-surface-1 overflow-hidden"
              >
                {/* Detail header */}
                <div className="px-5 py-4 border-b border-border">
                  {editMode ? (
                    <div className="space-y-3">
                      <Input
                        value={editTitle}
                        onChange={(e) => setEditTitle(e.target.value)}
                        placeholder="剧本名称"
                      />
                      <Input
                        value={editDesc}
                        onChange={(e) => setEditDesc(e.target.value)}
                        placeholder="剧本描述（可选）"
                      />
                      <div className="flex items-center gap-2">
                        <Button
                          variant="primary"
                          size="sm"
                          onClick={handleSaveEdit}
                          loading={saving}
                          disabled={!editTitle.trim()}
                        >
                          保存
                        </Button>
                        <Button variant="ghost" size="sm" onClick={() => setEditMode(false)}>
                          取消
                        </Button>
                      </div>
                    </div>
                  ) : (
                    <div className="flex items-start justify-between gap-3">
                      <div className="min-w-0 flex-1">
                        <h2 className="text-base font-semibold text-text-primary truncate">
                          {selectedPlaybook.title}
                        </h2>
                        {selectedPlaybook.description && (
                          <p className="mt-1 text-sm text-text-secondary leading-relaxed">
                            {selectedPlaybook.description}
                          </p>
                        )}
                        <p className="mt-2 text-xs text-text-tertiary flex items-center gap-1">
                          <Calendar size={12} />
                          创建于 {formatDate(selectedPlaybook.createdAt)}
                        </p>
                      </div>
                      <div className="flex items-center gap-1 shrink-0">
                        <Button variant="ghost" size="sm" icon={<Pencil size={14} />} onClick={startEdit}>
                          编辑
                        </Button>
                        <Button
                          variant="danger"
                          size="sm"
                          icon={<Trash2 size={14} />}
                          onClick={() => setDeleteTarget(selectedPlaybook)}
                        >
                          删除剧本
                        </Button>
                      </div>
                    </div>
                  )}
                </div>

                {/* Citations section */}
                <div className="px-5 py-4">
                  <div className="flex items-center gap-2 mb-3">
                    <h3 className="text-xs font-medium uppercase tracking-wider text-text-tertiary">
                      引用列表
                    </h3>
                    <Badge variant="default">{citations.length}</Badge>
                  </div>

                  {loadingCitations ? (
                    <div className="space-y-2">
                      {Array.from({ length: 3 }).map((_, i) => (
                        <CardSkeleton key={i} />
                      ))}
                    </div>
                  ) : citations.length === 0 ? (
                    <div className="flex flex-col items-center justify-center py-12 text-center">
                      <div className="p-3 rounded-xl bg-surface-2 text-text-tertiary mb-3">
                        <FileText size={24} />
                      </div>
                      <p className="text-sm font-medium text-text-secondary mb-1">尚无引用</p>
                      <p className="text-xs text-text-tertiary max-w-xs leading-relaxed">
                        在搜索结果中将证据卡片保存到此剧本，引用将显示在这里
                      </p>
                    </div>
                  ) : (
                    <AnimatePresence mode="popLayout">
                      <div className="space-y-2">
                        {citations.map((cit, i) => (
                          <motion.div
                            key={cit.id}
                            custom={i}
                            variants={listItemVariants}
                            initial="hidden"
                            animate="visible"
                            exit="exit"
                            layout
                            className="group rounded-md border border-border bg-surface-2 p-3 transition-colors hover:border-border-hover"
                          >
                            <div className="flex items-start justify-between gap-2">
                              <div className="min-w-0 flex-1">
                                <p className="text-xs font-mono text-text-tertiary">
                                  片段: {cit.chunkId.slice(0, 12)}…
                                </p>
                                {cit.annotation && (
                                  <p className="mt-1.5 text-sm text-text-secondary leading-relaxed">
                                    {cit.annotation}
                                  </p>
                                )}
                              </div>
                              <Button
                                variant="ghost"
                                size="sm"
                                icon={<X size={14} />}
                                className="opacity-0 group-hover:opacity-100 transition-opacity shrink-0"
                                onClick={() => setRemoveCitTarget(cit.id)}
                              />
                            </div>
                          </motion.div>
                        ))}
                      </div>
                    </AnimatePresence>
                  )}
                </div>
              </motion.div>
            ) : (
              <motion.div
                key="empty-detail"
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                exit={{ opacity: 0 }}
                className="flex flex-col items-center justify-center h-full min-h-[400px] text-center"
              >
                <div className="p-4 rounded-2xl bg-surface-2 text-text-tertiary mb-4">
                  <BookOpen size={28} />
                </div>
                <p className="text-sm text-text-tertiary">选择一个剧本查看引用详情</p>
              </motion.div>
            )}
          </AnimatePresence>
        </div>
      </div>

      {/* ── Create playbook modal ────────────────────────────────── */}
      <Modal
        open={showCreateModal}
        onClose={() => setShowCreateModal(false)}
        title="新建剧本"
        footer={
          <>
            <Button variant="ghost" size="sm" onClick={() => setShowCreateModal(false)}>
              取消
            </Button>
            <Button
              variant="primary"
              size="sm"
              onClick={handleCreate}
              loading={creating}
              disabled={!formTitle.trim()}
            >
              创建
            </Button>
          </>
        }
      >
        <div className="space-y-4">
          <div>
            <label className="block text-xs font-medium text-text-secondary mb-1.5">
              名称 <span className="text-danger">*</span>
            </label>
            <Input
              value={formTitle}
              onChange={(e) => setFormTitle(e.target.value)}
              placeholder="例如：我的研究主题"
              autoFocus
            />
          </div>
          <div>
            <label className="block text-xs font-medium text-text-secondary mb-1.5">
              描述
            </label>
            <Input
              value={formDesc}
              onChange={(e) => setFormDesc(e.target.value)}
              placeholder="简要描述此剧本的用途"
            />
          </div>
          <div>
            <label className="block text-xs font-medium text-text-secondary mb-1.5">
              基础查询（可选）
            </label>
            <Input
              value={formQuery}
              onChange={(e) => setFormQuery(e.target.value)}
              placeholder="创建此剧本的原始搜索查询"
            />
          </div>
        </div>
      </Modal>

      {/* ── Delete confirm dialog ────────────────────────────────── */}
      <ConfirmDialog
        open={!!deleteTarget}
        onClose={() => setDeleteTarget(null)}
        onConfirm={handleDelete}
        title="删除剧本"
        message={`确定要删除「${deleteTarget?.title ?? ''}」吗？此操作不可撤销，剧本中的所有引用也将被移除。`}
        confirmText="删除剧本"
        variant="danger"
        loading={deleting}
      />

      {/* ── Remove citation confirm dialog ───────────────────────── */}
      <ConfirmDialog
        open={!!removeCitTarget}
        onClose={() => setRemoveCitTarget(null)}
        onConfirm={handleRemoveCitation}
        title="移除引用"
        message="确定要从此剧本中移除该引用吗？"
        confirmText="移除"
        variant="danger"
      />
    </div>
  );
}
