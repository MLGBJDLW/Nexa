import { useTranslation } from '../i18n';

interface LogoProps {
  size?: number;
  className?: string;
}

export function Logo({ size = 32, className }: LogoProps) {
  const { t } = useTranslation();
  return (
    <img
      src="/logo.svg"
      alt={t('app.name')}
      width={size}
      height={size}
      className={className}
    />
  );
}
