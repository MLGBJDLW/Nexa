import {
  AudioLines,
  Camera,
  Film,
  Gamepad2,
  Globe2,
  Cpu,
  Network,
  NotebookPen,
  ChartNoAxesCombined,
  Music2,
  Layers3,
  Leaf,
  Compass,
  Microscope,
  Coffee,
  Orbit,
  Wrench,
  Languages,
  BookOpen,
  BriefcaseBusiness,
  Code2,
  Database,
  Folder,
  FlaskConical,
  GraduationCap,
  Lightbulb,
  Palette,
  Rocket,
  ShieldCheck,
  Sparkles,
  type LucideIcon,
} from 'lucide-react';
import type { CSSProperties } from 'react';
import { useTranslation, type TranslationKey } from '../i18n';

export interface ProjectIconOption {
  id: string;
  labelKey: TranslationKey;
  icon: LucideIcon;
  tone: string;
}

export interface ProjectColorOption {
  label: string;
  value: string;
}

export const PROJECT_ICON_OPTIONS: ProjectIconOption[] = [
  { id: 'folder', labelKey: 'project.iconGeneral', icon: Folder, tone: 'text-sky-500 bg-sky-500/10' },
  { id: 'book', labelKey: 'project.iconResearch', icon: BookOpen, tone: 'text-emerald-500 bg-emerald-500/10' },
  { id: 'briefcase', labelKey: 'project.iconWork', icon: BriefcaseBusiness, tone: 'text-amber-500 bg-amber-500/10' },
  { id: 'code', labelKey: 'project.iconCode', icon: Code2, tone: 'text-violet-500 bg-violet-500/10' },
  { id: 'database', labelKey: 'project.iconKnowledge', icon: Database, tone: 'text-cyan-500 bg-cyan-500/10' },
  { id: 'rocket', labelKey: 'project.iconLaunch', icon: Rocket, tone: 'text-orange-500 bg-orange-500/10' },
  { id: 'sparkles', labelKey: 'project.iconCreative', icon: Sparkles, tone: 'text-fuchsia-500 bg-fuchsia-500/10' },
  { id: 'palette', labelKey: 'project.iconDesign', icon: Palette, tone: 'text-rose-500 bg-rose-500/10' },
  { id: 'flask', labelKey: 'project.iconExperiment', icon: FlaskConical, tone: 'text-lime-500 bg-lime-500/10' },
  { id: 'graduation', labelKey: 'project.iconLearning', icon: GraduationCap, tone: 'text-blue-500 bg-blue-500/10' },
  { id: 'shield', labelKey: 'project.iconSecurity', icon: ShieldCheck, tone: 'text-teal-500 bg-teal-500/10' },
  { id: 'idea', labelKey: 'project.iconIdeas', icon: Lightbulb, tone: 'text-yellow-500 bg-yellow-500/10' },
  { id: 'audio', labelKey: 'project.iconAudio', icon: AudioLines, tone: 'text-fuchsia-500 bg-fuchsia-500/10' },
  { id: 'camera', labelKey: 'project.iconPhotography', icon: Camera, tone: 'text-rose-500 bg-rose-500/10' },
  { id: 'film', labelKey: 'project.iconFilm', icon: Film, tone: 'text-violet-500 bg-violet-500/10' },
  { id: 'gamepad', labelKey: 'project.iconGames', icon: Gamepad2, tone: 'text-purple-500 bg-purple-500/10' },
  { id: 'globe', labelKey: 'project.iconWeb', icon: Globe2, tone: 'text-cyan-500 bg-cyan-500/10' },
  { id: 'cpu', labelKey: 'project.iconHardware', icon: Cpu, tone: 'text-teal-500 bg-teal-500/10' },
  { id: 'network', labelKey: 'project.iconInfrastructure', icon: Network, tone: 'text-sky-500 bg-sky-500/10' },
  { id: 'notes', labelKey: 'project.iconWriting', icon: NotebookPen, tone: 'text-amber-500 bg-amber-500/10' },
  { id: 'chart', labelKey: 'project.iconAnalytics', icon: ChartNoAxesCombined, tone: 'text-indigo-500 bg-indigo-500/10' },
  { id: 'music', labelKey: 'project.iconMusic', icon: Music2, tone: 'text-pink-500 bg-pink-500/10' },
  { id: 'layers', labelKey: 'project.iconArchitecture', icon: Layers3, tone: 'text-slate-500 bg-slate-500/10' },
  { id: 'leaf', labelKey: 'project.iconEnvironment', icon: Leaf, tone: 'text-emerald-500 bg-emerald-500/10' },
  { id: 'compass', labelKey: 'project.iconExploration', icon: Compass, tone: 'text-orange-500 bg-orange-500/10' },
  { id: 'microscope', labelKey: 'project.iconScience', icon: Microscope, tone: 'text-lime-500 bg-lime-500/10' },
  { id: 'coffee', labelKey: 'project.iconPersonal', icon: Coffee, tone: 'text-amber-500 bg-amber-500/10' },
  { id: 'orbit', labelKey: 'project.iconSpace', icon: Orbit, tone: 'text-blue-500 bg-blue-500/10' },
  { id: 'tools', labelKey: 'project.iconTools', icon: Wrench, tone: 'text-stone-500 bg-stone-500/10' },
  { id: 'languages', labelKey: 'project.iconLanguages', icon: Languages, tone: 'text-cyan-500 bg-cyan-500/10' },
];

export const PROJECT_COLOR_OPTIONS: ProjectColorOption[] = [
  { label: 'Sky', value: '#0ea5e9' },
  { label: 'Emerald', value: '#10b981' },
  { label: 'Amber', value: '#f59e0b' },
  { label: 'Violet', value: '#8b5cf6' },
  { label: 'Cyan', value: '#06b6d4' },
  { label: 'Orange', value: '#f97316' },
  { label: 'Fuchsia', value: '#d946ef' },
  { label: 'Rose', value: '#f43f5e' },
  { label: 'Lime', value: '#84cc16' },
  { label: 'Blue', value: '#3b82f6' },
  { label: 'Teal', value: '#14b8a6' },
  { label: 'Slate', value: '#64748b' },
];

export const DEFAULT_PROJECT_COLOR = PROJECT_COLOR_OPTIONS[0].value;

const LEGACY_ICON_ALIASES: Record<string, string> = {
  '📁': 'folder',
  '📚': 'book',
  '💼': 'briefcase',
  '💻': 'code',
  '🚀': 'rocket',
  '✨': 'sparkles',
  '🎨': 'palette',
  '🔬': 'flask',
  '🎓': 'graduation',
  '🛡️': 'shield',
  '💡': 'idea',
};

export function normalizeProjectIconId(icon: string | null | undefined): string {
  const trimmed = icon?.trim() ?? '';
  return LEGACY_ICON_ALIASES[trimmed] ?? trimmed;
}

export function getProjectIconOption(icon: string | null | undefined): ProjectIconOption {
  const id = normalizeProjectIconId(icon);
  return PROJECT_ICON_OPTIONS.find((option) => option.id === id) ?? PROJECT_ICON_OPTIONS[0];
}

function normalizeHexColor(color: string | null | undefined): string | null {
  const trimmed = color?.trim() ?? '';
  if (!/^#[0-9a-fA-F]{6}$/.test(trimmed)) return null;
  return trimmed.toLowerCase();
}

export function normalizeProjectColor(
  color: string | null | undefined,
  fallback = DEFAULT_PROJECT_COLOR,
): string {
  return normalizeHexColor(color) ?? fallback;
}

function hexToRgba(hex: string, alpha: number): string {
  const normalized = normalizeHexColor(hex) ?? DEFAULT_PROJECT_COLOR;
  const red = Number.parseInt(normalized.slice(1, 3), 16);
  const green = Number.parseInt(normalized.slice(3, 5), 16);
  const blue = Number.parseInt(normalized.slice(5, 7), 16);
  return `rgba(${red}, ${green}, ${blue}, ${alpha})`;
}

export function ProjectIcon({
  icon,
  color,
  className = '',
  size = 14,
}: {
  icon?: string | null;
  color?: string | null;
  className?: string;
  size?: number;
}) {
  const { t } = useTranslation();
  const option = getProjectIconOption(icon);
  const Icon = option.icon;
  const customColor = normalizeHexColor(color);
  const customStyle: CSSProperties | undefined = customColor
    ? {
        color: customColor,
        backgroundColor: hexToRgba(customColor, 0.14),
        borderColor: hexToRgba(customColor, 0.24),
      }
    : undefined;

  return (
    <span
      className={`inline-flex shrink-0 items-center justify-center rounded-md ${
        customColor ? 'border' : option.tone
      } ${className}`}
      style={customStyle}
      title={t(option.labelKey)}
      aria-hidden="true"
    >
      <Icon size={size} strokeWidth={1.8} />
    </span>
  );
}
