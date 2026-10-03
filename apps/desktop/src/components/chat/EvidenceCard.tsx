import { useEffect, useRef, useCallback, useState } from 'react';
import { createPortal } from 'react-dom';
import { motion, AnimatePresence, useReducedMotion } from 'framer-motion';
import { FileText, Film, Music, Clock, ExternalLink, Copy, Check, X } from 'lucide-react';
import { useTranslation } from '../../i18n';
import { openFileInDefaultApp, showInFileExplorer, getEvidenceCard } from '../../lib/api';
import { canPreviewInApp, useFilePreview } from '../../features/preview';
import { getSoftDropdownMotion } from '../../lib/uiMotion';
import { VideoPreviewModal } from '../media/VideoPreviewModal';
import type { CitationCardData } from '../../lib/citationParser';
import { isWebUrl, sourceBasename, sourceHost } from '../../lib/sourceDisplay';
import { useOverlayRoot } from '../ui/overlay/OverlayProvider';
import { formatUserError } from '../../lib/userError';
import { toast } from 'sonner';

/* ------------------------------------------------------------------ */
/*  Types                                                              */
/* ------------------------------------------------------------------ */

interface EvidenceCardPopupProps {
  card: CitationCardData;
  /** Anchor element to position near */
  anchorRect: DOMRect | null;
  onClose: () => void;
}

/* ------------------------------------------------------------------ */
/*  Helpers                                                            */
/* ------------------------------------------------------------------ */

const VIDEO_EXTS = ['.mp4', '.mkv', '.webm', '.mov', '.avi', '.flv', '.wmv', '.m4v', '.mpeg', '.mpg'];
const AUDIO_EXTS = ['.mp3', '.wav', '.flac', '.ogg', '.aac', '.m4a', '.wma', '.opus'];

function isVideoFile(path: string): boolean {
  return VIDEO_EXTS.some(ext => path.toLowerCase().endsWith(ext));
}

function isAudioFile(path: string): boolean {
  return AUDIO_EXTS.some(ext => path.toLowerCase().endsWith(ext));
}

function extractTimestamp(headingContext: string | undefined): string | null {
  if (!headingContext) return null;
  const match = headingContext.match(/(\d{2}:\d{2}:\d{2})/);
  return match ? match[1] : null;
}

function truncate(text: string, max: number): string {
  if (text.length <= max) return text;
  return text.slice(0, max).trimEnd() + '…';
}

function formatScore(score: number): string {
  if (score <= 0) return '';
  return `${(score * 100).toFixed(0)}%`;
}

function popupStyle(anchorRect: DOMRect | null): React.CSSProperties {
  const width = Math.min(340, Math.max(0, window.innerWidth - 16));
  const height = Math.min(300, Math.max(0, window.innerHeight - 16));
  return {
    position: 'fixed',
    top: Math.max(8, Math.min((anchorRect?.bottom ?? 8) + 6, window.innerHeight - height - 8)),
    left: Math.max(8, Math.min(anchorRect?.left ?? 8, window.innerWidth - width - 8)),
    width,
    maxHeight: height,
    zIndex: 100,
  };
}

/* ------------------------------------------------------------------ */
/*  Component                                                          */
/* ------------------------------------------------------------------ */

export function EvidenceCardPopup({ card, anchorRect, onClose }: EvidenceCardPopupProps) {
  const { t } = useTranslation();
  const overlayRoot = useOverlayRoot();
  const shouldReduceMotion = useReducedMotion();
  const { openFilePreview, openWebLink, openEvidence, remote } = useFilePreview();
  const popupRef = useRef<HTMLDivElement>(null);
  const [copied, setCopied] = useState(false);
  const [videoPreviewPath, setVideoPreviewPath] = useState<string | null>(null);
  useEffect(() => { popupRef.current?.querySelector<HTMLButtonElement>('button')?.focus(); }, []);

  // Close on click outside
  useEffect(() => {
    function handleClickOutside(e: MouseEvent) {
      if (popupRef.current && !popupRef.current.contains(e.target as Node)) {
        onClose();
      }
    }
    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, [onClose]);

  // Close on Escape
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if (e.key === 'Escape') onClose();
    }
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [onClose]);

  const handleOpenFile = useCallback(() => {
    if (card.evidenceRef && openEvidence) { onClose(); openEvidence(card.evidenceRef); return; }
    if (!card.documentPath) return;
    if (isWebUrl(card.documentPath)) {
      openWebLink(card.documentPath, card.documentTitle || sourceHost(card.documentPath));
      return;
    }
    if (remote || canPreviewInApp(card.documentPath)) {
      openFilePreview(card.documentPath);
    } else {
      void openFileInDefaultApp(card.documentPath).catch((error) => toast.error(formatUserError(t('card.fileNotFound'), error)));
    }
  }, [card.documentPath, card.documentTitle, card.evidenceRef, openFilePreview, openWebLink, openEvidence, onClose, remote, t]);

  const handleShowInExplorer = useCallback(() => {
    if (card.documentPath && !isWebUrl(card.documentPath)) {
      void showInFileExplorer(card.documentPath).catch((error) => toast.error(formatUserError(t('card.fileNotFound'), error)));
    }
  }, [card.documentPath, t]);

  const handleCopy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(card.content);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch (error) {
      toast.error(formatUserError(t('common.error'), error));
    }
  }, [card.content, t]);

  // Position: below the anchor, clamped to viewport
  const style = popupStyle(anchorRect);

  const isWebSource = isWebUrl(card.documentPath);
  const title = card.documentTitle || sourceBasename(card.documentPath) || t('citation.evidence');
  const scoreLabel = formatScore(card.score);
  const isVideo = card.documentPath && !isWebSource ? isVideoFile(card.documentPath) : false;
  const isAudio = card.documentPath && !isWebSource ? isAudioFile(card.documentPath) : false;
  const headingCtx = card.headingPath.length > 0 ? card.headingPath.join(' › ') : undefined;
  const timestamp = extractTimestamp(headingCtx);

  const FileIcon = isWebSource ? ExternalLink : isVideo ? Film : isAudio ? Music : FileText;
  const iconColor = isWebSource ? 'text-accent' : isVideo ? 'text-violet-500' : isAudio ? 'text-amber-500' : 'text-accent';

  return (
    <>
    {createPortal(<AnimatePresence>
      <motion.div
        ref={popupRef}
        {...getSoftDropdownMotion(!!shouldReduceMotion)}
        style={style}
        className="rounded-lg border border-border bg-surface-1 shadow-xl overflow-y-auto"
        role="dialog"
        aria-label={t('citation.evidence')}
      >
        {/* Header */}
        <div className="flex items-center gap-2 px-3 py-2 border-b border-border bg-surface-2">
          <FileIcon className={`h-3.5 w-3.5 ${iconColor} shrink-0`} />
          <span className="text-xs font-medium text-text-primary truncate flex-1" title={title}>
            {title}
          </span>
          {scoreLabel && (
            <span className="text-[10px] font-medium text-accent bg-accent/10 px-1.5 py-0.5 rounded-full shrink-0">
              {scoreLabel}
            </span>
          )}
          {timestamp && (isVideo || isAudio) && (
            <span className="inline-flex items-center px-1.5 py-0.5 rounded-full text-[10px] font-medium bg-violet-100 text-violet-700 dark:bg-violet-900/30 dark:text-violet-300 shrink-0">
              <Clock className="h-2.5 w-2.5 mr-0.5" />
              {timestamp}
            </span>
          )}
          <button
            type="button"
            onClick={onClose}
            className="p-0.5 text-text-tertiary hover:text-text-primary transition-colors cursor-pointer"
            aria-label={t('common.close')}
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>

        {/* Source path */}
        {card.documentPath && (
          <div className="px-3 py-1.5 border-b border-border/50">
            <span className="text-[10px] text-text-tertiary break-all">
              {card.sourceName ? `${card.sourceName} · ` : ''}
              {isWebSource ? sourceHost(card.documentPath) : card.documentPath}
            </span>
          </div>
        )}

        {/* Heading path breadcrumb */}
        {card.headingPath.length > 0 && (
          <div className="px-3 py-1 border-b border-border/50">
            <span className="text-[10px] text-text-tertiary">
              {card.headingPath.join(' › ')}
            </span>
          </div>
        )}

        {/* Content preview */}
        <div className="px-3 py-2 max-h-[150px] overflow-y-auto">
          <p className="text-xs text-text-secondary leading-relaxed whitespace-pre-wrap">
            {truncate(card.snippet || card.content, 200)}
          </p>
        </div>

        {/* Actions */}
        <div className="flex items-center gap-1 px-3 py-2 border-t border-border bg-surface-2">
          {card.documentPath && (
            <>
              <button
                type="button"
                onClick={isVideo && !remote && !card.evidenceRef ? () => setVideoPreviewPath(card.documentPath) : handleOpenFile}
                className="inline-flex items-center gap-1 px-2 py-1 text-[10px] font-medium rounded-md
                  bg-accent/10 text-accent hover:bg-accent/20 transition-colors cursor-pointer"
              >
                <ExternalLink className="h-3 w-3" />
                {isVideo ? t('media.videoDetails') : isWebSource ? sourceHost(card.documentPath) : t('citation.openFile')}
              </button>
              {!isWebSource && !remote && (
                <button
                  type="button"
                  onClick={handleShowInExplorer}
                  className="inline-flex items-center gap-1 px-2 py-1 text-[10px] font-medium rounded-md
                    bg-surface-3 text-text-tertiary hover:text-text-primary transition-colors cursor-pointer"
                >
                  {t('citation.showInFolder')}
                </button>
              )}
            </>
          )}
          <div className="flex-1" />
          <button
            type="button"
            onClick={handleCopy}
            className="inline-flex items-center gap-1 px-2 py-1 text-[10px] font-medium rounded-md
              bg-surface-3 text-text-tertiary hover:text-text-primary transition-colors cursor-pointer"
          >
            {copied ? (
              <>
                <Check className="h-3 w-3 text-green-500" />
                <span className="text-green-500">{t('chat.copied')}</span>
              </>
            ) : (
              <>
                <Copy className="h-3 w-3" />
                {t('citation.copy')}
              </>
            )}
          </button>
        </div>
      </motion.div>
    </AnimatePresence>, overlayRoot ?? document.body)}

    {videoPreviewPath && (
      <VideoPreviewModal
        open
        onClose={() => setVideoPreviewPath(null)}
        filePath={videoPreviewPath}
      />
    )}
    </>
  );
}

/* ------------------------------------------------------------------ */
/*  Inline Citation Chip                                               */
/* ------------------------------------------------------------------ */

interface CitationChipProps {
  chunkId: string;
  displayText: string;
  card: CitationCardData | undefined;
}

export function CitationChip({ chunkId, displayText, card }: CitationChipProps) {
  const { t } = useTranslation();
  const overlayRoot = useOverlayRoot();
  const { loadEvidence } = useFilePreview();
  const [popupOpen, setPopupOpen] = useState(false);
  const [anchorRect, setAnchorRect] = useState<DOMRect | null>(null);
  const [fetchedCard, setFetchedCard] = useState<CitationCardData | null>(null);
  const [fetching, setFetching] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const fetchGeneration = useRef(0);
  const chipRef = useRef<HTMLButtonElement>(null);
  const statusRef = useRef<HTMLDivElement>(null);

  const resolvedCard = fetchedCard ?? card;

  useEffect(() => {
    setFetchedCard(null);
    setError(null);
    setFetching(false);
    setPopupOpen(false);
    return () => { fetchGeneration.current += 1; };
  }, [chunkId]);

  const fetchCard = useCallback(async () => {
    const generation = ++fetchGeneration.current;
    setFetching(true);
    setError(null);
    try {
      const ec = await (loadEvidence ?? getEvidenceCard)(chunkId, card?.evidenceRef);
      if (generation === fetchGeneration.current) setFetchedCard(ec);
    } catch (cause) {
      if (generation === fetchGeneration.current) setError(formatUserError(t('citation.loadError'), cause));
    } finally {
      if (generation === fetchGeneration.current) setFetching(false);
    }
  }, [chunkId, card?.evidenceRef, loadEvidence, t]);

  useEffect(() => {
    if (!popupOpen) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setPopupOpen(false);
        chipRef.current?.focus();
      }
    };
    const dismiss = (event: MouseEvent) => {
      if (!chipRef.current?.contains(event.target as Node) && !statusRef.current?.contains(event.target as Node) && (!resolvedCard || fetching || error)) setPopupOpen(false);
    };
    const reposition = () => setAnchorRect(chipRef.current?.getBoundingClientRect() ?? null);
    document.addEventListener('keydown', closeOnEscape);
    document.addEventListener('mousedown', dismiss);
    window.addEventListener('resize', reposition);
    window.addEventListener('scroll', reposition, true);
    return () => {
      document.removeEventListener('keydown', closeOnEscape);
      document.removeEventListener('mousedown', dismiss);
      window.removeEventListener('resize', reposition);
      window.removeEventListener('scroll', reposition, true);
    };
  }, [popupOpen, resolvedCard, fetching, error]);

  const handleClick = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    if (chipRef.current) {
      setAnchorRect(chipRef.current.getBoundingClientRect());
    }

    if (!fetchedCard && !fetching && !popupOpen) void fetchCard();
    setPopupOpen((prev) => !prev);
  }, [fetchedCard, fetching, popupOpen, fetchCard]);

  const handleClose = useCallback(() => {
    setPopupOpen(false);
    chipRef.current?.focus();
  }, []);

  const title = resolvedCard?.documentTitle || sourceBasename(resolvedCard?.documentPath) || chunkId.slice(0, 8);
  const tooltipText = resolvedCard ? `${resolvedCard.documentTitle || sourceBasename(resolvedCard.documentPath)}` : chunkId.slice(0, 8);
  const isWebSource = isWebUrl(resolvedCard?.documentPath);

  return (
    <>
      <button
        ref={chipRef}
        type="button"
        onClick={handleClick}
        title={tooltipText}
        aria-expanded={popupOpen}
        aria-haspopup="dialog"
        className="inline-flex items-center gap-0.5 px-1.5 py-0 text-[11px] font-medium
          rounded-full border cursor-pointer transition-all duration-150
          bg-accent/10 text-accent border-accent/20
          hover:bg-accent/20 hover:border-accent/30
          active:scale-95 align-baseline leading-[1.4]
          mx-0.5"
      >
        {isWebSource ? <ExternalLink className="h-2.5 w-2.5 shrink-0" /> : isVideoFile(resolvedCard?.documentPath ?? '') ? <Film className="h-2.5 w-2.5 shrink-0" /> : isAudioFile(resolvedCard?.documentPath ?? '') ? <Music className="h-2.5 w-2.5 shrink-0" /> : <FileText className="h-2.5 w-2.5 shrink-0" />}
        <span className="truncate max-w-[120px]">{displayText || title}</span>
      </button>
      {popupOpen && resolvedCard && !fetching && !error && (
        <EvidenceCardPopup card={resolvedCard} anchorRect={anchorRect} onClose={handleClose} />
      )}
      {popupOpen && (!resolvedCard || fetching || error) && createPortal(
        <div ref={statusRef} role="dialog" aria-label={t('citation.evidence')} style={popupStyle(anchorRect)} className="rounded-lg border border-border bg-surface-1 p-3 shadow-xl overflow-y-auto">
          <button type="button" onClick={handleClose} aria-label={t('common.close')} className="float-right p-1"><X size={14} /></button>
          <p role={error ? 'alert' : 'status'} className="pr-6 text-xs text-text-secondary">{error ?? t('common.loading')}</p>
          {error && <button type="button" className="mt-2 text-xs text-accent" onClick={() => void fetchCard()}>{t('common.retry')}</button>}
        </div>, overlayRoot ?? document.body,
      )}
    </>
  );
}
