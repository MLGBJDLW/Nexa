/** Compare public hosted IDs without merging model versions or private names. */
export function usageModelIdentity(modelId: string): string {
  let model = modelId.trim();
  const hosted = /^(openai|anthropic|google|mistralai|cohere|perplexity|deepseek|deepseek-ai|minimax|qwen|x-ai|z-ai|moonshotai)\/(.+)$/i.exec(model);
  if (hosted) model = hosted[2];
  if (/^(?:gpt-|o\d(?:-|$)|claude-|gemini-|mistral-|codestral-|command-|sonar|deepseek-|minimax-|qwen\d|grok-|glm-|kimi-)/i.test(model)) model = model.toLowerCase();
  // Anthropic and Copilot/router catalogs spell the same version differently.
  // Keep date snapshots, effort variants and pricing suffixes intact.
  return model.replace(/^(claude-(?:opus|sonnet|haiku)-\d+)-(\d+)(?=-|:|$)/i, '$1.$2');
}
