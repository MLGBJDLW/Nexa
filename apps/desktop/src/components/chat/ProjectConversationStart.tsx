import { useEffect, useState } from 'react';
import { useTranslation } from '../../i18n';
import { getProject } from '../../lib/api';
import { ProjectIcon } from '../../lib/projectIcons';
import type { Project } from '../../types/project';

export function ProjectConversationStart({ projectId }: { projectId: string }) {
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
    <div data-testid="project-new-conversation" className="mx-4 mb-5 min-w-0 space-y-2 px-2 text-center">
      {selected && (
        <div className="flex min-w-0 items-center justify-center gap-2 text-sm text-text-secondary">
          <ProjectIcon icon={selected.icon} color={selected.color} className="h-6 w-6 shrink-0" size={14} />
          <span className="max-w-full truncate">{selected.name}</span>
        </div>
      )}
      <h1 className="text-xl font-semibold text-text-primary">{t('project.newConversationTitle')}</h1>
      {selected && <p className="break-words text-sm text-text-tertiary">{t('project.newConversationHint', { name: selected.name })}</p>}
    </div>
  );
}
