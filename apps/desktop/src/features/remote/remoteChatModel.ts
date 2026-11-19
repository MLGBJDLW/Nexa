import type { TurnModelSelection } from '../models/modelChoices';

export interface ChatConnection {
  id: string;
  name: string;
  model: string;
  isDefault: boolean;
  provider?: string;
}

export interface ChatConversation {
  id: string;
  title: string;
  model: string;
  provider?: string;
  agentConfigId?: string | null;
  modelSelection?: TurnModelSelection | null;
}

export interface ChatModelRoute {
  connectionId: string;
  provider?: string;
  modelSelection: TurnModelSelection | null;
  submitted?: boolean;
}

export function resolveChatModel(
  connections: ChatConnection[], conversationId: string, chat: ChatConversation | undefined,
  draft: ChatModelRoute | null,
): ChatModelRoute {
  const id = draft?.connectionId ?? chat?.agentConfigId;
  const provider = draft?.provider ?? (draft ? undefined : chat?.provider);
  let connection: ChatConnection | undefined;
  if (id) {
    connection = connections.find(item => item.id === id && (!provider || !item.provider || item.provider === provider));
  } else if (!conversationId) {
    connection = connections.find(item => item.isDefault) ?? connections[0];
  } else if (chat) {
    // Old hosts did not expose a connection ID. Select automatically only
    // when the available account is unambiguous for this saved route.
    const candidates = connections.filter(item => provider ? item.provider === provider : item.model === chat.model);
    if (candidates.length === 1) connection = candidates[0];
  }
  const legacySelection = chat?.model
    ? { model: chat.model, reasoningEnabled: null, reasoningEffort: null, thinkingBudget: null }
    : null;
  return {
    connectionId: connection?.id ?? '', provider: connection?.provider,
    modelSelection: draft ? draft.modelSelection : chat?.modelSelection ?? legacySelection,
  };
}

export function readChatModelDraft(key: string): ChatModelRoute | null {
  try {
    const route = JSON.parse(localStorage.getItem(key) ?? 'null') as ChatModelRoute | null;
    return route && typeof route.connectionId === 'string' &&
      (route.modelSelection === null || typeof route.modelSelection?.model === 'string') ? route : null;
  } catch {
    return null;
  }
}
