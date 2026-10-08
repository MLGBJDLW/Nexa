import { useMemo, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { AnimatePresence, motion } from 'framer-motion';
import { AlertTriangle, CheckCircle2, FileArchive, FolderOpen, Loader2, PackagePlus, ShieldAlert, X } from 'lucide-react';
import { useTranslation } from '../../i18n';
import * as api from '../../lib/api';
import type { DiscoveredSkillBundle, Skill } from '../../types/extensions';
import { Badge } from '../ui/Badge';
import { Button } from '../ui/Button';

interface SkillInstallerProps {
  skills: Skill[];
  onInstalled?: () => void;
}

export function SkillInstaller({ skills, onInstalled }: SkillInstallerProps) {
  const { t } = useTranslation();
  const [openInstaller, setOpenInstaller] = useState(false);
  const [sources, setSources] = useState<string[]>([]);
  const [preview, setPreview] = useState<DiscoveredSkillBundle[]>([]);
  const [selectedFiles, setSelectedFiles] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [replaceExisting, setReplaceExisting] = useState(false);
  const [acceptBlocked, setAcceptBlocked] = useState(false);

  const installedNames = useMemo(
    () => new Set(skills.filter((skill) => !skill.builtin).map((skill) => skill.canonicalName.toLocaleLowerCase())),
    [skills],
  );
  const builtinNames = useMemo(
    () => new Set(skills.filter((skill) => skill.builtin).map((skill) => skill.canonicalName.toLocaleLowerCase())),
    [skills],
  );
  const selected = useMemo(() => {
    const files = new Set(selectedFiles);
    return preview.filter(skill => files.has(skill.skillFile));
  }, [preview, selectedFiles]);
  const duplicateNames = useMemo(() => {
    const seen = new Set<string>();
    return selected.filter(skill => {
      const name = skill.name.toLocaleLowerCase();
      if (seen.has(name)) return true;
      seen.add(name);
      return false;
    }).map(skill => skill.name);
  }, [selected]);
  const conflicts = useMemo(
    () => selected.filter((skill) => installedNames.has(skill.name.toLocaleLowerCase())),
    [installedNames, selected],
  );
  const builtinConflicts = useMemo(
    () => selected.filter((skill) => builtinNames.has(skill.name.toLocaleLowerCase())),
    [builtinNames, selected],
  );
  const warnings = selected.flatMap((skill) => skill.warnings.map((warning) => ({ skill: skill.name, ...warning })));
  const hasBlockedWarnings = warnings.some((warning) => warning.severity === 'block');
  const canInstall = selected.length > 0
    && duplicateNames.length === 0
    && builtinConflicts.length === 0
    && (!conflicts.length || replaceExisting)
    && (!hasBlockedWarnings || acceptBlocked)
    && !busy;

  const changeSelection = (files: string[]) => {
    setSelectedFiles(files);
    setAcceptBlocked(false);
    setReplaceExisting(false);
  };

  const reset = () => {
    setSources([]);
    setPreview([]);
    setSelectedFiles([]);
    setError(null);
    setReplaceExisting(false);
    setAcceptBlocked(false);
  };

  const close = () => {
    if (busy) return;
    setOpenInstaller(false);
    reset();
  };

  const chooseSource = async (directory: boolean) => {
    setError(null);
    setBusy(true);
    try {
      const chosen = await open({
        directory,
        multiple: true,
        ...(directory
          ? {}
          : {
              filters: [{
                name: t('settings.skillInstallSupportedFiles'),
                extensions: ['skill', 'zip', 'md'],
              }],
            }),
      });
      if (!chosen) return;
      const nextSources = Array.from(new Set([...sources, ...(Array.isArray(chosen) ? chosen : [chosen])]));
      const result = await api.inspectSkillInstallSources(nextSources);
      setSources(nextSources);
      setPreview(result);
      const names = new Map<string, number>();
      for (const skill of result) names.set(skill.name.toLocaleLowerCase(), (names.get(skill.name.toLocaleLowerCase()) ?? 0) + 1);
      const previousFiles = new Set(preview.map(skill => skill.skillFile));
      setSelectedFiles(result.filter(skill => {
        if (previousFiles.has(skill.skillFile)) return selectedFiles.includes(skill.skillFile);
        const name = skill.name.toLocaleLowerCase();
        return !builtinNames.has(name) && !installedNames.has(name) && names.get(name) === 1;
      }).map(skill => skill.skillFile));
      setReplaceExisting(false);
      setAcceptBlocked(false);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
    }
  };

  const install = async () => {
    if (sources.length === 0 || !canInstall) return;
    setBusy(true);
    setError(null);
    try {
      await api.installSkillsFromSources(sources, selected.map(skill => ({ skillFile: skill.skillFile, contentDigest: skill.contentDigest })), replaceExisting, acceptBlocked);
      onInstalled?.();
      setOpenInstaller(false);
      reset();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <Button
        variant="secondary"
        size="sm"
        icon={<PackagePlus size={14} />}
        onClick={() => setOpenInstaller(true)}
      >
        {t('settings.skillInstall')}
      </Button>

      <AnimatePresence>
        {openInstaller && (
          <motion.div
            className="fixed inset-0 z-60 flex items-center justify-center bg-black/55 p-4 backdrop-blur-sm"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            onMouseDown={(event) => event.target === event.currentTarget && close()}
          >
            <motion.section
              role="dialog"
              aria-modal="true"
              aria-labelledby="skill-installer-title"
              data-testid="skill-installer"
              className="flex max-h-[min(760px,88vh)] w-full max-w-2xl flex-col overflow-hidden rounded-2xl border border-border/80 bg-surface-1 shadow-2xl"
              initial={{ opacity: 0, y: 18, scale: 0.97 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              exit={{ opacity: 0, y: 12, scale: 0.98 }}
              transition={{ type: 'spring', stiffness: 360, damping: 30 }}
            >
              <header className="flex items-start justify-between gap-4 border-b border-border bg-linear-to-r from-accent/10 via-surface-1 to-surface-1 px-5 py-4">
                <div>
                  <div className="mb-1 flex items-center gap-2 text-accent">
                    <PackagePlus size={18} />
                    <h2 id="skill-installer-title" className="text-base font-semibold text-text-primary">
                      {t('settings.skillInstallTitle')}
                    </h2>
                  </div>
                  <p className="text-xs leading-5 text-text-secondary">
                    {t('settings.skillInstallDescription')}
                  </p>
                </div>
                <button
                  type="button"
                  aria-label={t('common.close')}
                  onClick={close}
                  className="rounded-lg p-1.5 text-text-tertiary transition hover:bg-surface-3 hover:text-text-primary"
                >
                  <X size={16} />
                </button>
              </header>

              <div className="flex-1 space-y-4 overflow-y-auto p-5">
                <div className="grid gap-3 sm:grid-cols-2">
                  <button
                    type="button"
                    onClick={() => chooseSource(false)}
                    disabled={busy}
                    className="group rounded-xl border border-border bg-surface-2 p-4 text-left transition hover:-translate-y-0.5 hover:border-accent/50 hover:bg-accent/5 disabled:opacity-50"
                  >
                    <FileArchive className="mb-3 text-accent transition-transform group-hover:scale-110" size={22} />
                    <p className="text-sm font-medium text-text-primary">{t('settings.skillInstallPackage')}</p>
                    <p className="mt-1 text-xs text-text-tertiary">.skill, .zip, SKILL.md</p>
                  </button>
                  <button
                    type="button"
                    onClick={() => chooseSource(true)}
                    disabled={busy}
                    className="group rounded-xl border border-border bg-surface-2 p-4 text-left transition hover:-translate-y-0.5 hover:border-accent/50 hover:bg-accent/5 disabled:opacity-50"
                  >
                    <FolderOpen className="mb-3 text-accent transition-transform group-hover:scale-110" size={22} />
                    <p className="text-sm font-medium text-text-primary">{t('settings.skillInstallFolder')}</p>
                    <p className="mt-1 text-xs text-text-tertiary">{t('settings.skillInstallFolderHint')}</p>
                  </button>
                </div>

                {busy && preview.length === 0 && (
                  <div className="flex items-center justify-center gap-2 rounded-xl border border-border bg-surface-2 py-8 text-sm text-text-secondary">
                    <Loader2 size={16} className="animate-spin text-accent" />
                    {t('settings.skillInstallInspecting')}
                  </div>
                )}

                {sources.length > 0 && preview.length > 0 && (
                  <div className="space-y-3" data-testid="skill-install-preview">
                    <div className="flex items-center gap-2 text-xs text-text-tertiary">
                      <CheckCircle2 size={14} className="text-success" />
                      <span className="truncate" title={sources.join('\n')}>{t('settings.skillInstallSources', { count: String(sources.length) })}</span>
                      <Button size="sm" variant="ghost" disabled={busy} onClick={reset}>{t('common.clear')}</Button>
                    </div>
                    <div className="flex flex-wrap items-center gap-2 text-xs text-text-secondary">
                      <span>{t('settings.skillInstallSelection', { selected: String(selected.length), total: String(preview.length) })}</span>
                      <Button size="sm" variant="ghost" disabled={busy} onClick={() => changeSelection(preview.filter(skill => !builtinNames.has(skill.name.toLocaleLowerCase())).map(skill => skill.skillFile))}>{t('settings.skillInstallSelectAll')}</Button>
                      <Button size="sm" variant="ghost" disabled={busy} onClick={() => changeSelection([])}>{t('settings.skillInstallSelectNone')}</Button>
                    </div>
                    {preview.map((skill) => {
                      const conflict = installedNames.has(skill.name.toLocaleLowerCase());
                      const builtin = builtinNames.has(skill.name.toLocaleLowerCase());
                      return (
                        <article key={skill.skillFile} className="rounded-xl border border-border bg-surface-2 p-3">
                          <div className="flex items-start justify-between gap-3">
                            <label className="flex min-w-0 gap-3">
                              <input type="checkbox" checked={selectedFiles.includes(skill.skillFile)} disabled={busy || builtin} aria-label={skill.name}
                                onChange={event => changeSelection(event.target.checked ? [...selectedFiles, skill.skillFile] : selectedFiles.filter(file => file !== skill.skillFile))}
                                className="mt-1 shrink-0 accent-accent" />
                              <div className="min-w-0">
                              <p className="truncate text-sm font-semibold text-text-primary">{skill.name}</p>
                              <p className="mt-0.5 text-xs text-text-secondary">{skill.description}</p>
                              <p className="mt-1 truncate text-[11px] text-text-tertiary" title={skill.skillFile}>{skill.skillFile}</p>
                              {builtin && <p className="mt-1 text-xs text-warning">{t('settings.skillInstallBuiltinConflict', { names: skill.name })}</p>}
                              </div>
                            </label>
                            <div className="flex shrink-0 gap-1">
                              {conflict && <Badge variant="default">{t('settings.skillInstallUpdate')}</Badge>}
                              <Badge variant="default">{t('settings.skillInstallResourceCount', { count: String(skill.resources.length) })}</Badge>
                            </div>
                          </div>
                        </article>
                      );
                    })}
                  </div>
                )}

                {duplicateNames.length > 0 && <div role="alert" className="rounded-xl border border-warning/35 bg-warning/8 p-3 text-xs text-warning">{t('settings.skillInstallDuplicate', { names: duplicateNames.join(', ') })}</div>}

                {conflicts.length > 0 && (
                  <label className="flex cursor-pointer items-start gap-3 rounded-xl border border-warning/35 bg-warning/8 p-3 text-xs text-text-secondary">
                    <input
                      type="checkbox"
                      checked={replaceExisting}
                      onChange={(event) => setReplaceExisting(event.target.checked)}
                      className="mt-0.5 accent-accent"
                    />
                    <span>
                      <span className="block font-medium text-text-primary">{t('settings.skillInstallReplaceTitle')}</span>
                      {t('settings.skillInstallReplaceDescription', { names: conflicts.map((skill) => skill.name).join(', ') })}
                    </span>
                  </label>
                )}

                {builtinConflicts.length > 0 && (
                  <div role="alert" className="flex items-start gap-2 rounded-xl border border-danger/35 bg-danger/8 p-3 text-xs text-danger">
                    <ShieldAlert size={15} className="mt-0.5 shrink-0" />
                    {t('settings.skillInstallBuiltinConflict', { names: builtinConflicts.map((skill) => skill.name).join(', ') })}
                  </div>
                )}

                {warnings.length > 0 && (
                  <div className="rounded-xl border border-warning/35 bg-warning/8 p-3">
                    <div className="mb-2 flex items-center gap-2 text-xs font-medium text-warning">
                      <AlertTriangle size={14} />
                      {t('settings.skillInstallWarnings', { count: String(warnings.length) })}
                    </div>
                    <ul className="max-h-36 space-y-1 overflow-y-auto text-xs text-text-secondary">
                      {warnings.map((warning, index) => (
                        <li key={`${warning.skill}-${warning.code}-${index}`} className="flex gap-2">
                          <span className={warning.severity === 'block' ? 'text-danger' : 'text-warning'}>•</span>
                          <span><strong>{warning.skill}</strong>: {warning.message}</span>
                        </li>
                      ))}
                    </ul>
                    {hasBlockedWarnings && (
                      <label className="mt-3 flex cursor-pointer items-start gap-2 border-t border-warning/25 pt-3 text-xs text-text-secondary">
                        <input
                          type="checkbox"
                          checked={acceptBlocked}
                          onChange={(event) => setAcceptBlocked(event.target.checked)}
                          className="mt-0.5 accent-danger"
                        />
                        <ShieldAlert size={14} className="shrink-0 text-danger" />
                        {t('settings.skillInstallRiskConfirm')}
                      </label>
                    )}
                  </div>
                )}

                {error && (
                  <div role="alert" className="rounded-xl border border-danger/30 bg-danger/8 px-3 py-2 text-xs text-danger">
                    {error}
                  </div>
                )}
              </div>

              <footer className="flex items-center justify-end gap-2 border-t border-border bg-surface-2/70 px-5 py-3">
                <Button variant="ghost" size="sm" onClick={close}>{t('common.cancel')}</Button>
                <Button
                  variant="primary"
                  size="sm"
                  loading={busy && preview.length > 0}
                  disabled={!canInstall}
                  onClick={install}
                >
                  {t('settings.skillInstallConfirm', { count: String(selected.length) })}
                </Button>
              </footer>
            </motion.section>
          </motion.div>
        )}
      </AnimatePresence>
    </>
  );
}
