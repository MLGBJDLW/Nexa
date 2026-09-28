import { useEffect, useState } from 'react';
import { useTranslation } from '../../i18n';
import { getProject } from '../../lib/api';
import { ProjectIcon } from '../../lib/projectIcons';
import type { Project } from '../../types/project';
import { Plus } from 'lucide-react';
import { Button } from '../ui/Button';

export function ProjectConversationStart({ projectId, onStart }: { projectId: string; onStart: () => void }) {
  const { t } = useTranslation();
  const [project, setProject] = useState<Project | null>(null);
  useEffect(() => {
    let current = true;
    void getProject(projectId).then(project => {
      if (current) setProject(project);
    }).catch(() => { if (current) setProject(null); });
    return () => { current = false; };
  }, [projectId]);
  const selected = project?.id === projectId ? project : null;
  return (
    <div data-testid="project-new-conversation" className="mx-4 min-w-0 space-y-5 px-2 py-10 text-center">
      <h1 className="flex min-w-0 items-center justify-center gap-2 text-xl font-semibold text-text-primary">
        {selected && <ProjectIcon icon={selected.icon} color={selected.color} className="h-7 w-7 shrink-0" size={16} />}
        <span className="min-w-0 break-words">{selected ? t('project.welcomeTitle', { name: selected.name }) : t('project.newConversationTitle')}</span>
      </h1>
      <Button size="sm" icon={<Plus size={15} />} onClick={onStart} data-testid="project-start-chat">{t('chat.newChat')}</Button>
    </div>
  );
}
