//! A microphone recording outlives individual provider sockets. Unconfirmed
//! PCM is journaled on disk and replayed after reconnect; utterance commits
//! advance the audio checkpoint, never stop the microphone.
use super::*;
use std::time::Instant;
use tokio::io::AsyncWriteExt;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
const OPENAI_ROTATE_AFTER: Duration = Duration::from_secs(55 * 60);
const JOURNAL_CHECKPOINT_BYTES: u64 = 1024 * 1024;
const WRITE_TIMEOUT: Duration = Duration::from_secs(15);

pub(super) async fn connect(
    config: &nexa_core::app_settings::SpeechToTextConfig,
    dialect: RealtimeDialect,
    task_id: &str,
) -> Result<Socket, String> {
    let endpoint = if dialect == RealtimeDialect::DashScopeTask {
        nexa_core::dashscope_speech::task_endpoint(config.base_url.as_deref().unwrap_or_default())?
    } else {
        build_realtime_endpoint(
            config.base_url.as_deref().unwrap_or_default(),
            &config.model,
        )?
    };
    let mut request = endpoint
        .into_client_request()
        .map_err(|error| format!("Invalid Realtime WebSocket request: {error}"))?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {}", config.api_key.trim())
            .parse()
            .map_err(|_| "Invalid realtime authorization header")?,
    );
    request.headers_mut().insert(
        "User-Agent",
        nexa_core::USER_AGENT
            .parse()
            .map_err(|_| "Invalid User-Agent header")?,
    );
    let (mut socket, _) = tokio::time::timeout(
        SESSION_READY_TIMEOUT,
        tokio_tungstenite::connect_async(request),
    )
    .await
    .map_err(|_| "Timed out connecting to realtime transcription")?
    .map_err(|error| format!("Unable to connect to realtime transcription: {error}"))?;
    // gpt-live-transcribe requires client commits; server/semantic VAD is not
    // supported even when this session belongs to a Live meeting.
    socket
        .send(Message::Text(
            build_session_update(dialect, &config.model, config.language.as_deref(), task_id)
                .to_string()
                .into(),
        ))
        .await
        .map_err(|error| format!("Unable to configure realtime transcription: {error}"))?;
    wait_for_session_ready(&mut socket, dialect, task_id).await?;
    Ok(socket)
}

struct AudioJournal {
    file: tokio::fs::File,
    origin: u64,
    confirmed: u64,
    end: u64,
    sent: u64,
}
impl AudioJournal {
    fn new() -> Result<Self, String> {
        let file = tempfile::tempfile()
            .map_err(|error| format!("Unable to retain realtime audio: {error}"))?;
        Ok(Self {
            file: tokio::fs::File::from_std(file),
            origin: 0,
            confirmed: 0,
            end: 0,
            sent: 0,
        })
    }
    async fn append(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.file
            .seek(SeekFrom::Start(self.end - self.origin))
            .await
            .map_err(|e| e.to_string())?;
        self.file
            .write_all(bytes)
            .await
            .map_err(|e| format!("Unable to retain realtime audio: {e}"))?;
        self.end += bytes.len() as u64;
        Ok(())
    }
    async fn next(&mut self, maximum: usize) -> Result<Vec<u8>, String> {
        let length = (self.end - self.sent).min(maximum as u64) as usize;
        let mut bytes = vec![0; length];
        self.file
            .seek(SeekFrom::Start(self.sent - self.origin))
            .await
            .map_err(|e| e.to_string())?;
        self.file
            .read_exact(&mut bytes)
            .await
            .map_err(|e| e.to_string())?;
        Ok(bytes)
    }
    async fn confirm(&mut self, end: u64) -> Result<(), String> {
        self.confirmed = end.min(self.sent);
        if self.confirmed == self.end {
            self.file.set_len(0).await.map_err(|e| e.to_string())?;
            self.origin = self.end;
        } else if self.confirmed - self.origin >= JOURNAL_CHECKPOINT_BYTES {
            // Reclaim only acknowledged PCM. The tail remains byte-identical
            // and the file handle's lifetime owns cleanup on cancel or exit.
            let replacement = tempfile::tempfile().map_err(|e| e.to_string())?;
            let mut replacement = tokio::fs::File::from_std(replacement);
            self.file
                .seek(SeekFrom::Start(self.confirmed - self.origin))
                .await
                .map_err(|e| e.to_string())?;
            tokio::io::copy(
                &mut (&mut self.file).take(self.end - self.confirmed),
                &mut replacement,
            )
            .await
            .map_err(|e| e.to_string())?;
            self.file = replacement;
            self.origin = self.confirmed;
        }
        Ok(())
    }
}

#[derive(Default)]
struct UtteranceBoundary {
    frames: u64,
    silent_frames: u64,
    speech: bool,
}
impl UtteranceBoundary {
    fn append(&mut self, pcm: &[u8], sample_rate: u32) -> bool {
        let frames = pcm.len() as u64 / 2;
        let speech = pcm
            .chunks_exact(2)
            .any(|sample| i16::from_le_bytes([sample[0], sample[1]]).unsigned_abs() >= 400);
        self.frames += frames;
        self.speech |= speech;
        self.silent_frames = if speech {
            0
        } else {
            self.silent_frames + frames
        };
        // A processing segment may end during continuous speech, but the
        // logical recording, microphone, journal and transcript stay alive.
        (self.speech && self.silent_frames >= u64::from(sample_rate) * 2 / 5)
            || self.frames >= u64::from(sample_rate) * 30
    }
}

fn join_transcript(prefix: &str, suffix: &str) -> String {
    match (prefix.trim().is_empty(), suffix.trim().is_empty()) {
        (true, _) => suffix.to_owned(),
        (_, true) => prefix.to_owned(),
        _ => format!("{} {}", prefix.trim_end(), suffix.trim_start()),
    }
}
fn permanent_provider_error(message: &str) -> bool {
    let lower = message.to_lowercase();
    [
        "unauthorized",
        "authentication",
        "api key",
        "permission",
        "quota exhausted",
        "insufficient_quota",
        "unsupported",
        "invalid model",
        "401 unauthorized",
        "403 forbidden",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}
fn should_rotate(dialect: RealtimeDialect, elapsed: Duration) -> bool {
    dialect == RealtimeDialect::OpenAi && elapsed >= OPENAI_ROTATE_AFTER
}

pub(super) async fn run(
    config: nexa_core::app_settings::SpeechToTextConfig,
    dialect: RealtimeDialect,
    mut task_id: String,
    mut socket: Socket,
    mut commands: mpsc::Receiver<RealtimeCommand>,
    events: &RealtimeEventSink,
    session_id: &str,
) -> Result<(), String> {
    let mut audio = AudioJournal::new()?;
    let mut prefix = String::new();
    let mut transcript = RealtimeTranscript::default();
    let mut sequence = 0;
    let mut pending_final = None;
    let mut pending_commit: Option<u64> = None;
    let mut expected_item: Option<String> = None;
    let mut committed_at: Option<Instant> = None;
    let mut acknowledged_items = std::collections::HashSet::new();
    let mut boundary = UtteranceBoundary::default();
    let mut connected_at = Instant::now();
    let mut retry_delay = Duration::from_millis(250);
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let work: Result<(), String> = async {
        loop {
            let mut reconnect = false;
            let mut checkpoint = false;
            tokio::select! {
                command = commands.recv() => match command {
                    Some(RealtimeCommand::Append(bytes)) => audio.append(&bytes).await?,
                    Some(RealtimeCommand::Finish(response)) => {
                        if pending_final.is_some() { let _ = response.send(Err("Realtime transcription is already finishing".into())); }
                        else { pending_final = Some(response); }
                    }
                    None => return Ok(()),
                },
                _ = tick.tick() => {
                    if pending_commit.is_none() && audio.sent < audio.end {
                        let bytes = audio.next(dialect.sample_rate() as usize / 5).await?; // <= 100 ms PCM16
                        let commit = boundary.append(&bytes, dialect.sample_rate());
                        match tokio::time::timeout(WRITE_TIMEOUT, socket.send(audio_message(dialect, &bytes))).await {
                            Ok(Ok(())) => { audio.sent += bytes.len() as u64; retry_delay = Duration::from_millis(250); }
                            _ => reconnect = true,
                        }
                        if !reconnect && dialect == RealtimeDialect::OpenAi && commit { pending_commit = Some(audio.sent); }
                    }
                    if !reconnect && pending_commit.is_none() && pending_final.is_some() && audio.sent == audio.end {
                        if audio.sent == audio.confirmed {
                            resolve_pending_final(&mut pending_final, Ok(prefix.clone()));
                            return Ok(());
                        }
                        pending_commit = Some(audio.sent);
                    }
                    if !reconnect && should_rotate(dialect, connected_at.elapsed()) && pending_commit.is_none() {
                        if audio.sent == audio.confirmed { reconnect = true; }
                        else { pending_commit = Some(audio.sent); }
                    }
                    // Commit exactly once at an explicit audio boundary. We
                    // retain later microphone audio on disk while finals arrive.
                    if !reconnect && pending_commit.is_some() && expected_item.is_none() {
                        expected_item = Some(String::new());
                        committed_at = Some(Instant::now());
                        for message in finish_messages(dialect, &task_id) {
                            if !matches!(tokio::time::timeout(WRITE_TIMEOUT, socket.send(Message::Text(message.to_string().into()))).await, Ok(Ok(()))) { reconnect = true; break; }
                        }
                    }
                    if committed_at.is_some_and(|at| at.elapsed() >= FINAL_TRANSCRIPT_TIMEOUT) {
                        reconnect = true;
                    }
                },
                incoming = socket.next() => match incoming {
                    Some(Ok(Message::Text(text))) => {
                        let Ok(event) = serde_json::from_str::<Value>(&text) else { continue; };
                        if dialect == RealtimeDialect::DashScopeTask && event.pointer("/header/task_id").and_then(Value::as_str) != Some(task_id.as_str()) { continue; }
                        if event["type"] == "input_audio_buffer.committed" {
                            expected_item = event["item_id"].as_str().map(str::to_owned).or(expected_item);
                        }
                        match parse_server_event(dialect, &event) {
                            ParsedRealtimeServerEvent::Interim { utterance_id, text, update } => {
                                if utterance_id.as_ref().is_some_and(|id| acknowledged_items.contains(id)) { continue; }
                                transcript.update(utterance_id.as_deref(), &text, update == TranscriptUpdate::AppendDelta, false);
                                let current = transcript.snapshot();
                                let logical_id = format!("audio-{}", audio.confirmed);
                                emit_live_transcript(events, "interim", Some(&current), Some("replaceSnapshot"), Some(&logical_id));
                                emit_realtime_event(events, session_id, &mut sequence, "interim", Some(&join_transcript(&prefix, &current)), Some(TranscriptUpdate::ReplaceSnapshot), Some(&logical_id));
                            }
                            ParsedRealtimeServerEvent::Final { utterance_id, text } => {
                                if utterance_id.as_ref().is_some_and(|id| acknowledged_items.contains(id)) { continue; }
                                transcript.update(utterance_id.as_deref(), &text, false, true);
                                if dialect == RealtimeDialect::OpenAi {
                                    checkpoint = pending_commit.is_some() && expected_item.as_deref().is_none_or(|id| id.is_empty() || Some(id) == utterance_id.as_deref());
                                    if checkpoint { if let Some(id) = utterance_id { acknowledged_items.insert(id); } }
                                } else if pending_commit.is_none() && audio.sent - audio.confirmed >= JOURNAL_CHECKPOINT_BYTES {
                                    // Qwen ASR has no recording duration cap.
                                    // A sentence boundary checkpoints the transport
                                    // journal so reconnection replays a short tail.
                                    pending_commit = Some(audio.sent);
                                }
                                if !checkpoint {
                                    let current = transcript.snapshot();
                                    let logical_id = format!("audio-{}", audio.confirmed);
                                    emit_live_transcript(events, "interim", Some(&current), Some("replaceSnapshot"), Some(&logical_id));
                                    emit_realtime_event(events, session_id, &mut sequence, "interim", Some(&join_transcript(&prefix, &current)), Some(TranscriptUpdate::ReplaceSnapshot), Some(&logical_id));
                                }
                            }
                            ParsedRealtimeServerEvent::SessionFinished => { checkpoint = true; }
                            ParsedRealtimeServerEvent::Error(message) => {
                                if permanent_provider_error(&message) || pending_final.is_some() { return Err(message); }
                                reconnect = true;
                            }
                            ParsedRealtimeServerEvent::Other => {}
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if !matches!(tokio::time::timeout(WRITE_TIMEOUT, socket.send(Message::Pong(payload))).await, Ok(Ok(()))) { reconnect = true; }
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => reconnect = true,
                    Some(Ok(_)) => {}
                }
            }
            if checkpoint {
                let text = transcript.finish()?;
                let logical_id = format!("audio-{}", audio.confirmed);
                let end = pending_commit.take().unwrap_or(audio.sent);
                prefix = join_transcript(&prefix, &text);
                audio.confirm(end).await?;
                emit_live_transcript(events, "final", Some(&text), Some("replaceSnapshot"), Some(&logical_id));
                emit_realtime_event(events, session_id, &mut sequence, "final", Some(&prefix), Some(TranscriptUpdate::ReplaceSnapshot), Some(&logical_id));
                transcript = RealtimeTranscript::default();
                expected_item = None;
                committed_at = None;
                boundary = UtteranceBoundary::default();
                if pending_final.is_some() && audio.confirmed == audio.end {
                    resolve_pending_final(&mut pending_final, Ok(prefix.clone()));
                    return Ok(());
                }
                reconnect = dialect.waits_for_session_finished() || should_rotate(dialect, connected_at.elapsed());
            }
            if reconnect {
                audio.sent = audio.confirmed;
                pending_commit = None;
                expected_item = None;
                committed_at = None;
                acknowledged_items.clear();
                boundary = UtteranceBoundary::default();
                transcript = RealtimeTranscript::default();
                emit_realtime_event(events, session_id, &mut sequence, "reconnecting", None, None, None);
                loop {
                    task_id = Uuid::new_v4().simple().to_string();
                    let delay = retry_delay;
                    let reconnect = async {
                        tokio::time::sleep(delay).await;
                        connect(&config, dialect, &task_id).await
                    };
                    tokio::pin!(reconnect);
                    let result = loop {
                        tokio::select! {
                            result = &mut reconnect => break result,
                            command = commands.recv() => match command {
                                Some(RealtimeCommand::Append(bytes)) => audio.append(&bytes).await?,
                                Some(RealtimeCommand::Finish(response)) => {
                                    if pending_final.is_none() { pending_final = Some(response); }
                                    else { let _ = response.send(Err("Realtime transcription is already finishing".into())); }
                                }
                                None => return Ok(()),
                            }
                        }
                    };
                    match result {
                        Ok(connected) => {
                            socket = connected;
                            connected_at = Instant::now();
                            emit_realtime_event(events, session_id, &mut sequence, "connected", None, None, None);
                            break;
                        }
                        Err(message) if permanent_provider_error(&message) => return Err(message),
                        Err(_) => retry_delay = (retry_delay * 2).min(Duration::from_secs(15)),
                    }
                }
            }
        }
    }.await;
    if let Err(message) = &work {
        resolve_pending_final(&mut pending_final, Err(message.clone()));
        emit_realtime_event(
            events,
            session_id,
            &mut sequence,
            "error",
            Some(message),
            None,
            None,
        );
    }
    let _ = tokio::time::timeout(Duration::from_secs(1), socket.close(None)).await;
    work
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn audio_checkpoint_replays_only_unconfirmed_pcm_after_reconnect() {
        let mut journal = AudioJournal::new().unwrap();
        journal.append(&[1, 0, 2, 0, 3, 0]).await.unwrap();
        journal.sent = 4;
        journal.confirm(4).await.unwrap();
        journal.append(&[4, 0]).await.unwrap();
        journal.sent = journal.confirmed;
        assert_eq!(journal.next(1024).await.unwrap(), [3, 0, 4, 0]);
        journal.sent = 8;
        journal.confirm(8).await.unwrap();
        journal.append(&[5, 0]).await.unwrap();
        assert_eq!(journal.next(1024).await.unwrap(), [5, 0]);
    }
    #[test]
    fn provider_session_rotation_is_not_a_recording_duration_limit() {
        assert!(should_rotate(
            RealtimeDialect::OpenAi,
            Duration::from_secs(60 * 60)
        ));
        assert!(!should_rotate(
            RealtimeDialect::DashScope,
            Duration::from_secs(24 * 60 * 60)
        ));
        assert!(!should_rotate(
            RealtimeDialect::DashScopeTask,
            Duration::from_secs(24 * 60 * 60)
        ));
        let mut vad = UtteranceBoundary::default();
        let speech: Vec<u8> = (0..2_400).flat_map(|_| 1000_i16.to_le_bytes()).collect();
        assert!(!vad.append(&speech, 24_000));
        assert!(!vad.append(&vec![0; 24_000 / 5], 24_000));
        assert!(vad.append(&vec![0; 24_000], 24_000));
    }
}
