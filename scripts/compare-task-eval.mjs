import { readFileSync } from 'node:fs';

export function compareTaskEval(baseline, candidate) {
  for (const field of ['schemaVersion', 'mode', 'scoreKind', 'corpusDigest', 'model', 'provider', 'apiStyle', 'reasoningEnabled', 'providerEndpointId', 'nodeVersion', 'repetitions', 'maxTokensPerTask', 'maxOutputTokensPerRequest', 'timeoutSeconds']) {
    if (JSON.stringify(baseline[field]) !== JSON.stringify(candidate[field])) throw new Error(`Incomparable reports: ${field} differs`);
  }
  if (baseline.sourceDirty || candidate.sourceDirty) throw new Error('A version baseline requires clean source trees; dirty development runs are diagnostics only');
  const byKey = new Map(baseline.cases.map(item => [`${item.taskId}/${item.repetition}`, item]));
  if (byKey.size !== baseline.cases.length || candidate.cases.length !== baseline.cases.length) throw new Error('Incomparable task sets');
  const rows = candidate.cases.map(item => {
    const key = `${item.taskId}/${item.repetition}`;
    const old = byKey.get(key);
    if (!old) throw new Error(`Missing or duplicate baseline task: ${key}`);
    byKey.delete(key);
    if (JSON.stringify(old.availableTools) !== JSON.stringify(item.availableTools) || old.contextWindowTokens !== item.contextWindowTokens || old.inputImages !== item.inputImages || old.costScope !== item.costScope) throw new Error(`Incomparable task configuration: ${key}`);
    return {
      task: key, before: old.passed, after: item.passed,
      elapsedDeltaMs: item.elapsedMs - old.elapsedMs,
      invocationDelta: item.providerInvocations.length - old.providerInvocations.length,
      reportedUsageCostDelta: old.reportedUsageCost === null || item.reportedUsageCost === null || JSON.stringify(baseline.prices) !== JSON.stringify(candidate.prices) ? null : item.reportedUsageCost - old.reportedUsageCost,
    };
  });
  if (byKey.size) throw new Error('Candidate omitted baseline tasks');
  return { scoreKind: candidate.scoreKind, baselineSha: baseline.sourceSha, candidateSha: candidate.sourceSha, passedBefore: baseline.passed, passedAfter: candidate.passed, total: candidate.total, regressions: rows.filter(item => item.before && !item.after).map(item => item.task), cases: rows };
}

if (process.argv[1]?.endsWith('compare-task-eval.mjs')) {
  const [before, after] = process.argv.slice(2);
  if (!before || !after) throw new Error('Usage: node scripts/compare-task-eval.mjs BASELINE.json CANDIDATE.json');
  const report = compareTaskEval(JSON.parse(readFileSync(before, 'utf8')), JSON.parse(readFileSync(after, 'utf8')));
  console.log(JSON.stringify(report, null, 2));
  if (report.regressions.length) process.exitCode = 1;
}
