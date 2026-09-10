use super::{
    protocol::{self, Decoder, NativeEvent},
    Frame, Input, LiveOptions, LivePhase, NativeLiveConfig, NativeLiveProtocol, Session,
};
use futures::{SinkExt, StreamExt};
use serde_json::Value;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::{
    tungstenite::{protocol::WebSocketConfig, Message},
    MaybeTlsStream, WebSocketStream,
};

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

pub(super) async fn run(
    session: &Session,
    config: NativeLiveConfig,
    options: LiveOptions,
    input: mpsc::Receiver<Input>,
    frames: watch::Receiver<Option<Frame>>,
) -> Result<(), String> {
    let request = config.request()?;
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut limits = WebSocketConfig::default();
    limits.max_message_size = Some(2 * 1024 * 1024);
    limits.max_frame_size = Some(2 * 1024 * 1024);
    let (socket, _) = tokio::time::timeout(
        Duration::from_secs(15),
        tokio_tungstenite::connect_async_with_config(request, Some(limits), false),
    )
    .await
    .map_err(|_| "Live connection timed out")?
    .map_err(|error| match error {
        tokio_tungstenite::tungstenite::Error::Http(response) => format!(
            "Live connection returned HTTP {}; check the selected model and account access",
            response.status()
        ),
        _ => "Unable to connect to the live provider".into(),
    })?;
    run_socket(session, &config, options, socket, input, frames).await
}

pub(super) async fn run_socket(
    session: &Session,
    config: &NativeLiveConfig,
    options: LiveOptions,
    mut socket: Socket,
    mut input: mpsc::Receiver<Input>,
    mut frames: watch::Receiver<Option<Frame>>,
) -> Result<(), String> {
    socket
        .send(Message::Text(
            config
                .setup(&super::observation_instructions(&options.purpose))
                .to_string()
                .into(),
        ))
        .await
        .map_err(|_| "Unable to configure live session")?;
    let mut decoder = Decoder::default();
    tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(message) = socket.next().await {
            let message = message.map_err(|_| "Live connection closed during setup".to_string())?;
            if let Message::Ping(bytes) = message {
                socket
                    .send(Message::Pong(bytes))
                    .await
                    .map_err(|_| "Live connection closed during setup")?;
                continue;
            }
            if let Some(value) = decode(&message)? {
                for event in decoder.parse(config.protocol, &value) {
                    match event {
                        NativeEvent::Ready => return Ok(()),
                        NativeEvent::Error(error) => return Err(config.safe_error(&error)),
                        _ => {}
                    }
                }
            }
        }
        Err("Live connection closed before the model was ready".to_string())
    })
    .await
    .map_err(|_| "The live model did not acknowledge the session configuration")??;
    session.phase(LivePhase::Listening, None);
    let (mut sink, mut source) = socket.split();
    let mut ticker = tokio::time::interval(options.interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut responding = false;
    let mut needs_response = false;
    let mut response_started = Instant::now();
    let mut previous_frame_id: Option<String> = None;
    let mut frame_sequence = 0;
    let mut pending_audio_bytes = 0;
    loop {
        let mut outbound = Vec::new();
        tokio::select! {
            message = source.next() => {
                let message = message.ok_or("Live connection ended; reconnect to continue")?.map_err(|_| "Live provider connection was interrupted")?;
                if let Message::Ping(bytes) = message { sink.send(Message::Pong(bytes)).await.map_err(|_| "Live connection closed")?; continue; }
                if let Some(value) = decode(&message)? {
                    for event in decoder.parse(config.protocol, &value) {
                        match event {
                            NativeEvent::Entry { id, role, text, append, complete } => session.entry(&id, role, &text, append, complete),
                            NativeEvent::Started => { responding = true; response_started = Instant::now(); session.phase(LivePhase::Analyzing, None); }
                            NativeEvent::AudioCommitted => needs_response = true,
                            NativeEvent::TurnComplete => { responding = false; session.response_time(response_started); session.phase(LivePhase::Listening, None); }
                            NativeEvent::Error(error) => return Err(config.safe_error(&error)),
                            NativeEvent::Ready => {}
                        }
                    }
                }
            }
            command = input.recv() => match command {
                Some(Input::Audio(pcm)) => { pending_audio_bytes += pcm.len(); outbound.push(protocol::audio(config.protocol, &pcm)); }
                Some(Input::Text { id, text, append, complete }) => {
                    let _ = (id, append);
                    if complete && !text.trim().is_empty() { outbound.push(protocol::text(config.protocol, &text)); needs_response = true; }
                }
                None => return Ok(()),
            },
            _ = ticker.tick() => {
                if session.expired() { return Ok(()); }
                if responding && response_started.elapsed() > Duration::from_secs(60) { return Err("The live model did not finish its response; reconnect to continue".into()); }
                if !responding {
                    let frame = frames.borrow_and_update().clone();
                    if let Some(frame) = frame.filter(|frame| frame.sequence > frame_sequence && (config.protocol != NativeLiveProtocol::QwenRealtime || pending_audio_bytes >= 3200)) {
                        let id = format!("frame_{}", uuid::Uuid::new_v4().simple());
                        outbound.push(protocol::frame(config.protocol, &frame, &id));
                        if config.protocol == NativeLiveProtocol::OpenAiRealtime {
                            if let Some(previous) = previous_frame_id.replace(id) { outbound.push(serde_json::json!({"type":"conversation.item.delete","item_id":previous})); }
                        }
                        session.frame_sent(&frame, &mut frame_sequence);
                        needs_response = true;
                    }
                    if config.protocol == NativeLiveProtocol::QwenRealtime && pending_audio_bytes >= 3200 {
                        outbound.push(serde_json::json!({"type":"input_audio_buffer.commit"}));
                        pending_audio_bytes = 0;
                        needs_response = true;
                    }
                }
            }
        }
        if needs_response && !responding {
            outbound.push(protocol::respond(config.protocol));
            responding = true;
            needs_response = false;
            response_started = Instant::now();
            session.phase(LivePhase::Analyzing, None);
        }
        for value in outbound {
            tokio::time::timeout(
                Duration::from_secs(5),
                sink.send(Message::Text(value.to_string().into())),
            )
            .await
            .map_err(|_| "Live provider is not accepting input; reconnect to continue")?
            .map_err(|_| "Unable to stream input to the live provider")?;
        }
    }
}

fn decode(message: &Message) -> Result<Option<Value>, String> {
    match message {
        Message::Text(text) => serde_json::from_str(text)
            .map(Some)
            .map_err(|_| "Invalid live provider event".into()),
        Message::Binary(bytes) => serde_json::from_slice(bytes)
            .map(Some)
            .map_err(|_| "Invalid live provider event".into()),
        Message::Close(_) => Err("Live provider disconnected; reconnect to continue".into()),
        _ => Ok(None),
    }
}
