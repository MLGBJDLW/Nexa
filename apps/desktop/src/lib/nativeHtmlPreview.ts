import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import type { HtmlPreviewLease } from '../features/preview/filePreviewContext';

export async function createNativeHtmlPreview(html: string): Promise<HtmlPreviewLease> {
  const path = await invoke<string>('create_chat_ui_preview_cmd', {html});
  const id = path.split('/').pop()!;
  return {url: convertFileSrc('', 'nexa-ui') + path.replace(/^\//, ''),
    release: () => { void invoke('release_chat_ui_preview_cmd', {id}).catch(() => {}); }};
}
