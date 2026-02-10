import { useState, useEffect, useCallback } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
  FolderOpen,
  FolderPlus,
  File,
  Globe,
  ScanSearch,
  Cpu,
  Trash2,
  Plus,
  RefreshCw,
} from 'lucide-react';
import { toast } from 'sonner';
import * as api from '../lib/api';
import type { Source, IngestResult, EmbedResult } from '../types';
import { Button } from '../components/ui/Button';
import { Input } from '../components/ui/Input';
import { Badge } from '../components/ui/Badge';
import { Modal } from '../components/ui/Modal';
import { CardSkeleton } from '../components/ui/Skeleton';
import { EmptyState } from '../components/ui/EmptyState';
import { ConfirmDialog } from '../components/ui/ConfirmDialog';

/* ------------------------------------------------------------------ */
/*  Helpers                                                            */
/* ------------------------------------------------------------------ */

const KIND_OPTIONS = [
  { value: 'local_folder', label: '本地文件夹', icon: FolderOpen },
  { value: 'single_file', label: '单个文件', icon: File },
  { value: 'web', label: '网页', icon: Globe },
] as const;

function kindIcon(kind: string) {
  switch (kind) {
    case 'single_file':
      return <File size={18} />;
    case 'web':
      return <Globe size={18} />;
    default:
      return <FolderOpen size={18} />;
  }
}

function kindLabel(kind: string) {
  return KIND_OPTIONS.find((k) => k.value === kind)?.label ?? kind;
}

function formatScanResult(r: IngestResult): string {
  const parts: string[] = [];
  parts.push(`扫描 ${r.filesScanned} 个文件`);
  if (r.filesAdded > 0) parts.push(`新增 ${r.filesAdded}`);
  if (r.filesUpdated > 0) parts.push(`更新 ${r.filesUpdated}`);
  if (r.filesSkipped > 0) parts.push(`跳过 ${r.filesSkipped}`);
  if (r.filesFailed > 0) parts.push(`失败 ${r.filesFailed}`);
  return parts.join('，');
}

function formatEmbedResult(r: EmbedResult): string {
  return `嵌入 ${r.chunksEmbedded} 个片段，跳过 ${r.chunksSkipped} 个 (${r.model})`;
}

/* ------------------------------------------------------------------ */
/*  Stagger animation                                                  */
/* ------------------------------------------------------------------ */

const listContainer = {
  hidden: {},
  show: { transition: { staggerChildren: 0.06 } },
};

const listItem = {
  hidden: { opacity: 0, y: 12 },
  show: { opacity: 1, y: 0, transition: { duration: 0.25, ease: [0.16, 1, 0.3, 1] as const } },
};

/* ------------------------------------------------------------------ */
/*  Component                                                          */
/* ------------------------------------------------------------------ */

export function SourcesPage() {
  const [sources, setSources] = useState<Source[]>([]);
  const [loading, setLoading] = useState(true);
  const [scanningId, setScanningId] = useState<string | null>(null);
  const [scanningAll, setScanningAll] = useState(false);
  const [embeddingId, setEmbeddingId] = useState<string | null>(null);
  const [rebuildingEmbeddings, setRebuildingEmbeddings] = useState(false);

  // Add source modal
  const [showAddModal, setShowAddModal] = useState(false);
  const [formKind, setFormKind] = useState('local_folder');
  const [formPath, setFormPath] = useState('');
  const [formInclude, setFormInclude] = useState('**/*.md');
  const [formExclude, setFormExclude] = useState('');
  const [adding, setAdding] = useState(false);

  // Delete confirmation
  const [deleteTarget, setDeleteTarget] = useState<Source | null>(null);
  const [deleting, setDeleting] = useState(false);

  /* ── Load ─────────────────────────────────────────────────────────── */

  const loadSources = useCallback(async () => {
    try {
      const list = await api.listSources();
      setSources(list);
    } catch (e) {
      toast.error(`加载数据源失败: ${String(e)}`);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadSources();
  }, [loadSources]);

  /* ── Add ──────────────────────────────────────────────────────────── */

  const resetForm = () => {
    setFormKind('local_folder');
    setFormPath('');
    setFormInclude('**/*.md');
    setFormExclude('');
  };

  const handleAdd = async () => {
    if (!formPath.trim()) return;
    setAdding(true);
    try {
      const includeGlobs = formInclude
        .split(',')
        .map((s) => s.trim())
        .filter(Boolean);
      const excludeGlobs = formExclude
        .split(',')
        .map((s) => s.trim())
        .filter(Boolean);
      await api.addSource(formPath.trim(), includeGlobs, excludeGlobs);
      toast.success('数据源已添加');
      resetForm();
      setShowAddModal(false);
      await loadSources();
    } catch (e) {
      toast.error(`添加失败: ${String(e)}`);
    } finally {
      setAdding(false);
    }
  };

  /* ── Delete ───────────────────────────────────────────────────────── */

  const handleDelete = async () => {
    if (!deleteTarget) return;
    setDeleting(true);
    try {
      await api.deleteSource(deleteTarget.id);
      toast.success('数据源已删除');
      setDeleteTarget(null);
      await loadSources();
    } catch (e) {
      toast.error(`删除失败: ${String(e)}`);
    } finally {
      setDeleting(false);
    }
  };

  /* ── Scan ──────────────────────────────────────────────────────────── */

  const handleScan = async (sourceId: string) => {
    setScanningId(sourceId);
    try {
      const result = await api.scanSource(sourceId);
      toast.success(formatScanResult(result));
    } catch (e) {
      toast.error(`扫描失败: ${String(e)}`);
    } finally {
      setScanningId(null);
    }
  };

  const handleScanAll = async () => {
    setScanningAll(true);
    try {
      const results = await api.scanAllSources();
      const total = results.reduce((sum, r) => sum + r.filesScanned, 0);
      const added = results.reduce((sum, r) => sum + r.filesAdded, 0);
      const updated = results.reduce((sum, r) => sum + r.filesUpdated, 0);
      toast.success(`全部扫描完成: ${total} 个文件，新增 ${added}，更新 ${updated}`);
    } catch (e) {
      toast.error(`全部扫描失败: ${String(e)}`);
    } finally {
      setScanningAll(false);
    }
  };

  /* ── Embed ─────────────────────────────────────────────────────────── */

  const handleEmbed = async (sourceId: string) => {
    setEmbeddingId(sourceId);
    try {
      const result = await api.embedSource(sourceId);
      toast.success(formatEmbedResult(result));
    } catch (e) {
      toast.error(`嵌入失败: ${String(e)}`);
    } finally {
      setEmbeddingId(null);
    }
  };

  const handleRebuildEmbeddings = async () => {
    setRebuildingEmbeddings(true);
    try {
      const result = await api.rebuildEmbeddings();
      toast.success(formatEmbedResult(result));
    } catch (e) {
      toast.error(`重建嵌入失败: ${String(e)}`);
    } finally {
      setRebuildingEmbeddings(false);
    }
  };

  /* ── Loading skeleton ──────────────────────────────────────────────── */

  if (loading) {
    return (
      <div className="mx-auto max-w-3xl p-6 space-y-3">
        <div className="flex items-center justify-between mb-6">
          <div className="h-7 w-32 bg-surface-3 rounded-md animate-pulse" />
          <div className="h-9 w-28 bg-surface-3 rounded-md animate-pulse" />
        </div>
        {Array.from({ length: 3 }).map((_, i) => (
          <CardSkeleton key={i} />
        ))}
      </div>
    );
  }

  /* ── Render ─────────────────────────────────────────────────────────── */

  return (
    <div className="mx-auto max-w-3xl p-6">
      {/* Header */}
      <div className="mb-6 flex items-center justify-between">
        <h2 className="text-lg font-semibold text-text-primary">数据源管理</h2>
        <div className="flex gap-2">
          <Button
            variant="secondary"
            size="sm"
            icon={<ScanSearch size={14} />}
            onClick={handleScanAll}
            loading={scanningAll}
            disabled={sources.length === 0}
          >
            全部扫描
          </Button>
          <Button
            variant="secondary"
            size="sm"
            icon={<RefreshCw size={14} />}
            onClick={handleRebuildEmbeddings}
            loading={rebuildingEmbeddings}
            disabled={sources.length === 0}
          >
            重建嵌入
          </Button>
          <Button
            variant="primary"
            size="sm"
            icon={<Plus size={14} />}
            onClick={() => setShowAddModal(true)}
          >
            添加数据源
          </Button>
        </div>
      </div>

      {/* Source list or empty state */}
      {sources.length === 0 ? (
        <EmptyState
          icon={<FolderPlus size={32} />}
          title="暂无数据源"
          description="添加本地文件夹或文件来开始构建你的知识库。"
          action={{ label: '添加数据源', onClick: () => setShowAddModal(true) }}
        />
      ) : (
        <motion.div
          className="space-y-3"
          variants={listContainer}
          initial="hidden"
          animate="show"
        >
          <AnimatePresence mode="popLayout">
            {sources.map((source) => (
              <motion.div
                key={source.id}
                variants={listItem}
                layout
                exit={{ opacity: 0, x: -20, transition: { duration: 0.2 } }}
                className="rounded-lg border border-border bg-surface-2 p-4 hover:border-border-hover transition-colors duration-fast"
              >
                <div className="flex items-start justify-between gap-3">
                  {/* Left: icon + info */}
                  <div className="flex items-start gap-3 min-w-0 flex-1">
                    <div className="shrink-0 p-2 rounded-lg bg-accent-subtle text-accent mt-0.5">
                      {kindIcon(source.kind)}
                    </div>
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-2 mb-1">
                        <p className="truncate text-sm font-medium text-text-primary font-mono">
                          {source.rootPath}
                        </p>
                        <Badge variant="default">{kindLabel(source.kind)}</Badge>
                      </div>

                      {/* Globs */}
                      <div className="flex flex-wrap gap-1 mb-1.5">
                        {source.includeGlobs.map((g, i) => (
                          <Badge key={i} variant="success">{g}</Badge>
                        ))}
                        {source.excludeGlobs.map((g, i) => (
                          <Badge key={`e-${i}`} variant="danger">✕ {g}</Badge>
                        ))}
                      </div>

                      {/* Meta row */}
                      <div className="flex items-center gap-3 text-[11px] text-text-tertiary">
                        <span>监听: {source.watchEnabled ? '开启' : '关闭'}</span>
                        <span>添加于: {new Date(source.createdAt).toLocaleDateString('zh-CN')}</span>
                      </div>
                    </div>
                  </div>

                  {/* Right: actions */}
                  <div className="flex shrink-0 gap-1.5">
                    <Button
                      variant="ghost"
                      size="sm"
                      icon={<ScanSearch size={14} />}
                      onClick={() => handleScan(source.id)}
                      loading={scanningId === source.id}
                    >
                      扫描
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      icon={<Cpu size={14} />}
                      onClick={() => handleEmbed(source.id)}
                      loading={embeddingId === source.id}
                    >
                      嵌入向量
                    </Button>
                    <Button
                      variant="danger"
                      size="sm"
                      icon={<Trash2 size={14} />}
                      onClick={() => setDeleteTarget(source)}
                    >
                      删除
                    </Button>
                  </div>
                </div>
              </motion.div>
            ))}
          </AnimatePresence>
        </motion.div>
      )}

      {/* ── Add Source Modal ──────────────────────────────────────────── */}
      <Modal
        open={showAddModal}
        onClose={() => { setShowAddModal(false); resetForm(); }}
        title="添加数据源"
        footer={
          <>
            <Button variant="ghost" size="sm" onClick={() => { setShowAddModal(false); resetForm(); }}>
              取消
            </Button>
            <Button
              variant="primary"
              size="sm"
              onClick={handleAdd}
              loading={adding}
              disabled={!formPath.trim()}
            >
              添加
            </Button>
          </>
        }
      >
        <div className="space-y-4">
          {/* Kind selector */}
          <div>
            <label className="block text-xs font-medium text-text-secondary mb-1.5">类型</label>
            <div className="flex gap-2">
              {KIND_OPTIONS.map((opt) => {
                const Icon = opt.icon;
                const active = formKind === opt.value;
                return (
                  <button
                    key={opt.value}
                    type="button"
                    onClick={() => setFormKind(opt.value)}
                    className={`
                      flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium
                      border transition-colors duration-fast cursor-pointer
                      ${active
                        ? 'border-accent bg-accent-subtle text-accent'
                        : 'border-border bg-surface-1 text-text-secondary hover:border-border-hover'
                      }
                    `}
                  >
                    <Icon size={14} />
                    {opt.label}
                  </button>
                );
              })}
            </div>
          </div>

          {/* Root path */}
          <div>
            <label className="block text-xs font-medium text-text-secondary mb-1.5">根路径</label>
            <Input
              value={formPath}
              onChange={(e) => setFormPath(e.target.value)}
              placeholder="C:\Users\you\notes  或  /home/user/notes"
              icon={<FolderOpen size={15} />}
            />
          </div>

          {/* Include globs */}
          <div>
            <label className="block text-xs font-medium text-text-secondary mb-1.5">匹配模式（逗号分隔）</label>
            <Input
              value={formInclude}
              onChange={(e) => setFormInclude(e.target.value)}
              placeholder="**/*.md, **/*.txt"
            />
          </div>

          {/* Exclude globs */}
          <div>
            <label className="block text-xs font-medium text-text-secondary mb-1.5">排除模式（逗号分隔）</label>
            <Input
              value={formExclude}
              onChange={(e) => setFormExclude(e.target.value)}
              placeholder="**/node_modules/**, **/.git/**"
            />
          </div>
        </div>
      </Modal>

      {/* ── Delete Confirm Dialog ────────────────────────────────────── */}
      <ConfirmDialog
        open={!!deleteTarget}
        onClose={() => setDeleteTarget(null)}
        onConfirm={handleDelete}
        title="删除数据源"
        message={`确定要删除数据源「${deleteTarget?.rootPath ?? ''}」吗？此操作不可撤销，关联的文档索引也将被移除。`}
        confirmText="删除"
        variant="danger"
        loading={deleting}
      />
    </div>
  );
}
