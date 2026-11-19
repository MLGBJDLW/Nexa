import { Channel, invoke } from '@tauri-apps/api/core';
import { openScreenCapture } from '../../lib/screenCapture';
import { encodeScreenCaptureFrame } from '../../lib/screenCaptureFrame';

/** The OS owns source selection. Encode only in response to a paired viewer's pull. */
export async function shareFromSystemPicker(deviceId: string, signal: AbortSignal) {
  const source = `picker:${crypto.randomUUID()}`;
  const capture = await openScreenCapture();
  const video = document.createElement('video');
  const canvas = document.createElement('canvas');
  const track = capture.stream.getVideoTracks()[0];
  const reply = (requestId: string | null, image: {base64: string; width: number; height: number} | null) =>
    invoke('remote_screen_picker_reply_cmd', {deviceId, source, requestId, image});
  const dispose = () => {
    track?.removeEventListener('ended', ended);
    capture.stop();
    video.pause();
    video.srcObject = null;
  };
  const ended = () => { dispose(); void reply(null, null).catch(() => {}); };
  const onmessage = (requestId: string | null) => {
    if (requestId === null) { dispose(); return; }
    let image = null;
    try {
      const frame = encodeScreenCaptureFrame(video, video.videoWidth, video.videoHeight, canvas);
      if (frame) image = {base64: frame.base64, width: canvas.width, height: canvas.height};
    } catch { /* The OS can end capture between a pull and the canvas draw. */ }
    void reply(requestId, image).catch(dispose);
  };
  signal.addEventListener('abort', ended, {once: true});
  try {
    signal.throwIfAborted();
    track?.addEventListener('ended', ended, {once: true});
    video.muted = true;
    video.playsInline = true;
    video.srcObject = capture.stream;
    await video.play();
    signal.throwIfAborted();
    if (track?.readyState !== 'live') throw new Error('The selected screen is unavailable.');
    await invoke('remote_screen_share_cmd', {deviceId, source, control: false, pickerEvents: new Channel(onmessage)});
  } catch (error) {
    dispose();
    throw error;
  } finally {
    signal.removeEventListener('abort', ended);
  }
}
