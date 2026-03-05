import { Moon, Sun, Star } from 'lucide-react';
import { motion } from 'framer-motion';
import type { LucideProps } from 'lucide-react';
import { useTheme } from '../../lib/ThemeProvider';
import { THEMES, type ThemeId } from '../../lib/theme';

const ICON_MAP: Record<ThemeId, React.ComponentType<LucideProps>> = {
  dark: Moon,
  light: Sun,
  midnight: Star,
};

interface ThemeSwitcherProps {
  /** Show labels next to icons (default: true) */
  showLabels?: boolean;
}

export function ThemeSwitcher({ showLabels = true }: ThemeSwitcherProps) {
  const { theme, setTheme } = useTheme();

  return (
    <div className="flex gap-1 rounded-lg bg-surface-2 p-1">
      {THEMES.map((t) => {
        const Icon = ICON_MAP[t.id];
        const isActive = theme === t.id;
        return (
          <button
            key={t.id}
            onClick={() => setTheme(t.id)}
            className={`relative flex items-center justify-center gap-1.5 rounded-md px-3 py-1.5 text-xs font-medium
              transition-colors duration-150 cursor-pointer
              ${isActive ? 'text-text-inverse' : 'text-text-secondary hover:text-text-primary'}`}
            aria-label={t.label}
          >
            {isActive && (
              <motion.span
                layoutId="theme-indicator"
                className="absolute inset-0 rounded-md bg-accent"
                transition={{ type: 'spring', stiffness: 400, damping: 30 }}
              />
            )}
            <span className="relative flex items-center gap-1.5">
              <Icon size={14} />
              {showLabels && <span>{t.label}</span>}
            </span>
          </button>
        );
      })}
    </div>
  );
}

export default ThemeSwitcher;
