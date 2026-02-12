//! Gemini Live API client for real-time bidirectional audio streaming.
//! Replaces Whisper + edge-tts with a single WebSocket connection.

use anyhow::{anyhow, Result};
use crossbeam_channel::{bounded, Receiver, Sender};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tokio_tungstenite::tungstenite::protocol::frame::CloseFrame;

pub const GEMINI_LIVE_MODEL: &str = "gemini-2.5-flash-native-audio-preview";
pub const SEND_SAMPLE_RATE: u32 = 16000;
pub const RECEIVE_SAMPLE_RATE: u32 = 24000;

/// Events emitted by the Gemini Live session
#[derive(Debug, Clone)]
pub enum GeminiLiveEvent {
    Connected,
    Disconnected,
    AudioOutput(Vec<u8>),
    TextOutput(String),
    InputTranscript(String),
    FunctionCall { id: String, name: String, args: serde_json::Value },
    TurnComplete,
    Interrupted,
    Error(String),
}

/// Commands to control the Gemini Live session
#[derive(Debug, Clone)]
pub enum GeminiLiveCommand {
    Connect { api_key: String, system_instruction: Option<String>, tools: Option<serde_json::Value> },
    Disconnect,
    SendAudio(Vec<u8>),
    SendText(String),
    SendToolResult { id: String, function_name: String, result: serde_json::Value },
}

/// Gemini Live API WebSocket client
pub struct GeminiLiveClient {
    cmd_tx: Sender<GeminiLiveCommand>,
    event_rx: Receiver<GeminiLiveEvent>,
    is_connected: Arc<AtomicBool>,
    is_model_speaking: Arc<AtomicBool>,
}

impl GeminiLiveClient {
    pub fn new() -> Self {
        let (cmd_tx, cmd_rx) = bounded::<GeminiLiveCommand>(64);
        let (event_tx, event_rx) = bounded::<GeminiLiveEvent>(64);
        let is_connected = Arc::new(AtomicBool::new(false));
        let is_model_speaking = Arc::new(AtomicBool::new(false));

        let connected = is_connected.clone();
        let speaking = is_model_speaking.clone();

        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(Self::run_loop(cmd_rx, event_tx, connected, speaking));
        });

        Self {
            cmd_tx,
            event_rx,
            is_connected,
            is_model_speaking,
        }
    }

    pub fn send(&self, cmd: GeminiLiveCommand) {
        let _ = self.cmd_tx.try_send(cmd);
    }

    pub fn poll_events(&self) -> Vec<GeminiLiveEvent> {
        self.event_rx.try_iter().collect()
    }

    /// Convenience method: spawn a new client and immediately connect.
    /// Returns (cmd_tx, event_rx) for integration with VoiceService.
    pub fn spawn(
        api_key: String,
        system_instruction: Option<String>,
        tools: Option<serde_json::Value>,
    ) -> (Sender<GeminiLiveCommand>, Receiver<GeminiLiveEvent>) {
        let client = Self::new();
        client.send(GeminiLiveCommand::Connect {
            api_key,
            system_instruction,
            tools,
        });
        (client.cmd_tx, client.event_rx)
    }

    pub fn is_connected(&self) -> bool {
        self.is_connected.load(Ordering::SeqCst)
    }

    pub fn is_model_speaking(&self) -> bool {
        self.is_model_speaking.load(Ordering::SeqCst)
    }

    async fn run_loop(
        cmd_rx: Receiver<GeminiLiveCommand>,
        event_tx: Sender<GeminiLiveEvent>,
        is_connected: Arc<AtomicBool>,
        is_model_speaking: Arc<AtomicBool>,
    ) {
        let mut session: Option<LiveSession> = None;
        let mut setup_done = false;

        loop {
            // Process commands
            while let Ok(cmd) = cmd_rx.try_recv() {
                match cmd {
                    GeminiLiveCommand::Connect { api_key, system_instruction, tools } => {
                        if session.is_some() {
                            // Already connected, disconnect first
                            session = None;
                            is_connected.store(false, Ordering::SeqCst);
                            setup_done = false;
                        }
                        match LiveSession::connect(&api_key, system_instruction.as_deref(), tools.as_ref()).await {
                            Ok(s) => {
                                session = Some(s);
                                // Only mark connected after setupComplete to avoid sending audio too early.
                                is_connected.store(false, Ordering::SeqCst);
                                setup_done = false;
                            }
                            Err(e) => {
                                let _ = event_tx.try_send(GeminiLiveEvent::Error(e.to_string()));
                            }
                        }
                    }
                    GeminiLiveCommand::Disconnect => {
                        session = None;
                        is_connected.store(false, Ordering::SeqCst);
                        is_model_speaking.store(false, Ordering::SeqCst);
                        setup_done = false;
                        let _ = event_tx.try_send(GeminiLiveEvent::Disconnected);
                    }
                    GeminiLiveCommand::SendAudio(pcm) => {
                        if let Some(ref mut s) = session {
                            if !setup_done {
                                // Ignore audio until setupComplete.
                                continue;
                            }
                            if let Err(e) = s.send_audio(&pcm).await {
                                let _ = event_tx.try_send(GeminiLiveEvent::Error(e.to_string()));
                            }
                        }
                    }
                    GeminiLiveCommand::SendText(text) => {
                        if let Some(ref mut s) = session {
                            if !setup_done {
                                // Ignore text until setupComplete.
                                continue;
                            }
                            if let Err(e) = s.send_text(&text).await {
                                let _ = event_tx.try_send(GeminiLiveEvent::Error(e.to_string()));
                            }
                        }
                    }
                    GeminiLiveCommand::SendToolResult { id, function_name, result } => {
                        if let Some(ref mut s) = session {
                            if !setup_done {
                                continue;
                            }
                            if let Err(e) = s.send_tool_result(&id, &function_name, &result).await {
                                let _ = event_tx.try_send(GeminiLiveEvent::Error(e.to_string()));
                            }
                        }
                    }
                }
            }

            // Receive from WebSocket
            if let Some(ref mut s) = session {
                match s.try_recv().await {
                    Ok(Some(response)) => {
                        if response.setup_complete.is_some() && !setup_done {
                            setup_done = true;
                            is_connected.store(true, Ordering::SeqCst);
                            let _ = event_tx.try_send(GeminiLiveEvent::Connected);
                        }
                        Self::process_response(&response, &event_tx, &is_model_speaking);
                    }
                    Ok(None) => {}
                    Err(e) => {
                        let _ = event_tx.try_send(GeminiLiveEvent::Error(e.to_string()));
                        session = None;
                        is_connected.store(false, Ordering::SeqCst);
                        is_model_speaking.store(false, Ordering::SeqCst);
                        setup_done = false;
                        let _ = event_tx.try_send(GeminiLiveEvent::Disconnected);
                    }
                }
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }
    }

    fn process_response(
        response: &LiveResponse,
        event_tx: &Sender<GeminiLiveEvent>,
        is_model_speaking: &Arc<AtomicBool>,
    ) {
        if let Some(ref server_content) = response.server_content {
            if server_content.interrupted == Some(true) {
                is_model_speaking.store(false, Ordering::SeqCst);
                let _ = event_tx.try_send(GeminiLiveEvent::Interrupted);
                return;
            }

            if let Some(ref model_turn) = server_content.model_turn {
                is_model_speaking.store(true, Ordering::SeqCst);
                for part in &model_turn.parts {
                    if let Some(ref inline_data) = part.inline_data {
                        if inline_data.mime_type == "audio/pcm" {
                            use base64::Engine;
                            if let Ok(audio_bytes) = base64::engine::general_purpose::STANDARD.decode(&inline_data.data) {
                                let _ = event_tx.try_send(GeminiLiveEvent::AudioOutput(audio_bytes));
                            }
                        }
                    }
                    if let Some(ref text) = part.text {
                        let _ = event_tx.try_send(GeminiLiveEvent::TextOutput(text.clone()));
                    }
                }
            }

            if server_content.turn_complete == Some(true) {
                is_model_speaking.store(false, Ordering::SeqCst);
                let _ = event_tx.try_send(GeminiLiveEvent::TurnComplete);
            }
            if server_content.interrupted == Some(true) {
                is_model_speaking.store(false, Ordering::SeqCst);
                let _ = event_tx.try_send(GeminiLiveEvent::Interrupted);
            }
        }

        if let Some(ref tool_call) = response.tool_call {
            for fc in &tool_call.function_calls {
                let _ = event_tx.try_send(GeminiLiveEvent::FunctionCall {
                    id: fc.id.clone(),
                    name: fc.name.clone(),
                    args: fc.args.clone(),
                });
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// WebSocket Session
// ─────────────────────────────────────────────────────────────────────────────

struct LiveSession {
    ws_tx: mpsc::Sender<Message>,
    ws_rx: mpsc::Receiver<Result<Message, tokio_tungstenite::tungstenite::Error>>,
}

type WsStream = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

impl LiveSession {
    async fn connect(api_key: &str, system_instruction: Option<&str>, tools: Option<&serde_json::Value>) -> Result<Self> {
        if api_key.trim().is_empty() {
            return Err(anyhow!("Missing Gemini API key (GEMINI_API_KEY / GOOGLE_API_KEY / GOOGLE_GENERATIVE_AI_API_KEY, or settings/ai/providers.json)."));
        }
        let url = format!(
            "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent?key={}",
            api_key
        );

        let (ws_stream, _): (WsStream, _) = connect_async(&url).await.map_err(|e| anyhow!("WebSocket connect failed: {e}"))?;
        let (mut write, mut read) = ws_stream.split();

        // Channels for async communication
        let (tx_to_ws, mut rx_to_ws) = mpsc::channel::<Message>(64);
        let (tx_from_ws, rx_from_ws) = mpsc::channel::<Result<Message, tokio_tungstenite::tungstenite::Error>>(64);

        // Spawn writer task
        tokio::spawn(async move {
            while let Some(msg) = rx_to_ws.recv().await {
                if write.send(msg).await.is_err() {
                    break;
                }
            }
        });

        // Spawn reader task
        tokio::spawn(async move {
            while let Some(msg) = read.next().await {
                if tx_from_ws.send(msg).await.is_err() {
                    break;
                }
            }
        });

        // Send setup message
        let model = std::env::var("CUNNING_GEMINI_LIVE_MODEL").unwrap_or_else(|_| GEMINI_LIVE_MODEL.to_string());
        let voice_name = std::env::var("CUNNING_GEMINI_LIVE_VOICE").unwrap_or_else(|_| "Aoede".to_string());
        let generation_config = json!({
            "responseModalities": ["AUDIO"],
            "speechConfig": {
                "voiceConfig": {
                    "prebuiltVoiceConfig": {
                        "voiceName": voice_name
                    }
                }
            }
        });

        let mut setup = json!({
            "model": format!("models/{}", model),
            "generationConfig": generation_config,
        });

        if let Some(instruction) = system_instruction {
            setup["systemInstruction"] = json!({
                "parts": [{"text": instruction}]
            });
        }

        if let Some(tools_def) = tools {
            setup["tools"] = tools_def.clone();
        }

        let setup_msg = json!({ "setup": setup });

        tx_to_ws.send(Message::Text(setup_msg.to_string())).await
            .map_err(|e| anyhow!("Failed to send setup: {e}"))?;

        Ok(Self {
            ws_tx: tx_to_ws,
            ws_rx: rx_from_ws,
        })
    }

    async fn send_audio(&mut self, pcm_16k: &[u8]) -> Result<()> {
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(pcm_16k);
        let msg = json!({
            "realtimeInput": {
                "mediaChunks": [{
                    "data": b64,
                    "mimeType": "audio/pcm"
                }]
            }
        });
        self.ws_tx.send(Message::Text(msg.to_string())).await
            .map_err(|e| anyhow!("Send audio failed: {e}"))
    }

    async fn send_text(&mut self, text: &str) -> Result<()> {
        let msg = json!({
            "clientContent": {
                "turns": [{ "role": "user", "parts": [{"text": text}] }],
                "turnComplete": true
            }
        });
        self.ws_tx.send(Message::Text(msg.to_string())).await
            .map_err(|e| anyhow!("Send text failed: {e}"))
    }

    async fn send_tool_result(&mut self, id: &str, function_name: &str, result: &serde_json::Value) -> Result<()> {
        let msg = json!({
            "toolResponse": {
                "functionResponses": [{
                    "id": id,
                    "name": function_name,
                    "response": result
                }]
            }
        });
        self.ws_tx.send(Message::Text(msg.to_string())).await
            .map_err(|e| anyhow!("Send tool result failed: {e}"))
    }

    async fn try_recv(&mut self) -> Result<Option<LiveResponse>> {
        match self.ws_rx.try_recv() {
            Ok(Ok(Message::Text(text))) => {
                let response: LiveResponse = serde_json::from_str(&text)
                    .map_err(|e| anyhow!("Parse response failed: {e}\nRaw: {text}"))?;
                Ok(Some(response))
            }
            Ok(Ok(Message::Close(frame))) => Err(anyhow!("{}", close_reason(frame.as_ref()))),
            Ok(Ok(Message::Ping(data))) => {
                // Keepalive: respond to ping to avoid server close.
                let _ = self.ws_tx.send(Message::Pong(data)).await;
                Ok(None)
            }
            Ok(Ok(Message::Pong(_))) => Ok(None),
            Ok(Ok(Message::Binary(_))) => Ok(None),
            Ok(Err(e)) => Err(anyhow!("WebSocket error: {e}")),
            Err(mpsc::error::TryRecvError::Empty) => Ok(None),
            Err(mpsc::error::TryRecvError::Disconnected) => Err(anyhow!("WebSocket disconnected")),
            _ => Ok(None),
        }
    }
}

fn close_reason(frame: Option<&CloseFrame<'static>>) -> String {
    match frame {
        Some(f) => format!("WebSocket closed (code={}, reason={})", u16::from(f.code), f.reason),
        None => "WebSocket closed".to_string(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Response Types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct LiveResponse {
    server_content: Option<ServerContent>,
    tool_call: Option<ToolCall>,
    setup_complete: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServerContent {
    model_turn: Option<ModelTurn>,
    turn_complete: Option<bool>,
    interrupted: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ModelTurn {
    parts: Vec<Part>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Part {
    text: Option<String>,
    inline_data: Option<InlineData>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InlineData {
    mime_type: String,
    data: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolCall {
    function_calls: Vec<FunctionCall>,
}

#[derive(Debug, Deserialize)]
struct FunctionCall {
    #[serde(default)]
    id: String,
    name: String,
    args: serde_json::Value,
}
