import { useRef, useState } from 'react';
import { FileText, MessagesSquare } from 'lucide-react';
import { useTranslation } from '../../i18n';
import { applyVoiceDictationEvent, type VoiceDraftSession } from '../voice/voiceDraftProjection';
import { MobileDialog } from './MobileChatShell';
import { NativeDictationInput } from './NativeDictationInput';
import type { NativeSpeech } from './remoteClient';

export function MobileMeeting({speech, open, onClose, onInsert}: {
  speech: NativeSpeech; open: boolean; onClose: () => void; onInsert: (text: string) => void;
}) {
  const {t} = useTranslation();
  const [text, setText] = useState('');
  const [recording, setRecording] = useState(false);
  const latest = useRef(text); latest.current = text;
  const session = useRef<VoiceDraftSession | null>(null);
  function insert(summary: boolean) {
    const transcript = latest.current.trim();
    if (!transcript || recording) return;
    onInsert(summary ? [t('live.handoff'), t('remote.meetingInstructions'), transcript].join('\n\n') : transcript);
    latest.current = ''; setText(''); onClose();
  }
  return <MobileDialog open={open} title={t('remote.meetingNotes')} onClose={onClose}>
    <div className="mobile-meeting">
      <div className="mobile-meeting-recorder"><span>{t(recording ? 'live.listening' : 'live.microphone')}</span>
        {open && <NativeDictationInput speech={speech} disabled={false} onBusy={setRecording} onEvent={event => {
          const next = applyVoiceDictationEvent(latest.current, session.current, event);
          session.current = next.session; latest.current = next.draft; setText(next.draft);
        }}/>}
      </div>
      <label>{t('live.observations')}<textarea rows={7} value={text} onChange={event => { latest.current = event.target.value; setText(event.target.value); }}/></label>
      <button className="mobile-primary" disabled={recording || !text.trim()} onClick={() => insert(true)}><FileText size={17}/>{t('live.summarize')}</button>
      <button className="mobile-drawer-new" disabled={recording || !text.trim()} onClick={() => insert(false)}><MessagesSquare size={17}/>{t('live.sendToChat')}</button>
    </div>
  </MobileDialog>;
}
