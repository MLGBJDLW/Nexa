import type { CSSProperties } from 'react';
import { useTranslation } from '../../i18n';
import './imageGenerationActivity.css';

const NODES = [
  [28, 92], [42, 172], [70, 48], [88, 132], [98, 224],
  [142, 72], [154, 180], [194, 120], [208, 42], [216, 230],
  [280, 136],
  [344, 42], [352, 230], [366, 120], [406, 180], [418, 72],
  [462, 224], [472, 132], [490, 48], [518, 172], [532, 92],
];
const EDGES = [
  [0, 2], [0, 3], [1, 3], [1, 4], [2, 5], [3, 5], [3, 6], [4, 6],
  [5, 7], [5, 8], [6, 7], [6, 9], [7, 10], [8, 10], [9, 10],
  [10, 11], [10, 12], [10, 13], [11, 15], [12, 14], [13, 14], [13, 15],
  [14, 16], [14, 17], [15, 17], [15, 18], [16, 19], [17, 19], [17, 20], [18, 20],
];

function aspectRatio(size = ''): string {
  const match = size.match(/^(\d{1,5})\s*[x*:]\s*(\d{1,5})$/i);
  return match && Number(match[1]) > 0 && Number(match[2]) > 0 ? `${match[1]} / ${match[2]}` : '16 / 10';
}

/** Decorative, indeterminate activity. The image APIs do not report a percent.
 * Fixed SVG geometry and CSS animation avoid per-frame React work or timers. */
export function ImageGenerationActivity({ prompt, size, compact = false, active = true, label }: {
  prompt?: string;
  size?: string;
  compact?: boolean;
  active?: boolean;
  label?: string;
}) {
  const { t } = useTranslation();
  return <div
    data-testid="image-generation-activity"
    data-active={active}
    data-compact={compact}
    className="image-generation-activity"
    style={{ aspectRatio: aspectRatio(size) }}
    role="status"
    aria-label={label ?? t('chat.generatedImageLoading')}
  >
    <svg className="image-neural-network" viewBox="0 0 560 272" fill="none" aria-hidden="true">
      <g className="image-neural-guides">{EDGES.map(([a, b], i) => <path key={i} d={`M${NODES[a].join(',')} L${NODES[b].join(',')}`} />)}</g>
      <g className="image-neural-links">{EDGES.map(([a, b], i) => <path key={i}
        d={`M${NODES[a].join(',')} L${NODES[b].join(',')}`} pathLength="1"
        style={{ '--neural-delay': `${i * 90}ms` } as CSSProperties} />)}</g>
      <g className="image-neural-nodes">{NODES.map(([cx, cy], i) => <circle key={i} cx={cx} cy={cy} r={i === 10 ? 7 : 3}
        style={{ '--neural-delay': `${i * 130}ms` } as CSSProperties} />)}</g>
      <circle className="image-neural-core" cx="280" cy="136" r="22" />
      <circle className="image-neural-core image-neural-core-outer" cx="280" cy="136" r="34" />
    </svg>
    <div className="image-generation-caption" aria-hidden="true">
      <span className="image-generation-label"><span className="image-generation-dot" />{label ?? t('chat.generatedImageLoading')}</span>
      {!compact && prompt && <span className="image-generation-prompt">{prompt}</span>}
      <span className="image-generation-track"><span /></span>
    </div>
  </div>;
}
