import './updateActivity.css';

interface UpdateActivityProps {
  status: 'checking' | 'downloading' | 'installing' | 'ready';
  progress?: number;
  className?: string;
}

/** Shared download glyph for updates, models and runtime tools. */
export function UpdateActivity({ status, progress, className = '' }: UpdateActivityProps) {
  const percent = typeof progress === 'number' && Number.isFinite(progress)
    ? Math.max(0, Math.min(100, progress)) : undefined;
  const determinate = status === 'downloading' && percent !== undefined;
  return (
    <span className={`update-activity ${className}`} data-phase={status}
      data-determinate={determinate} aria-hidden="true" data-testid="update-activity">
      <svg viewBox="0 0 32 32" fill="none" className="update-activity-ring">
        <circle cx="16" cy="16" r="13" stroke="currentColor" strokeWidth="2" opacity="0.14" />
        {determinate ? (
          <circle className="update-activity-progress" cx="16" cy="16" r="13" pathLength="100"
            stroke="currentColor" strokeWidth="2.4" strokeLinecap="round"
            strokeDasharray={`${percent} 100`} transform="rotate(-90 16 16)" />
        ) : status === 'ready' ? (
          <circle cx="16" cy="16" r="13" stroke="currentColor" strokeWidth="2" opacity="0.45" />
        ) : null}
      </svg>
      {!determinate && status !== 'ready' && (
        <span className="update-activity-layer update-activity-orbit">
          <svg viewBox="0 0 32 32" fill="none">
            <path d="M16 3a13 13 0 0 1 12.36 9" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" />
            <path d="M16 29A13 13 0 0 1 3.64 20" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" opacity="0.45" />
          </svg>
        </span>
      )}
      {status === 'installing' || status === 'ready' ? (
        <svg className={`update-activity-glyph ${status === 'ready' ? 'update-activity-check' : 'update-activity-package'}`}
          viewBox="0 0 32 32" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
          {status === 'ready' ? <path d="m10.5 16 3.7 3.7 7.3-7.4" />
            : <path d="m10 13 6-3 6 3v7l-6 3-6-3zM10 13l6 3 6-3M16 16v7" />}
        </svg>
      ) : (
        <svg className="update-activity-glyph" viewBox="0 0 32 32" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinejoin="round" strokeLinecap="round">
          <path d="m9 18 7-3 7 3-7 3z" opacity=".45" />
          <svg x="9" y="5" width="14" height="14" viewBox="0 0 14 14" overflow="hidden">
            <path className="update-activity-arrow" d="M7 2v8m-3-3 3 3 3-3" />
          </svg>
          <path d="m9 18 7 3 7-3v5l-7 3-7-3zM16 21v5" />
        </svg>
      )}
    </span>
  );
}


export function DownloadProgressBar({ progress, label }: { progress?: number; label: string }) {
  const percent = typeof progress === 'number' && Number.isFinite(progress)
    ? Math.min(100, Math.max(0, Math.round(progress))) : undefined;
  return (
    <div className="flex items-center gap-3">
      <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-surface-3" role="progressbar" aria-label={label}
        aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent}>
        <div className={percent === undefined ? 'download-bar-pending' : 'h-full origin-left rounded-full bg-accent transition-transform duration-300 motion-reduce:transition-none'}
          style={percent === undefined ? undefined : { transform: `scaleX(${percent / 100})` }} />
      </div>
      {percent !== undefined && <span className="text-xs tabular-nums text-text-tertiary">{percent}%</span>}
    </div>
  );
}
