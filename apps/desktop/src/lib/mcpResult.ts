import type { ArtifactPayload } from '../types/conversation';

export type McpContentBlock =
  | { type: 'text'; text: string }
  | { type: 'image' | 'audio'; data: string; mimeType: string }
  | { type: 'resource'; resource: { uri: string; mimeType?: string; text?: string; blob?: string } }
  | { type: 'resource_link'; uri: string; name: string; title?: string; mimeType?: string }
  | { type: 'unsupported'; originalType: string; message: string };
export interface McpToolResult { kind: 'mcpToolResult'; version: 1; toolIdentity?: { connectorId: string; toolName: string }; contentBlocks: McpContentBlock[]; structuredContent?: unknown; notices: string[]; }

export function extractMcpResult(artifacts: ArtifactPayload | undefined): McpToolResult | null {
  if (!artifacts || Array.isArray(artifacts) || artifacts.kind !== 'mcpToolResult' || artifacts.version !== 1 || !Array.isArray(artifacts.contentBlocks)) return null;
  const blocks = artifacts.contentBlocks.filter((block): block is McpContentBlock => {
    if (!block || typeof block !== 'object') return false;
    switch (block.type) {
      case 'text': return typeof block.text === 'string';
      case 'image': case 'audio': return typeof block.data === 'string' && typeof block.mimeType === 'string';
      case 'resource': return block.resource && typeof block.resource.uri === 'string';
      case 'resource_link': return typeof block.uri === 'string' && typeof block.name === 'string';
      case 'unsupported': return typeof block.originalType === 'string' && typeof block.message === 'string';
      default: return false;
    }
  });
  const identity = artifacts.toolIdentity as McpToolResult['toolIdentity'];
  return { kind: 'mcpToolResult', version: 1, toolIdentity: identity && typeof identity.connectorId === 'string' && typeof identity.toolName === 'string' ? identity : undefined, contentBlocks: blocks, structuredContent: artifacts.structuredContent, notices: Array.isArray(artifacts.notices) ? artifacts.notices.filter((item): item is string => typeof item === 'string') : [] };
}

export function mcpExternalLink(uri: string): string | null {
  try { const url = new URL(uri); return ['https:','http:'].includes(url.protocol) ? url.href : null; } catch { return null; }
}

export function mcpInlineMedia(data: string, mimeType: string, kind: 'image' | 'audio'): string | null {
  const allowed = kind === 'image' ? /^(image\/(png|jpeg|webp|gif))$/ : /^(audio\/(mpeg|mp3|wav|x-wav|ogg|webm|mp4|aac|flac))$/;
  return allowed.test(mimeType) && data.length <= 6*1024*1024 && /^[A-Za-z0-9+/]*={0,2}$/.test(data) ? `data:${mimeType};base64,${data}` : null;
}
