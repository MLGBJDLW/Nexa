import { type MouseEvent } from 'react';
import { Tooltip } from './Tooltip';
import { openFileInDefaultApp, showInFileExplorer } from '../../lib/api';
import { canPreviewInApp, useFilePreview } from '../../features/preview';
import { resolveFileBadgeIcon, type FileBadgeTone } from './fileBadgeCatalog';

interface FileBadgeProps {
  path: string;
  className?: string;
}

const colors = {
  red: 'text-red-400', rose: 'text-rose-400', blue: 'text-info', sky: 'text-sky-400',
  cyan: 'text-cyan-400', teal: 'text-teal-400', green: 'text-success', emerald: 'text-emerald-400',
  orange: 'text-orange-400', amber: 'text-amber-400', yellow: 'text-yellow-400', purple: 'text-purple-400',
  fuchsia: 'text-fuchsia-400', pink: 'text-pink-400', violet: 'text-violet-400', indigo: 'text-indigo-400',
  slate: 'text-slate-400', gray: 'text-text-secondary',
} satisfies Record<FileBadgeTone, string>;

function isDirectory(path: string): boolean {
  return path.endsWith('/') || path.endsWith('\\');
}

export function isAbsoluteFileSystemPath(path: string): boolean {
  const trimmed = path.trim();
  if (!trimmed) return false;
  if (trimmed.startsWith('/')) return true;
  if (/^[A-Za-z]:[\\/]/.test(trimmed)) return true;
  return /^\\\\[^\\/]+[\\/][^\\/]+/.test(trimmed);
}

function basename(path: string): string {
  const normalized = path.replace(/[\\/]+$/, '');
  const lastSep = Math.max(normalized.lastIndexOf('/'), normalized.lastIndexOf('\\'));
  return lastSep === -1 ? normalized : normalized.slice(lastSep + 1);
}

export function FileBadge({ path, className = '' }: FileBadgeProps) {
  const { openFilePreview, remote } = useFilePreview();
  const safePath = path.trim();
  if (!isAbsoluteFileSystemPath(safePath)) return null;

  const dir = isDirectory(safePath);
  const name = basename(safePath);
  const { tone, Icon, iconId, treatment, brandColor, accentColor } = resolveFileBadgeIcon(name, dir);
  const color = colors[tone];

  const handleClick = (e: MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    if (remote) {
      openFilePreview(safePath);
    } else if (e.altKey) {
      showInFileExplorer(safePath);
    } else if (!dir && canPreviewInApp(safePath) && !e.ctrlKey && !e.metaKey && !e.shiftKey) {
      openFilePreview(safePath);
    } else {
      openFileInDefaultApp(safePath);
    }
  };

  const handleContextMenu = (e: MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    if (remote) openFilePreview(safePath); else showInFileExplorer(safePath);
  };

  return (
    <Tooltip content={safePath} side="top">
      <button
        type="button"
        data-file-icon={iconId}
        data-file-treatment={treatment}
        onClick={handleClick}
        onContextMenu={handleContextMenu}
        className={`
          inline-flex max-w-full items-center gap-1.5 px-2 py-0.5 text-[11px] font-medium
          rounded-md border cursor-pointer transition-colors duration-150
          file-badge-shell focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent
          ${color}
          ${className}
        `}
      >
        <span className="relative inline-flex h-[13px] w-[13px] shrink-0 items-center justify-center" aria-hidden="true">
          <Icon
            size={13}
            className="file-badge-icon h-[13px] w-[13px]"
            style={brandColor ? { color: brandColor } : undefined}
          />
          {accentColor ? (
            <span
              data-file-icon-accent="true"
              className="file-badge-icon-accent absolute -bottom-px -right-px h-1.5 w-1.5 rounded-full border border-surface-0"
              style={{ backgroundColor: accentColor }}
            />
          ) : null}
        </span>
        <span className="truncate max-w-[200px] text-text-secondary">{name}</span>
      </button>
    </Tooltip>
  );
}
