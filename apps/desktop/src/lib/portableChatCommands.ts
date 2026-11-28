import { buildSlashCommandOptions, resolveSlashCommandSelection } from './slashCommands';

// These prompt-only commands can run on phone, paired and desktop chat without host UI actions.
const names = new Set(['debug', 'refactor', 'test', 'docs', 'research', 'summarize', 'compare', 'tasks', 'review-prompt', 'skills']);
export const portableCommands = buildSlashCommandOptions([], []).filter(option => names.has(option.name));

const intentPatterns: [string, RegExp][] = [
  ['compare', /\b(compare|versus|trade.?offs?)\b|对比|比较|比較|相違/iu],
  ['summarize', /\b(summari[sz]e|minutes|recap|summary)\b|总结|摘要|纪要|總結|紀要|要約/iu],
  ['debug', /\b(debug|fix|crash|bug|broken|fails?)\b|修复|修復|报错|報錯|闪退|卡死|不工作|エラー/iu],
  ['refactor', /\b(refactor|reorganize)\b|重构|重構|リファクタ/iu],
  ['review-prompt', /\b(review|audit)\b|审查|評審|审阅|レビュー/iu],
  ['test', /\b(tests?|verify|validate)\b|测试|測試|验证|テスト/iu],
  ['docs', /\b(documentation|readme|tutorial)\b|文档|文件說明|教程|文書化/iu],
  ['tasks', /\b(plan|milestones|break down)\b|计划|规划|規劃|計画/iu],
  ['research', /\b(research|investigate|look up)\b|调研|調研|查证|調査/iu],
];

export function suggestedPrompt(draft: string) {
  if (!draft.trim() || draft.trimStart().startsWith('/')) return null;
  const name = intentPatterns.find(([, pattern]) => pattern.test(draft))?.[0];
  const command = portableCommands.find(option => option.name === name);
  if (!command || !command.promptTemplate || draft.startsWith(command.promptTemplate.split('{{input}}')[0])) return null;
  return resolveSlashCommandSelection(command, draft).message;
}
