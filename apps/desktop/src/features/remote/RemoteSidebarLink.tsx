import { NavLink } from 'react-router';
import { Smartphone } from 'lucide-react';
import { Tooltip } from '../../components/ui';
import { useTranslation } from '../../i18n';
import { useRemoteDesktopStatus } from './remoteDesktopStatus';
export function RemoteSidebarLink() {
  const { t } = useTranslation();
  const { status } = useRemoteDesktopStatus();
  const online = status?.connectedDeviceIds?.length || 0;
  const caption = online
    ? t('remote.connectedCount', { count: online })
    : status?.preparing
      ? t('remote.starting')
      : status?.enabled
        ? t('remote.ready')
        : t('remote.disabled');
  return (
    <Tooltip content={`${t('remote.title')} · ${caption}`} side="right" delay={180}>
      <NavLink
        to="/remote"
        aria-label={t('remote.title')}
        data-testid="remote-sidebar-link"
        data-remote-status={
          online
            ? 'connected'
            : status?.preparing
              ? 'preparing'
              : status?.enabled
                ? 'ready'
                : 'disabled'
        }
        className={({ isActive }) =>
          `relative grid h-10 w-10 place-items-center rounded-md transition-colors ${isActive ? 'bg-accent-subtle text-accent-hover' : 'text-text-tertiary hover:bg-surface-2 hover:text-text-primary'}`
        }
      >
        <Smartphone className="h-4.5 w-4.5" />
        {(status?.enabled || status?.preparing) && (
          <span
            className={`absolute right-1.5 top-1.5 h-1.5 w-1.5 rounded-full ring-2 ring-surface-0 ${status.warning || status.preparing ? 'bg-amber-400' : online ? 'bg-emerald-500' : 'bg-accent'}`}
          />
        )}
      </NavLink>
    </Tooltip>
  );
}
