import { createContext, lazy, Suspense, useCallback, useContext, useEffect, useState, type ReactNode } from 'react';
import type { ImageAttachment } from '../../types/conversation';

export interface ImageEditSubmission {
  prompt: string;
  attachments: ImageAttachment[];
  artifacts: { kind: 'imageEdit'; version: 1; original: string; mask: string; guide: string };
}
const Editor = lazy(() => import('./ImageEditorDialog'));
const ImageEditContext = createContext<((image: { src: string; name: string }) => void) | null>(null);
export const useImageEditor = () => useContext(ImageEditContext);

export function ImageEditProvider({ children, conversationId, disabled, onSubmit }: {
  children: ReactNode; conversationId: string | null; disabled?: boolean;
  onSubmit: (submission: ImageEditSubmission) => Promise<void>;
}) {
  const [image, setImage] = useState<{ src: string; name: string } | null>(null);
  useEffect(() => setImage(null), [conversationId]);
  const open = useCallback((value: { src: string; name: string }) => setImage(value), []);
  return <ImageEditContext.Provider value={disabled ? null : open}>
    {children}
    {image && <Suspense fallback={null}><Editor key={`${conversationId}:${image.src}`} image={image} onClose={() => setImage(null)} onSubmit={onSubmit} /></Suspense>}
  </ImageEditContext.Provider>;
}
