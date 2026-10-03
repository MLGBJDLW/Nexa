import { useState, useEffect, useRef, useCallback } from 'react';
import { Brain, Database, FolderKanban, FolderOpen, Plus, ChevronDown, Check, Pencil, Trash2 } from 'lucide-react';
import { toast } from 'sonner';
import { useTranslation } from '../../i18n';
import type { Project, CreateProjectInput, UpdateProjectInput } from '../../types/project';
import type { Source } from '../../types';
import * as api from '../../lib/api';
import {
  DEFAULT_PROJECT_COLOR,
  PROJECT_COLOR_OPTIONS,
  PROJECT_ICON_OPTIONS,
  ProjectIcon,
  getProjectIconOption,
  normalizeProjectColor,
} from '../../lib/projectIcons';
import { ConfirmDialog } from '../ui/ConfirmDialog';
import { ProjectMemoryPanel } from './ProjectMemoryPanel';
import { Modal } from '../ui/Modal';
import { Button } from '../ui/Button';
import { WorkspaceRootsPicker } from './WorkspaceRootsPicker';
import { formatUserError } from '../../lib/userError';
import { ProjectWorkspacePanel } from './ProjectWorkspacePanel';
import { WorkspaceRulesPanel } from './WorkspaceRulesPanel';
import { ProjectHooksPanel } from './ProjectHooksPanel';
import { useAppCommand } from '../../lib/appCommands';

const PROJECT_STORAGE_KEY = 'active-project-id';

function getStoredProjectId(): string | null {
  try {
    return localStorage.getItem(PROJECT_STORAGE_KEY);
  } catch {
    return null;
  }
}

function setStoredProjectId(id: string | null) {
  if (id) {
    localStorage.setItem(PROJECT_STORAGE_KEY, id);
  } else {
    localStorage.removeItem(PROJECT_STORAGE_KEY);
  }
}

interface ProjectSwitcherProps {
  activeProjectId: string | null;
  conversationId?: string | null;
  onProjectChange: (projectId: string | null) => void;
}

export function useActiveProject() {
  const [activeProjectId, setActiveProjectId] = useState<string | null>(getStoredProjectId);

  const setProject = useCallback((id: string | null) => {
    setActiveProjectId(id);
    setStoredProjectId(id);
  }, []);

  return { activeProjectId, setProject };
}

export function ProjectSwitcher({ activeProjectId, conversationId, onProjectChange }: ProjectSwitcherProps) {
  const { t } = useTranslation();
  const [projects, setProjects] = useState<Project[]>([]);
  const [open, setOpen] = useState(false);
  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState('');
  const [newIcon, setNewIcon] = useState(PROJECT_ICON_OPTIONS[0].id);
  const [newColor, setNewColor] = useState(DEFAULT_PROJECT_COLOR);
  const [newSourceScope, setNewSourceScope] = useState<string[]>([]);
  const [newRoots, setNewRoots] = useState<string[]>([]);
  const [editRoots, setEditRoots] = useState<string[]>([]);
  const [editWorkspaceChanged, setEditWorkspaceChanged] = useState(false);
  const [editingProjectId, setEditingProjectId] = useState<string | null>(null);
  const [editName, setEditName] = useState('');
  const [editIcon, setEditIcon] = useState(PROJECT_ICON_OPTIONS[0].id);
  const [editColor, setEditColor] = useState(DEFAULT_PROJECT_COLOR);
  const [editSourceScope, setEditSourceScope] = useState<string[]>([]);
  const [sources, setSources] = useState<Source[]>([]);
  const [deleteTarget, setDeleteTarget] = useState<Project | null>(null);
  const [projectBusy, setProjectBusy] = useState(false);
  const [showMemoryPanel, setShowMemoryPanel] = useState(false);
  const [showWorkspacePanel, setShowWorkspacePanel] = useState(false);
  const [showRulesPanel, setShowRulesPanel] = useState(false);
  const [showHooksPanel, setShowHooksPanel] = useState(false);
  useAppCommand({ id: 'chat.hooks', label: 'project.hooks', keywords: 'hooks checks lifecycle 校验 钩子', enabled: !!activeProjectId, run: () => setShowHooksPanel(true) });
  useAppCommand({ id: 'chat.rules', label: 'project.fileRules', keywords: 'rules instructions AGENTS.md 规则', enabled: !!activeProjectId, run: () => setShowRulesPanel(true) });
  const dropdownRef = useRef<HTMLDivElement>(null);

  const loadProjects = useCallback(async () => {
    try {
      const list = await api.listProjects();
      setProjects(Array.isArray(list) ? list : []);
    } catch {
      // non-critical
    }
  }, []);

  useEffect(() => {
    loadProjects();
  }, [loadProjects]);

  useEffect(() => {
    api.listSources()
      .then((list) => setSources(Array.isArray(list) ? list : []))
      .catch(() => setSources([]));
  }, []);

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        setOpen(false);
        setCreating(false);
        setNewName('');
        setNewIcon(PROJECT_ICON_OPTIONS[0].id);
        setNewColor(DEFAULT_PROJECT_COLOR);
        setNewSourceScope([]);
        setEditingProjectId(null);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [open]);

  const activeProject = projects.find((p) => p.id === activeProjectId);

  const handleSelect = (id: string | null) => {
    onProjectChange(id);
    setOpen(false);
  };

  const handleCreate = async () => {
    const trimmed = newName.trim();
    if (!trimmed) return;
    setProjectBusy(true);
    try {
      const input: CreateProjectInput = {
        name: trimmed,
        icon: newIcon,
        color: newColor,
        sourceScope: newSourceScope.length > 0 ? newSourceScope : null,
        workspaceRoots: newRoots.map(root => root.trim()).filter(Boolean),
      };
      const created = await api.createProject(input);
      setNewName('');
      setNewIcon(PROJECT_ICON_OPTIONS[0].id);
      setNewColor(DEFAULT_PROJECT_COLOR);
      setNewSourceScope([]);
      setCreating(false);
      await loadProjects();
      onProjectChange(created.id);
      toast.success(t('project.created'));
      setOpen(false);
    } catch (error) {
      toast.error(formatUserError(t('common.error'), error));
    } finally {
      setProjectBusy(false);
    }
  };

  const startEditProject = (project: Project) => {
    setCreating(false);
    setEditingProjectId(project.id);
    setEditName(project.name);
    setEditIcon(getProjectIconOption(project.icon).id);
    setEditColor(normalizeProjectColor(project.color));
    setEditSourceScope(project.sourceScope ?? []);
    setEditRoots(project.workspaceRoots ?? []);
    setEditWorkspaceChanged(false);
    setOpen(false);
  };

  const cancelEditProject = () => {
    setEditingProjectId(null);
    setEditName('');
    setEditIcon(PROJECT_ICON_OPTIONS[0].id);
    setEditColor(DEFAULT_PROJECT_COLOR);
    setEditSourceScope([]);
  };

  const handleUpdateProject = async (project: Project) => {
    const trimmed = editName.trim();
    if (!trimmed) return;
    const input: UpdateProjectInput = {
      name: trimmed,
      icon: editIcon,
      color: editColor,
      sourceScope: editSourceScope.length > 0 ? editSourceScope : [],
      workspaceRoots: editWorkspaceChanged ? editRoots.map(root => root.trim()).filter(Boolean) : undefined,
    };
    setProjectBusy(true);
    try {
      const updated = await api.updateProject(project.id, input);
      setProjects((prev) => prev.map((item) => item.id === updated.id ? updated : item));
      cancelEditProject();
      toast.success(t('common.success'));
    } catch (error) {
      toast.error(formatUserError(t('common.error'), error));
    } finally {
      setProjectBusy(false);
    }
  };

  const handleDeleteProject = async () => {
    if (!deleteTarget) return;
    setProjectBusy(true);
    try {
      await api.deleteProject(deleteTarget.id);
      setProjects((prev) => prev.filter((item) => item.id !== deleteTarget.id));
      if (activeProjectId === deleteTarget.id) {
        onProjectChange(null);
      }
      setDeleteTarget(null);
      setEditingProjectId(null);
      toast.success(t('project.deleted'));
    } catch (error) {
      toast.error(formatUserError(t('common.error'), error));
    } finally {
      setProjectBusy(false);
    }
  };

  const renderIconPicker = (value: string, onChange: (id: string) => void) => (
    <div className="grid grid-cols-6 gap-1">
      {PROJECT_ICON_OPTIONS.map((option) => {
        const selected = option.id === value;
        const Icon = option.icon;
        return (
          <button
            key={option.id}
            type="button"
            onClick={() => onChange(option.id)}
            className={`flex h-7 w-7 items-center justify-center rounded-md border transition-colors ${
              selected
                ? 'border-accent bg-accent/10 text-accent'
                : 'border-border bg-surface-1 text-text-tertiary hover:bg-surface-3 hover:text-text-primary'
            }`}
            title={option.label}
            aria-label={option.label}
          >
            <Icon size={14} />
          </button>
        );
      })}
    </div>
  );

  const renderColorPicker = (value: string, onChange: (color: string) => void) => {
    const normalized = normalizeProjectColor(value);
    return (
      <div className="flex items-center gap-1.5">
        <div className="grid flex-1 grid-cols-6 gap-1">
          {PROJECT_COLOR_OPTIONS.map((option) => {
            const selected = normalizeProjectColor(option.value) === normalized;
            return (
              <button
                key={option.value}
                type="button"
                onClick={() => onChange(option.value)}
                className={`h-6 rounded-md border transition-all ${
                  selected
                    ? 'border-text-primary ring-1 ring-text-primary/30'
                    : 'border-border/70 hover:border-border-hover'
                }`}
                style={{ backgroundColor: option.value }}
                title={option.label}
                aria-label={t('project.colorLabel', { label: option.label })}
              />
            );
          })}
        </div>
        <label
          className="relative flex h-6 w-7 shrink-0 cursor-pointer items-center justify-center rounded-md border border-border bg-surface-1 transition-colors hover:bg-surface-3"
          title={t('project.customColor')}
          aria-label={t('project.customColor')}
        >
          <span
            className="h-3.5 w-3.5 rounded-full border border-white/30 shadow-sm"
            style={{ backgroundColor: normalized }}
          />
          <input
            type="color"
            value={normalized}
            onChange={(event) => onChange(normalizeProjectColor(event.target.value))}
            className="absolute inset-0 cursor-pointer opacity-0"
          />
        </label>
      </div>
    );
  };

  const renderSourceScopePicker = (value: string[], onChange: (ids: string[]) => void) => {
    const selected = new Set(value);
    const toggleSource = (sourceId: string) => {
      const next = new Set(selected);
      if (next.has(sourceId)) {
        next.delete(sourceId);
      } else {
        next.add(sourceId);
      }
      onChange(Array.from(next));
    };

    if (sources.length === 0) {
      return null;
    }

    return (
      <div className="space-y-1.5 rounded-md border border-border/70 bg-surface-1 p-2">
        <div className="flex items-center gap-1.5 text-[11px] font-medium text-text-secondary">
          <Database size={12} />
          <span>{t('project.sources')}</span>
          <span className="ml-auto text-[10px] text-text-tertiary">
            {selected.size === 0 ? t('project.allSources') : `${selected.size}/${sources.length}`}
          </span>
        </div>
        <div className="max-h-28 space-y-1 overflow-y-auto pr-1">
          {sources.map((source) => {
            const checked = selected.has(source.id);
            return (
              <button
                key={source.id}
                type="button"
                onClick={() => toggleSource(source.id)}
                className="flex w-full items-center gap-1.5 rounded px-1.5 py-1 text-left text-[11px] text-text-secondary hover:bg-surface-3 hover:text-text-primary"
              >
                <span
                  className={`flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded border ${
                    checked
                      ? 'border-accent bg-accent text-white'
                      : 'border-border bg-surface-0'
                  }`}
                >
                  {checked && <Check size={10} />}
                </span>
                <span className="truncate">{source.rootPath}</span>
              </button>
            );
          })}
        </div>
      </div>
    );
  };

  const renderProjectRow = (project: Project) => {

    return (
      <div key={project.id} className="group flex items-center gap-1 px-1">
        <button
          onClick={() => handleSelect(project.id)}
          className="flex min-w-0 flex-1 items-center gap-2 rounded-md px-2 py-1.5 text-text-secondary transition-colors hover:bg-surface-3 hover:text-text-primary"
        >
          <ProjectIcon icon={project.icon} color={project.color} className="h-5 w-5" size={12} />
          <span className="flex-1 truncate text-left">{project.name}</span>
          {project.id === activeProjectId && <Check className="h-3 w-3 shrink-0 text-accent" />}
        </button>
        <div className="flex shrink-0 items-center opacity-0 transition-opacity group-hover:opacity-100">
          <button
            type="button"
            onClick={() => startEditProject(project)}
            className="rounded p-1.5 text-text-tertiary hover:bg-accent/10 hover:text-accent"
            aria-label={t('common.edit')}
            title={t('common.edit')}
          >
            <Pencil size={12} />
          </button>
          <button
            type="button"
            onClick={() => setDeleteTarget(project)}
            className="rounded p-1.5 text-text-tertiary hover:bg-danger/10 hover:text-danger"
            aria-label={t('project.delete')}
            title={t('project.delete')}
          >
            <Trash2 size={12} />
          </button>
        </div>
      </div>
    );
  };

  return (
    <div className="relative" ref={dropdownRef}>
      <button
        onClick={() => setOpen((v) => !v)}
        className="flex w-full cursor-pointer items-center gap-2 rounded-md px-3 py-2 text-xs font-medium text-text-primary transition-colors hover:bg-surface-2"
      >
        <FolderOpen className="h-3.5 w-3.5 shrink-0 text-text-tertiary" />
        <span className="min-w-0 flex-1 truncate text-left">
          {activeProject ? (
            <span className="inline-flex min-w-0 items-center gap-1.5">
              <ProjectIcon icon={activeProject.icon} color={activeProject.color} className="h-5 w-5" size={12} />
              <span className="truncate">{activeProject.name}</span>
            </span>
          ) : (
            t('project.allConversations')
          )}
        </span>
        <ChevronDown className={`h-3 w-3 shrink-0 text-text-tertiary transition-transform ${open ? 'rotate-180' : ''}`} />
      </button>

      {open && (
        <div className="absolute left-0 right-0 top-full z-50 mt-1 max-h-96 overflow-y-auto rounded-lg border border-border bg-surface-2 py-1 text-xs shadow-lg">
          <button
            onClick={() => handleSelect(null)}
            className="flex w-full cursor-pointer items-center gap-2 px-3 py-1.5 text-text-secondary transition-colors hover:bg-surface-3 hover:text-text-primary"
          >
            <FolderOpen className="h-3 w-3 shrink-0" />
            <span className="flex-1 text-left">{t('project.allConversations')}</span>
            {activeProjectId === null && <Check className="h-3 w-3 shrink-0 text-accent" />}
          </button>

          {projects.length > 0 && <div className="my-1 border-t border-border" />}
          {projects.map(renderProjectRow)}

          <div className="my-1 border-t border-border" />

          {activeProjectId && (
            <button
              onClick={() => {
                setShowWorkspacePanel(true);
                setOpen(false);
              }}
              data-testid="project-workspace-open"
              className="flex w-full cursor-pointer items-center gap-2 px-3 py-1.5 text-text-secondary transition-colors hover:bg-surface-3 hover:text-text-primary"
            >
              <FolderKanban className="h-3 w-3 shrink-0" />
              <span className="flex-1 text-left">{t('project.workspaceTitle')}</span>
            </button>
          )}

          {activeProjectId && <button type="button" data-testid="project-rules-open"
            onClick={() => { setShowRulesPanel(true); setOpen(false); }}
            className="flex w-full cursor-pointer items-center gap-2 px-3 py-1.5 text-text-secondary transition-colors hover:bg-surface-3 hover:text-text-primary">
            <FolderOpen className="h-3 w-3 shrink-0" /><span className="flex-1 text-left">{t('project.fileRules')}</span>
          </button>}
          {activeProjectId && <button type="button" data-testid="project-hooks-open"
            onClick={() => { setShowHooksPanel(true); setOpen(false); }}
            className="flex w-full cursor-pointer items-center gap-2 px-3 py-1.5 text-text-secondary transition-colors hover:bg-surface-3 hover:text-text-primary">
            <Check className="h-3 w-3 shrink-0" /><span className="flex-1 text-left">{t('project.hooks')}</span>
          </button>}

          {activeProjectId && (
            <button
              onClick={() => {
                setShowMemoryPanel(true);
                setOpen(false);
              }}
              className="flex w-full cursor-pointer items-center gap-2 px-3 py-1.5 text-text-secondary transition-colors hover:bg-surface-3 hover:text-text-primary"
            >
              <Brain className="h-3 w-3 shrink-0" />
              <span className="flex-1 text-left">{t('chat.projectMemoryMenu')}</span>
            </button>
          )}

            <button
              onClick={() => {
                setEditingProjectId(null);
                setNewSourceScope([]);
                setNewRoots([]);
                setNewName('');
                setCreating(true);
                setOpen(false);
              }}
              className="flex w-full cursor-pointer items-center gap-2 px-3 py-1.5 text-accent transition-colors hover:bg-surface-3 hover:text-accent-hover"
            >
              <Plus className="h-3 w-3 shrink-0" />
              <span>{t('project.createNew')}</span>
            </button>
        </div>
      )}

      <Modal
        open={creating || editingProjectId !== null}
        onClose={() => { if (!projectBusy) { setCreating(false); cancelEditProject(); } }}
        title={creating ? t('project.createNew') : t('project.editProject')}
        surfaceClassName="bg-surface-1 !w-[min(92vw,40rem)] !rounded-2xl"
        footer={<div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => { setCreating(false); cancelEditProject(); }} disabled={projectBusy}>{t('common.cancel')}</Button>
          <Button disabled={projectBusy || !(creating ? newName : editName).trim()} onClick={() => {
            if (creating) void handleCreate();
            else { const project = projects.find(item => item.id === editingProjectId); if (project) void handleUpdateProject(project); }
          }} data-testid="project-save">{t('common.save')}</Button>
        </div>}
      >
        <div className="space-y-6 p-1 sm:p-2" data-testid="project-editor">
          <div className="flex items-center gap-3">
            <ProjectIcon icon={creating ? newIcon : editIcon} color={creating ? newColor : editColor} className="h-12 w-12 shrink-0 rounded-xl" size={24} />
            <input autoFocus onKeyDown={event => {
              if (event.key !== 'Enter' || projectBusy) return;
              event.preventDefault();
              if (creating) void handleCreate();
              else { const project = projects.find(item => item.id === editingProjectId); if (project) void handleUpdateProject(project); }
            }} value={creating ? newName : editName} onChange={event => (creating ? setNewName : setEditName)(event.target.value)} placeholder={t('project.namePlaceholder')} aria-label={t('project.namePlaceholder')} className="min-w-0 flex-1 border-b border-border/70 bg-transparent py-2 text-lg font-medium text-text-primary outline-none placeholder:text-text-tertiary focus:border-accent" />
          </div>
          <WorkspaceRootsPicker roots={creating ? newRoots : editRoots} onChange={creating ? setNewRoots : roots => { setEditRoots(roots); setEditWorkspaceChanged(true); }} />
          <div className="space-y-3 border-t border-border/60 pt-4">
            <details>
              <summary className="cursor-pointer text-xs text-text-tertiary hover:text-text-primary">{t('project.appearance')}</summary>
              <div className="mt-3 grid grid-cols-1 gap-4 sm:grid-cols-2">
                {renderIconPicker(creating ? newIcon : editIcon, creating ? setNewIcon : setEditIcon)}
                {renderColorPicker(creating ? newColor : editColor, creating ? setNewColor : setEditColor)}
              </div>
            </details>
            {sources.length > 0 && <details>
              <summary className="cursor-pointer text-xs text-text-tertiary hover:text-text-primary">{t('project.sources')}</summary>
              <div className="mt-3">{renderSourceScopePicker(creating ? newSourceScope : editSourceScope, creating ? setNewSourceScope : setEditSourceScope)}</div>
            </details>}
          </div>
        </div>
      </Modal>

      <ProjectMemoryPanel
        projectId={activeProjectId}
        open={showMemoryPanel}
        onClose={() => setShowMemoryPanel(false)}
      />
      <ProjectWorkspacePanel
        projectId={activeProjectId}
        open={showWorkspacePanel}
        onClose={() => setShowWorkspacePanel(false)}
        onManageFolders={project => { setShowWorkspacePanel(false); startEditProject(project); }}
      />
      <Modal open={showRulesPanel && !!activeProjectId} onClose={() => setShowRulesPanel(false)} title={t('project.fileRules')}>
        {showRulesPanel && activeProjectId && <WorkspaceRulesPanel projectId={activeProjectId} conversationId={conversationId} />}
      </Modal>
      <Modal open={showHooksPanel && !!activeProjectId} onClose={() => setShowHooksPanel(false)} title={t('project.hooks')}>
        {showHooksPanel && activeProjectId && <ProjectHooksPanel projectId={activeProjectId} />}
      </Modal>
      <ConfirmDialog
        open={!!deleteTarget}
        onClose={() => setDeleteTarget(null)}
        onConfirm={() => { void handleDeleteProject(); }}
        title={t('project.deleteConfirm')}
        message={t('project.deleteConfirmDesc')}
        confirmText={t('common.delete')}
        variant="danger"
        loading={projectBusy}
      />
    </div>
  );
}
