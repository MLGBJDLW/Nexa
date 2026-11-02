import { useNavigate } from 'react-router';
import { useTranslation } from '../../i18n';
import { runAppCommand, useAppCommands } from '../../lib/appCommands';

export function WorkspaceErrorRecovery({ error }: { error: string | null | undefined }) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const commands = useAppCommands();
  if (!error || !/Workspace folder is unavailable|Choose a project workspace folder|existing absolute working directory/.test(error)) return null;
  const project = commands.some(command => command.id === 'chat.workspace-folders' && command.enabled);
  return <button type="button" className="rounded-md bg-surface-2 px-2 py-1 text-xs text-accent"
    onClick={() => { if (!project || !runAppCommand('chat.workspace-folders')) navigate('/settings'); }}>
    {t(project ? 'project.workspaceFolders' : 'settings.aiProviders')}
  </button>;
}
