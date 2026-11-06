import { invoke } from '@tauri-apps/api/core';
import type { ChatProjectSnapshot, ChatProjectStore } from '../../desktop/src/features/remote/useMobileChatProjects';

export function phoneChatProjects(hostId: string | null): ChatProjectStore {
  const run = (action: string, args: Record<string, string | null> = {}) =>
    invoke<ChatProjectSnapshot>('mobile_chat_projects', { hostId, action, ...args });
  return {
    list: () => run('list'),
    create: (projectId, name) => run('create', { projectId, name }),
    rename: (projectId, name) => run('rename', { projectId, name }),
    remove: projectId => run('delete', { projectId }),
    move: (conversationId, projectId) => run('move', { conversationId, projectId }),
    activeChats: hostId === null ? projectId => invoke('mobile_project_chats', { projectId }) : undefined,
  };
}
