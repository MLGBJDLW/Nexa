import { useState, useEffect, useCallback } from 'react';
import { motion } from 'framer-motion';
import {
  Database,
  Shield,
  RefreshCw,
  Zap,
  Plus,
  Trash2,
  Save,
  Languages,
} from 'lucide-react';
import { toast } from 'sonner';
import * as api from '../lib/api';
import type { IndexStats } from '../types/index-stats';
import type { PrivacyConfig, RedactRule } from '../types/privacy';
import { useTranslation } from '../i18n';
import { Button } from '../components/ui/Button';
import { Input } from '../components/ui/Input';
import { Badge } from '../components/ui/Badge';

/* ── Section wrapper ──────────────────────────────────────────────── */
function Section({
  icon,
  title,
  children,
  delay = 0,
}: {
  icon: React.ReactNode;
  title: string;
  children: React.ReactNode;
  delay?: number;
}) {
  return (
    <motion.section
      initial={{ opacity: 0, y: 12 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.3, delay, ease: [0.16, 1, 0.3, 1] }}
      className="rounded-xl border border-border bg-surface-1 p-6"
    >
      <div className="mb-5 flex items-center gap-2.5">
        <span className="text-accent">{icon}</span>
        <h2 className="text-base font-semibold text-text-primary">{title}</h2>
      </div>
      {children}
    </motion.section>
  );
}

/* ── Stat card ────────────────────────────────────────────────────── */
function StatCard({ label, value }: { label: string; value: number | string }) {
  return (
    <div className="rounded-lg bg-surface-2 px-4 py-3">
      <p className="text-xs text-text-tertiary">{label}</p>
      <p className="mt-1 text-xl font-bold text-text-primary">{value}</p>
    </div>
  );
}

/* ── Settings page ────────────────────────────────────────────────── */
export function SettingsPage() {
  const { t, locale, setLocale, availableLocales } = useTranslation();
  /* ── Index state ─────────────────────────────────────────────────── */
  const [stats, setStats] = useState<IndexStats | null>(null);
  const [rebuildLoading, setRebuildLoading] = useState(false);
  const [optimizeLoading, setOptimizeLoading] = useState(false);

  const loadStats = useCallback(() => {
    api.getIndexStats().then(setStats).catch(() => {
      toast.error(t('settings.loadStatsError'));
    });
  }, []);

  useEffect(() => {
    loadStats();
  }, [loadStats]);

  const handleRebuild = async () => {
    setRebuildLoading(true);
    try {
      await api.rebuildIndex();
      toast.success(t('settings.indexRebuilt'));
      loadStats();
    } catch {
      toast.error(t('settings.indexRebuildError'));
    } finally {
      setRebuildLoading(false);
    }
  };

  const handleOptimize = async () => {
    setOptimizeLoading(true);
    try {
      await api.optimizeFtsIndex();
      toast.success(t('settings.ftsOptimized'));
    } catch {
      toast.error(t('settings.ftsOptimizeError'));
    } finally {
      setOptimizeLoading(false);
    }
  };

  /* ── Privacy state ───────────────────────────────────────────────── */
  const [privacyConfig, setPrivacyConfig] = useState<PrivacyConfig | null>(null);
  const [newPattern, setNewPattern] = useState('');
  const [newRule, setNewRule] = useState<RedactRule>({ name: '', pattern: '', replacement: '' });
  const [saveLoading, setSaveLoading] = useState(false);

  useEffect(() => {
    api.getPrivacyConfig().then(setPrivacyConfig).catch(() => {
      toast.error(t('settings.loadPrivacyError'));
    });
  }, []);

  const addPattern = () => {
    const trimmed = newPattern.trim();
    if (!trimmed || !privacyConfig) return;
    if (privacyConfig.excludePatterns.includes(trimmed)) {
      toast.error(t('settings.patternExists'));
      return;
    }
    setPrivacyConfig({
      ...privacyConfig,
      excludePatterns: [...privacyConfig.excludePatterns, trimmed],
    });
    setNewPattern('');
  };

  const removePattern = (idx: number) => {
    if (!privacyConfig) return;
    setPrivacyConfig({
      ...privacyConfig,
      excludePatterns: privacyConfig.excludePatterns.filter((_, i) => i !== idx),
    });
  };

  const addRule = () => {
    if (!newRule.name.trim() || !newRule.pattern.trim() || !privacyConfig) return;
    setPrivacyConfig({
      ...privacyConfig,
      redactPatterns: [...privacyConfig.redactPatterns, { ...newRule }],
    });
    setNewRule({ name: '', pattern: '', replacement: '' });
  };

  const removeRule = (idx: number) => {
    if (!privacyConfig) return;
    setPrivacyConfig({
      ...privacyConfig,
      redactPatterns: privacyConfig.redactPatterns.filter((_, i) => i !== idx),
    });
  };

  const handleSavePrivacy = async () => {
    if (!privacyConfig) return;
    setSaveLoading(true);
    try {
      await api.savePrivacyConfig(privacyConfig);
      toast.success(t('settings.privacySaved'));
    } catch {
      toast.error(t('settings.privacySaveError'));
    } finally {
      setSaveLoading(false);
    }
  };

  /* ── Render ──────────────────────────────────────────────────────── */
  return (
    <div className="mx-auto max-w-3xl space-y-6 p-6">
      {/* Header */}
      <motion.div
        initial={{ opacity: 0, y: -8 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.25 }}
      >
        <h1 className="text-xl font-bold text-text-primary">{t('settings.title')}</h1>
        <p className="mt-1 text-sm text-text-secondary">{t('settings.subtitle')}</p>
      </motion.div>

      {/* ── Section 1: 索引管理 ──────────────────────────────────────── */}
      <Section icon={<Database size={20} />} title={t('settings.indexSection')} delay={0.05}>
        {/* Stats grid */}
        <div className="mb-5 grid grid-cols-3 gap-3">
          <StatCard label={t('settings.totalDocs')} value={stats?.totalDocuments ?? '—'} />
          <StatCard label={t('settings.totalChunks')} value={stats?.totalChunks ?? '—'} />
          <StatCard label={t('settings.ftsEntries')} value={stats?.ftsRows ?? '—'} />
        </div>

        {/* Actions */}
        <div className="flex items-center gap-3">
          <Button
            variant="secondary"
            size="sm"
            icon={<RefreshCw size={14} />}
            loading={rebuildLoading}
            onClick={handleRebuild}
          >
            {t('settings.rebuildIndex')}
          </Button>
          <Button
            variant="secondary"
            size="sm"
            icon={<Zap size={14} />}
            loading={optimizeLoading}
            onClick={handleOptimize}
          >
            {t('settings.optimizeIndex')}
          </Button>
        </div>
      </Section>

      {/* ── Section 2: 隐私配置 ──────────────────────────────────────── */}
      <Section icon={<Shield size={20} />} title={t('settings.privacySection')} delay={0.1}>
        {privacyConfig && (
          <div className="space-y-6">
            {/* Exclude patterns */}
            <div>
              <h3 className="mb-2 text-sm font-medium text-text-primary">{t('settings.excludePatterns')}</h3>
              <p className="mb-3 text-xs text-text-tertiary">{t('settings.excludePatternsDesc')}</p>

              {/* Pattern chips */}
              {privacyConfig.excludePatterns.length > 0 && (
                <div className="mb-3 flex flex-wrap gap-2">
                  {privacyConfig.excludePatterns.map((pat, i) => (
                    <Badge key={i} variant="default" className="gap-1.5 pl-2.5 pr-1.5 py-1">
                      <span className="font-mono text-[11px]">{pat}</span>
                      <button
                        onClick={() => removePattern(i)}
                        className="ml-0.5 rounded hover:bg-surface-4 p-0.5 text-text-tertiary hover:text-danger transition-colors cursor-pointer"
                        aria-label={`${t('common.remove')} ${pat}`}
                      >
                        <Trash2 size={12} />
                      </button>
                    </Badge>
                  ))}
                </div>
              )}

              {/* Add pattern */}
              <div className="flex gap-2">
                <Input
                  placeholder="*.log, .git/**, node_modules/**"
                  value={newPattern}
                  onChange={(e) => setNewPattern(e.target.value)}
                  onKeyDown={(e) => { if (e.key === 'Enter') addPattern(); }}
                  className="flex-1"
                />
                <Button
                  variant="ghost"
                  size="md"
                  icon={<Plus size={16} />}
                  onClick={addPattern}
                  disabled={!newPattern.trim()}
                >
                  {t('settings.addPattern')}
                </Button>
              </div>
            </div>

            {/* Redaction rules */}
            <div>
              <h3 className="mb-2 text-sm font-medium text-text-primary">{t('settings.redactRules')}</h3>
              <p className="mb-3 text-xs text-text-tertiary">{t('settings.redactRulesDesc')}</p>

              {/* Rules table */}
              {privacyConfig.redactPatterns.length > 0 && (
                <div className="mb-3 overflow-hidden rounded-lg border border-border">
                  <table className="w-full text-sm">
                    <thead>
                      <tr className="border-b border-border bg-surface-2">
                        <th className="px-3 py-2 text-left text-xs font-medium text-text-tertiary">{t('settings.ruleName')}</th>
                        <th className="px-3 py-2 text-left text-xs font-medium text-text-tertiary">{t('settings.rulePattern')}</th>
                        <th className="px-3 py-2 text-left text-xs font-medium text-text-tertiary">{t('settings.ruleReplacement')}</th>
                        <th className="w-10 px-3 py-2" />
                      </tr>
                    </thead>
                    <tbody>
                      {privacyConfig.redactPatterns.map((rule, i) => (
                        <tr
                          key={i}
                          className="border-b border-border last:border-0 hover:bg-surface-2/50 transition-colors"
                        >
                          <td className="px-3 py-2 text-text-primary">{rule.name}</td>
                          <td className="px-3 py-2 font-mono text-xs text-text-secondary">{rule.pattern}</td>
                          <td className="px-3 py-2 font-mono text-xs text-text-secondary">{rule.replacement}</td>
                          <td className="px-3 py-2 text-right">
                            <button
                              onClick={() => removeRule(i)}
                              className="rounded p-1 text-text-tertiary hover:text-danger hover:bg-danger/10 transition-colors cursor-pointer"
                              aria-label={`${t('common.delete')} ${rule.name}`}
                            >
                              <Trash2 size={14} />
                            </button>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}

              {/* Add rule form */}
              <div className="flex gap-2">
                <Input
                  placeholder={t('settings.ruleName')}
                  value={newRule.name}
                  onChange={(e) => setNewRule({ ...newRule, name: e.target.value })}
                  className="flex-1"
                />
                <Input
                  placeholder={t('settings.rulePattern')}
                  value={newRule.pattern}
                  onChange={(e) => setNewRule({ ...newRule, pattern: e.target.value })}
                  className="flex-1"
                />
                <Input
                  placeholder={t('settings.ruleReplacement')}
                  value={newRule.replacement}
                  onChange={(e) => setNewRule({ ...newRule, replacement: e.target.value })}
                  className="flex-1"
                />
                <Button
                  variant="ghost"
                  size="md"
                  icon={<Plus size={16} />}
                  onClick={addRule}
                  disabled={!newRule.name.trim() || !newRule.pattern.trim()}
                >
                  {t('settings.addRule')}
                </Button>
              </div>
            </div>

            {/* Save button */}
            <div className="flex justify-end border-t border-border pt-4">
              <Button
                variant="primary"
                size="md"
                icon={<Save size={16} />}
                loading={saveLoading}
                onClick={handleSavePrivacy}
              >
                {t('settings.saveConfig')}
              </Button>
            </div>
          </div>
        )}
      </Section>

      {/* ── Section 3: Language ──────────────────────────────────────── */}
      <Section icon={<Languages size={20} />} title={t('settings.languageSection')} delay={0.15}>
        <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-5 gap-2">
          {availableLocales.map((l) => (
            <button
              key={l.code}
              onClick={() => setLocale(l.code)}
              className={`rounded-lg border px-3 py-2.5 text-sm font-medium transition-all duration-fast cursor-pointer ${
                locale === l.code
                  ? 'border-accent bg-accent-subtle text-accent ring-1 ring-accent/20'
                  : 'border-border bg-surface-2 text-text-secondary hover:border-border-hover hover:bg-surface-3'
              }`}
            >
              {l.name}
            </button>
          ))}
        </div>
      </Section>
    </div>
  );
}
