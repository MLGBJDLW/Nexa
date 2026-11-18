import type { useTranslation } from '../i18n';
import type { EvidenceLocator } from '../types/evidence';

function lineRange(start: number, end: number): string {
  return start === end ? String(start) : `${start}–${end}`;
}

type Translate = ReturnType<typeof useTranslation>['t'];
export function evidenceLocationLabel(locator: EvidenceLocator, t: Translate): string {
  switch (locator.kind) {
    case 'text': return `${t('preview.lines')} ${lineRange(locator.lineStart, locator.lineEnd)}`;
    case 'pdf': {
      const page = `${t('preview.page')} ${locator.page}`;
      return locator.extractedLineStart && locator.extractedLineEnd
        ? `${page} · ${t('citation.extractedLines', { range: lineRange(locator.extractedLineStart, locator.extractedLineEnd) })}`
        : page;
    }
    case 'document': return locator.table && locator.row ? t('citation.tableRow', { table: locator.table, row: locator.row }) : t('citation.paragraph', { number: locator.paragraph });
    case 'sheet': return `${locator.sheet}!${locator.range}`;
    case 'slide': return t('citation.slide', { number: locator.slide });
    case 'media': return `${(locator.startMs / 1000).toFixed(1)}–${(locator.endMs / 1000).toFixed(1)} s`;
    case 'extracted': return locator.section;
    default: return t('citation.locationUnknown');
  }
}
