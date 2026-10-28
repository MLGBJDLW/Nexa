import './updateActivity.css';

interface UpdateActivityProps {
  status: 'checking' | 'downloading' | 'ready';
  progress?: number;
  className?: string;
}

/** One small motion surface shared by the sidebar and update details. */
export function UpdateActivity({ status, progress, className = '' }: UpdateActivityProps) {
  const percent = typeof progress === 'number' && Number.isFinite(progress)
    ? Math.max(0, Math.min(100, progress)) : undefined;
  const phase = status === 'downloading' && percent === 100 ? 'installing' : status;
  const determinate = status === 'downloading' && percent !== undefined && percent < 100;
  return (
    <span className={`update-activity ${className}`} data-phase={phase}
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
      <span className={`update-activity-layer ${status === 'ready' ? 'update-activity-check' : phase === 'installing' ? 'update-activity-package' : 'update-activity-arrow'}`}>
        <svg viewBox="0 0 32 32" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
          {status === 'ready' ? <path d="m10.5 16 3.7 3.7 7.3-7.4" /> : phase === 'installing'
            ? <path d="m10 13 6-3 6 3v7l-6 3-6-3zM10 13l6 3 6-3M16 16v7" />
            : <path d="M16 9v10m-4-4 4 4 4-4" />}
        </svg>
      </span>
      {status !== 'ready' && phase !== 'installing' && (
        <svg className="update-activity-glyph" viewBox="0 0 32 32" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
          <path d="M10 21v2h12v-2" opacity="0.65" />
        </svg>
      )}
    </span>
  );
}
