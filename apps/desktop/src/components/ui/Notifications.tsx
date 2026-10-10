import { createPortal } from 'react-dom';
import type { CSSProperties } from 'react';
import { Toaster } from 'sonner';
import { useTranslation } from '../../i18n';
import { useTheme } from '../../lib/ThemeProvider';
import { isLightTheme } from '../../lib/theme';
import './notifications.css';

/** A single notification surface, kept clear of the composer and side docks. */
export function Notifications() {
  const { theme } = useTheme();
  const { t } = useTranslation();
  const light = isLightTheme(theme);
  return <NotificationSurface light={light} closeLabel={t('common.close')} />;
}

export function NotificationSurface({ light, closeLabel }: { light: boolean; closeLabel: string }) {
  return createPortal(<Toaster theme={light ? 'light' : 'dark'} position="top-center"
    className="nexa-notifications" closeButton visibleToasts={2} gap={8} duration={5500}
    offset={{ top: 54 }} mobileOffset={{ top: 50, left: 12, right: 12 }}
    style={{ '--notification-base': light ? '#ffffff' : '#191b22', '--width': '380px', zIndex: 80 } as CSSProperties}
    toastOptions={{ className: 'nexa-notification', closeButtonAriaLabel: closeLabel }}
  />, document.body);
}
