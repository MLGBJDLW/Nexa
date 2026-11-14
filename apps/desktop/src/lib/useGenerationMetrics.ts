import { useState } from 'react';
import type { ContextUsageBreakdown } from '../types/conversation';
import { retainGenerationSample } from './contextUsage';

export function useGenerationMetrics(scope: string, observed: ContextUsageBreakdown['generation']) {
  const [sample, setSample] = useState(() => retainGenerationSample(null, scope, observed));
  const next = retainGenerationSample(sample, scope, observed);
  // Adjust before rendering children so a model switch never paints stale TPS.
  if (next !== sample) setSample(next);
  return next.metrics;
}
