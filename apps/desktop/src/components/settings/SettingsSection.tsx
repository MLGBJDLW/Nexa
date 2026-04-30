import { useState, type ReactNode } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { ChevronDown } from 'lucide-react';
import { useTranslation } from '../../i18n';

interface SectionProps {
  icon: ReactNode;
  title: string;
  children: ReactNode;
  delay?: number;
}

export function Section({ icon, title, children, delay = 0 }: SectionProps) {
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

export function StatCard({ label, value }: { label: string; value: number | string }) {
  return (
    <div className="rounded-lg bg-surface-2 px-4 py-3">
      <p className="text-xs text-text-tertiary">{label}</p>
      <p className="mt-1 text-xl font-bold text-text-primary">{value}</p>
    </div>
  );
}

interface CollapsiblePanelProps {
  title: string;
  description?: string;
  children: ReactNode;
  defaultOpen?: boolean;
}

export function CollapsiblePanel({
  title,
  description,
  children,
  defaultOpen = false,
}: CollapsiblePanelProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(defaultOpen);

  return (
    <div className="overflow-hidden rounded-lg border border-border bg-surface-1">
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        aria-expanded={open}
        aria-label={open ? t('common.collapse') : t('common.expand')}
        className="flex w-full items-start justify-between gap-3 px-4 py-3 text-left transition-colors hover:bg-surface-2/70"
      >
        <div className="min-w-0">
          <h4 className="text-sm font-medium text-text-primary">{title}</h4>
          {description && (
            <p className="mt-1 text-xs leading-relaxed text-text-tertiary">{description}</p>
          )}
        </div>
        <ChevronDown
          size={16}
          className={`mt-0.5 shrink-0 text-text-tertiary transition-transform ${open ? 'rotate-180' : ''}`}
        />
      </button>
      <AnimatePresence initial={false}>
        {open && (
          <motion.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: 'auto', opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.18, ease: [0.16, 1, 0.3, 1] }}
            className="overflow-hidden"
          >
            <div className="border-t border-border px-4 py-4">
              {children}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
