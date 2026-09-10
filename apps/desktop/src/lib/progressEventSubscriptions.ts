import { listen } from '@tauri-apps/api/event';
import { progressStore } from './progressStore';
import type { CompileProgress } from './progressStore';
import type { DownloadProgress, ScanProgress, BatchProgress, FtsProgress } from '../types/ingest';
import type { OcrDownloadProgress } from '../types/ocr';
import type { VideoDownloadProgress, FfmpegDownloadProgress } from '../types/video';
import type { ProcessingPhase } from '../components/media/VideoProcessingProgress';
import type { EventSubscription } from './eventSubscriptions';

/**
 * Progress registrations share the root event connection's readiness and cleanup.
 * Page navigation does not tear them down.
 */
export function progressEventSubscriptions(): EventSubscription[] {
  const subscriptions: EventSubscription[] = [];

  function reg<T>(event: string, handler: (payload: T) => void) {
    subscriptions.push((isActive) => listen<T>(event, (e) => {
      if (isActive()) handler(e.payload);
    }));
  }

  // Model downloads
  reg<DownloadProgress>('model:download-progress', (p) => {
    progressStore.update('modelDownload', p);
  });

  reg<OcrDownloadProgress>('ocr:download-progress', (p) => {
    progressStore.update('ocrDownload', p);
  });

  reg<VideoDownloadProgress>('video:download-progress', (p) => {
    progressStore.update('videoDownload', p);
  });

  reg<FfmpegDownloadProgress>('ffmpeg:download-progress', (p) => {
    progressStore.update('ffmpegDownload', p);
  });

  // Scan / batch
  reg<ScanProgress>('source:scan-progress', (p) => {
    progressStore.update('scanProgress', p);
  });

  reg<BatchProgress>('batch:scan-progress', (p) => {
    progressStore.update('batchProgress', p);
  });

  reg<ScanProgress>('batch:rebuild-progress', (p) => {
    progressStore.update('embedRebuildProgress', p);
  });

  reg<FtsProgress>('batch:fts-progress', (p) => {
    if (p.phase === 'complete') {
      progressStore.update('ftsProgress', null);
    } else {
      progressStore.update('ftsProgress', p);
    }
  });

  // Compile
  reg<CompileProgress>('compile:progress', (p) => {
    progressStore.update('compileProgress', p);
  });

  // Video processing
  reg<{ phase: ProcessingPhase; progress: number; fileName: string }>(
    'video:processing-progress',
    (p) => {
      if (p.phase === 'complete') {
        progressStore.update('videoProcessing', null);
      } else {
        progressStore.update('videoProcessing', p);
      }
    },
  );

  return subscriptions;
}
