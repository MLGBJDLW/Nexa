# Voice and Live

Dictation writes recognized speech into the editable chat composer. Live is a
separate observation session using microphone, camera, or screen input with a
configured model connection. Both depend on actual adapter capabilities and
browser/device permissions.

## Dictation

Configure speech recognition in desktop Settings, then use the composer
microphone. An interim-capable adapter updates the draft during recording;
a final-only adapter publishes text only when its operation finishes.

Manual edits own the text the user corrected. Later provider hypotheses must
not overwrite that correction or duplicate already finalized utterances.
Changing a model name cannot turn a batch HTTP adapter into a realtime stream.
Phone dictation uses the desktop speech configuration and preserves recognized
text if the microphone transport is interrupted.

## Start a Live session

1. Open Live on the desktop or a [paired phone](remote-access.md).
2. Select an available connection and input mode. Enable only the inputs needed
   for the observation.
3. Grant browser/OS microphone, camera, or screen permissions. Screen capture
   is available only where the browser and device provide it.
4. Start and wait for the model connection to become ready. Inspect connection
   state and observation text while capturing.
5. Stop capture, review the text record, and request a summary or continue in Chat.

Microphone and camera access in the phone browser require a secure context.
Use a trusted public HTTPS route, trusted LAN certificate, or the documented
localhost forwarding setup. An untrusted LAN certificate is not enough to
enable microphone capture.

## Connection modes

| Mode | Execution and requirements |
| --- | --- |
| Native realtime | Nexa adapters for OpenAI Realtime, Gemini Live, and Qwen Omni Realtime; the exact endpoint must support the protocol |
| Vision plus transcription | A configured API vision model observes selected frames and streaming transcription supplies speech text |
| Summary/handoff | The observation and final summary models can be selected separately; a subscription agent can receive the record through Continue in Chat |

Qwen native realtime requires the workspace's supported realtime WebSocket
endpoint. Private compatible URLs are not assigned a public protocol solely
because their model names resemble a known family. Subscription login remains
on its [subscription execution path](SUBSCRIPTION_AGENTS.md); it is not a native
Live API credential.

## Runtime contract

### Reading a session

Live separates the microphone transcript, AI observations, and capture notices.
Transcript and observation panels follow new content until the user scrolls up.
The summary has its own panel and an expandable, keyboard-accessible reading
window. Stop capture before generating a summary. Desktop and paired-phone
clients share this layout.

### Speaker attribution (verified 2026-09-13)

The current Nexa speech adapters do **not** provide live speaker diarization.
Utterance IDs identify transcript segments, not people. Nexa does not infer
speaker numbers or identities from wording or AI observations.

This audit covers all ten presets in
[the speech catalog](../shared/stt-provider-presets.json), plus native Live.
A text-model integration does not imply access to a company's separate speech
products.

| Existing route | Live speaker labels | Other capability / boundary |
| --- | --- | --- |
| OpenAI file / Live transcription / native Realtime | Not supported by the current live interfaces | The separate `gpt-4o-transcribe-diarize` file endpoint returns speaker segments; changing a Realtime model name does not enable it. [Official guide](https://developers.openai.com/api/docs/guides/speech-to-text#speaker-diarization) |
| Alibaba Qwen3 ASR / realtime ASR / Qwen Omni realtime | No supported speaker-label contract in the current adapters | Alibaba documents diarization for file transcription and offline Fun-ASR; its streaming ASR matrix marks diarization unsupported. [Model matrix](https://help.aliyun.com/en/model-studio/asr-model/) |
| Gemini Live | No speaker labels from Nexa's input transcription adapter | Google's live transcription guide directs diarization to its non-streaming endpoint. [Live transcription](https://ai.google.dev/gemini-api/docs/live-api/live-transcribe) |
| Groq Whisper | No live adapter or documented speaker-label option in the current file API | Word/segment timestamps are not speaker IDs. [Speech API](https://console.groq.com/docs/speech-to-text) |
| SiliconFlow SenseVoice / TeleSpeech | No live adapter or speaker metadata in the documented endpoint | Its response is a text string. [Transcription API](https://docs.siliconflow.cn/docs/api/audio-transcriptions-post) |
| Local Candle Whisper | No speaker model in Nexa's implementation | Speech recognition alone does not perform diarization. |
| Local sherpa SenseVoice / Zipformer | No speaker model in Nexa's adapters | Upstream offers a separate pipeline needing segmentation and embedding models. [Speaker diarization](https://k2-fsa.github.io/sherpa/onnx/speaker-diarization/index.html) |
| Custom OpenAI-compatible transcription | Provider capability unknown; Nexa accepts final text only | HTTP transcription compatibility does not establish live speaker-label support. |

Reliable live attribution needs a dedicated streaming diarization adapter or a
local segmentation/embedding pipeline. It must carry session-scoped speaker IDs
and timestamps through events, reconnects, archives, summaries, and remote
clients, and retain unknown/overlapping voices explicitly. Real multi-speaker
audio must validate it before release; offline diarization is not a substitute.

The [core Live manager](../crates/core/src/live_analysis/mod.rs) owns session
state, input bounds, observations, and lifecycle. Desktop and phone frontends
use transport adapters to reach the same desktop-hosted service.

- Input starts only after the selected model reports ready.
- Audio is queued within explicit bounds. Remote acknowledgements can be in
  flight concurrently while preserving ordered delivery; they are not a reason
  to retain unbounded audio.
- Pending frames are replaceable: a newer frame supersedes stale work instead
  of growing a backlog.
- Capture pauses during transport loss. Reconnection can resume the same
  session within its lease; expiration ends that capture session and requires
  a new start. Interruptions remain visible in the observation record.
- Leaving Live, revoking the device, or a capture failure releases the acquired
  microphone/camera resources. Cleanup also covers inputs acquired while an
  asynchronous start is still pending.
- Stop persists bounded text observations and summaries, not raw audio/video.
  Records retain at most 160 entries of at most 8,000 characters each and report
  omitted older entries. Summaries must account for gaps and truncation.

Transient frames and audio still travel to the selected provider when its mode
uses them. Text-only local persistence is not a claim that no media left the
device or that the external provider has the same retention policy.

## Troubleshooting and verification

| Symptom | Next check |
| --- | --- |
| No microphone permission | Check browser/OS permission and trusted HTTPS/localhost context |
| No suitable connection | Configure the actual native protocol or a vision model plus streaming speech recognition |
| Capture ends after switching networks | Check lease expiry and desktop availability, then start a new session if recovery expired |
| Missing observations | Inspect interruption/truncation markers and provider capability before treating a summary as complete |

Implementation: [desktop commands](../apps/desktop/src-tauri/src/commands/live.rs),
[native protocol normalization](../crates/core/src/live_analysis/protocol.rs),
[desktop/phone transport interfaces](../apps/desktop/src/features/live/liveTransport.ts),
and [composer speech projection](../apps/desktop/src/features/voice/voiceDraftProjection.ts).

Contract tests include [core Live tests](../crates/core/src/live_analysis/tests.rs)
and [audio delivery tests](../apps/desktop/tests/live-delivery.test.ts).
Native microphone quality, physical phone browser support, and live provider
entitlement require separate acceptance; mocked transport tests cannot prove them.
