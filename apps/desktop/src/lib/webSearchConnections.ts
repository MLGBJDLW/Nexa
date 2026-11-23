import type { WebSearchCustomProviderConfig } from '../types/conversation';

export const deepSeekSearchProvider: WebSearchCustomProviderConfig = {
  id: 'deepseek', preset: 'deepseek', name: 'DeepSeek Search & Fetch', enabled: false, apiKey: '',
  baseUrl: 'https://api.deepseek.com/v1/tools/search', priority: 15,
};
