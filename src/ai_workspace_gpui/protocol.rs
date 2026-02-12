//! Host↔UI protocol: Zed-like action/event driven architecture.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use uuid::Uuid;

/// Host → Bevy bridge (for voice + other runtime integrations).
#[derive(Debug, Clone)]
pub enum HostToBevy {
    VoiceSpeak { text: String },
    VoiceStopSpeaking,
    VoiceSetAssistantEnabled { enabled: bool },
    VoiceStartListening,
    VoiceStopListening,
    // Gemini Live
    StartGeminiLive { api_key: String, system_instruction: Option<String>, tools: Option<Value> },
    StopGeminiLive,
    SendGeminiLiveText { text: String },
    SendGeminiLiveToolResponse { id: String, name: String, response: Value },
}

/// Backend type for LLM provider selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackendType {
    #[default]
    Gemini,
    OpenAiCompat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VoiceModel {
    #[default]
    Off,
    Legacy,
    GeminiLive,
}

// ─────────────────────────────────────────────────────────────────────────────
// UI → Host Actions (from GPUI window to Host actor)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum UiToHost {
    // Session CRUD
    NewSession,
    SelectSession { session_id: Uuid },
    RenameSession { session_id: Uuid, title: String },
    CopySession { session_id: Uuid },
    DeleteSession { session_id: Uuid },

    // Chat
    SendMessage { session_id: Uuid, text: String, mentions: Vec<MentionSnapshot>, images: Vec<ImageSnapshot> },
    SendMessageToActive { text: String, mentions: Vec<MentionSnapshot>, images: Vec<ImageSnapshot> },
    AbortSession { session_id: Uuid },
    LocalAssistantMessageToActive { text: String },
    /// Set text in the chat composer (UI convenience).
    SetComposerText { session_id: Uuid, text: String },

    SpeakText { text: String },

    // Tool control
    CancelTool { session_id: Uuid, request_id: u64 },
    ApproveTool { session_id: Uuid, request_id: u64, remember: bool },
    DenyTool { session_id: Uuid, request_id: u64 },

    // Provider/model selection
    SelectBackend { backend: BackendType },
    SelectProvider { profile_idx: usize },
    SelectModel { profile_idx: usize, model: String },

    // Provider settings (persisted to settings/ai/providers.json)
    UpdateGeminiSettings { api_key: String, model: String, model_image: String },
    UpdateOpenAiCompatProfile { profile_idx: usize, name: String, base_url: String, api_key: String, models: Vec<String>, selected_model: String },
    AddOpenAiCompatProfile,
    DeleteOpenAiCompatProfile { profile_idx: usize },

    // Voice assistant
    SetVoiceAssistantEnabled { enabled: bool },
    SetVoiceModel { model: VoiceModel },
    UpdateVoiceAssistantSettings {
        enabled: bool,
        use_gemini_live: bool,
        wake_phrases: String,
        cmd_input_phrases: String,
        cmd_send_phrases: String,
        cmd_cancel_phrases: String,
        greet_text: String,
        sleep_text: String,
        idle_timeout_secs: i64,
        auto_send_pause_secs: i64,
    },
    RequestVoiceAssistantSettings,

    /// Unified voice start: host decides Gemini Live or Whisper based on settings
    StartVoice,
    StopVoice,

    // Gemini Live (real-time voice) - explicit control
    StartGeminiLive,
    StopGeminiLive,
    SendGeminiLiveText { text: String },
    GeminiLiveToolCall { id: String, name: String, args: Value },

    // Misc
    RequestSnapshot,
    Shutdown,

    // ─────────────────────────────────────────────────────────────────────────
    // IDE: File Tree
    // ─────────────────────────────────────────────────────────────────────────
    IdeSetRoot { path: PathBuf },
    IdeRefreshTree,
    IdeExpandDir { entry_id: EntryId },
    IdeCollapseDir { entry_id: EntryId },
    IdeToggleProjectPanel,

    // ─────────────────────────────────────────────────────────────────────────
    // IDE: Documents
    // ─────────────────────────────────────────────────────────────────────────
    IdeOpenFile { path: PathBuf },
    IdeCloseFile { path: PathBuf },
    IdeCloseAllFiles,
    IdeSaveFile { path: PathBuf },
    IdeSaveAllFiles,
    IdeSetActiveFile { path: PathBuf },
    IdeEditFile { path: PathBuf, version: u64, edits: Vec<TextEdit> },
    /// Open/activate a file and move the editor cursor (0-based).
    IdeGotoLine { path: PathBuf, line: u32, col: u32 },
    IdeRequestCompletion { path: PathBuf, line: u32, col: u32 },
    IdeRequestDefinition { path: PathBuf, line: u32, col: u32 },
    IdeRequestHover { path: PathBuf, line: u32, col: u32 },

    IdeUndo { path: PathBuf },
    IdeRedo { path: PathBuf },

    IdeRevealInExplorer { path: PathBuf },
    IdeRenamePath { from: PathBuf, to: PathBuf },
    IdeDeletePath { path: PathBuf },

    /// UI-only editor context updates for chat mentions.
    IdeCursorChanged { path: PathBuf, line: u32, col: u32 },
    /// Inclusive 0-based line range.
    IdeSelectionChanged { path: PathBuf, start_line: u32, end_line: u32 },
    IdeSelectionCleared { path: PathBuf },

    // ─────────────────────────────────────────────────────────────────────────
    // IDE: Search / Quick Open / Command Palette
    // ─────────────────────────────────────────────────────────────────────────
    IdeQuickOpenQuery { query: String },
    IdeGlobalSearch { query: String, case_sensitive: bool, whole_word: bool, regex: bool },
    IdeCommandPalette { query: String },
}

// ─────────────────────────────────────────────────────────────────────────────
// Host → UI Messages (from Host actor to GPUI window)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum HostToUi {
    /// Full state snapshot (sent on connect / resync)
    Snapshot(WorkspaceSnapshot),
    /// Incremental event for a session
    SessionEvent { session_id: Uuid, event: SessionEventSnapshot },
    /// Active session changed
    ActiveSessionChanged { session_id: Option<Uuid> },
    /// Provider list updated
    ProvidersUpdated { profiles: Vec<ProviderSnapshot>, active_idx: usize },
    /// Voice assistant settings updated
    VoiceAssistantSettingsUpdated(VoiceAssistantSettingsSnapshot),
    /// Gemini Live state changed
    GeminiLiveStateChanged { active: bool },
    /// Gemini Live transcription (what user said)
    GeminiLiveTranscript { text: String },
    /// Gemini Live model response text
    GeminiLiveResponse { text: String },
    /// Voice listening state changed (for Whisper mode)
    VoiceListeningChanged { listening: bool },
    /// UI convenience: set the chat composer text for a session.
    ComposerTextSet { session_id: Uuid, text: String },
    /// Shutdown acknowledged
    Shutdown,

    // ─────────────────────────────────────────────────────────────────────────
    // IDE Events (incremental, avoid full snapshot for performance)
    // ─────────────────────────────────────────────────────────────────────────
    IdeEvent(IdeEvent),
}

// ─────────────────────────────────────────────────────────────────────────────
// Snapshot Types (lightweight projections for UI)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct WorkspaceSnapshot {
    pub sessions: Vec<SessionSnapshot>,
    pub active_session_id: Option<Uuid>,
    pub providers: Vec<ProviderSnapshot>,
    pub active_provider_idx: usize,
    pub tool_allow_always: Vec<String>,
    pub voice_assistant_enabled: bool,
    pub voice_model: VoiceModel,
    pub gemini_live_active: bool,
    pub ide: IdeSnapshot,
}

#[derive(Debug, Clone)]
pub struct SessionSnapshot {
    pub id: Uuid,
    pub title: String,
    pub is_busy: bool,
    pub busy_reason: Option<String>,
    pub busy_stage: BusyStageSnapshot,
    pub entries: Vec<EntrySnapshot>,
    pub token_usage: TokenUsageSnapshot,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BusyStageSnapshot {
    #[default]
    Idle,
    ToolRunning,
    ToolFeedback,
    WaitingModel,
    Generating,
    AutoHeal { current: u8, max: u8 },
    NetworkRetry { attempt: u32 },
}

#[derive(Debug, Clone, Default)]
pub struct TokenUsageSnapshot {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub max_tokens: u64,
}

impl TokenUsageSnapshot {
    pub fn ratio(&self) -> f32 {
        if self.max_tokens == 0 { 0.0 } else { self.total_tokens as f32 / self.max_tokens as f32 }
    }
    pub fn is_warning(&self) -> bool { self.ratio() >= 0.75 }
}

#[derive(Debug, Clone)]
pub enum EntrySnapshot {
    User { text: String, images: Vec<ImageSnapshot>, mentions: Vec<MentionSnapshot>, timestamp: Option<i64> },
    Assistant { thinking: Option<ThinkingSnapshot>, content: String, state: MessageStateSnapshot, timestamp: Option<i64> },
    ToolCall(ToolCallSnapshot),
}

#[derive(Debug, Clone)]
pub struct ThinkingSnapshot {
    pub content: String,
    pub collapsed: bool,
    pub done: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageStateSnapshot {
    Pending,
    Streaming,
    Done,
    Error,
}

#[derive(Debug, Clone)]
pub struct ToolCallSnapshot {
    pub id: u64,
    pub tool_name: String,
    pub kind: ToolKindSnapshot,
    pub status: ToolCallStatusSnapshot,
    pub title: String,
    pub args_preview: String,
    pub raw_input: Option<String>,
    pub llm_result: Option<String>,
    pub raw_output: Option<String>,
    pub diffs: Vec<FileDiffSnapshot>,
    pub logs: Vec<ToolLogSnapshot>,
}

#[derive(Debug, Clone)]
pub struct FileDiffSnapshot {
    pub file_path: String,
    pub hunks: Vec<DiffHunkSnapshot>,
}

#[derive(Debug, Clone)]
pub struct DiffHunkSnapshot {
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub lines: Vec<DiffLineSnapshot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKindSnapshot { Context, Added, Removed }

#[derive(Debug, Clone)]
pub struct DiffLineSnapshot {
    pub kind: DiffLineKindSnapshot,
    pub line_num_old: Option<usize>,
    pub line_num_new: Option<usize>,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKindSnapshot {
    Read,
    Search,
    Execute,
    Edit,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCallStatusSnapshot {
    Pending,
    AwaitingApproval,
    InProgress,
    Completed,
    Rejected(String),
    Failed(String),
    Canceled,
}

impl ToolCallStatusSnapshot {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Rejected(_) | Self::Failed(_) | Self::Canceled)
    }
}

#[derive(Debug, Clone)]
pub struct ToolLogSnapshot {
    pub message: String,
    pub level: ToolLogLevelSnapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolLogLevelSnapshot {
    Info,
    Warn,
    Error,
    Progress,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageSnapshot {
    pub mime_type: String,
    pub data_b64: String,
    pub filename: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MentionSnapshot {
    File { path: String },
    Directory { path: String },
    Selection { path: Option<String>, start_line: u32, end_line: u32 },
    Symbol { path: String, name: String, start_line: u32, end_line: u32 },
    Diagnostics { errors: bool, warnings: bool },
    PastedImage { id: u64 },
    Fetch { url: String },
}

#[derive(Debug, Clone)]
pub struct ProviderSnapshot {
    pub name: String,
    pub base_url: String,
    pub models: Vec<String>,
    pub selected_model: String,
    pub has_api_key: bool,
    pub backend_type: BackendType,
    pub image_model: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct VoiceAssistantSettingsSnapshot {
    pub enabled: bool,
    pub use_gemini_live: bool,
    pub wake_phrases: String,
    pub cmd_input_phrases: String,
    pub cmd_send_phrases: String,
    pub cmd_cancel_phrases: String,
    pub greet_text: String,
    pub sleep_text: String,
    pub idle_timeout_secs: i64,
    pub auto_send_pause_secs: i64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Session Event Snapshot (incremental updates)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum SessionEventSnapshot {
    // Text streaming
    StartedThoughtProcess,
    EndedThoughtProcess,
    Thinking(String),
    Text(String),
    StreamedCompletion,

    // Tool lifecycle
    ToolCallRequest { tool_name: String, request_id: u64, args_preview: String },
    ToolAwaitingApproval { request_id: u64 },
    ToolExecutionStarted { request_id: u64 },
    ToolExecutionProgress { request_id: u64, log: ToolLogSnapshot },
    ToolExecutionSuccess { request_id: u64, llm_result: String, raw_output: Option<String> },
    ToolExecutionError { request_id: u64, error: String },
    ToolExecutionCancelled { request_id: u64 },
    ToolRejected { request_id: u64, reason: String },

    // Session state
    BusyChanged { is_busy: bool, reason: Option<String>, stage: BusyStageSnapshot },
    TitleUpdated(String),
    TokenUsageUpdated(TokenUsageSnapshot),
    NetworkRetry { attempt: u32, max_seconds: u64 },
    Error(String),

    // Entry mutations (for UI to update its local copy)
    EntryAdded { index: usize, entry: EntrySnapshot },
    EntryUpdated { index: usize, entry: EntrySnapshot },
}

// ─────────────────────────────────────────────────────────────────────────────
// IDE: Types and Snapshots (Zed-isomorphic)
// ─────────────────────────────────────────────────────────────────────────────

/// Unique identifier for a file tree entry
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct EntryId(pub u64);

impl EntryId {
    pub fn new(id: u64) -> Self { Self(id) }
}

/// Text edit operation for document editing
#[derive(Debug, Clone)]
pub struct TextEdit {
    pub start_offset: usize,
    pub end_offset: usize,
    pub new_text: String,
}

/// File tree entry kind (Zed-isomorphic: lazy loading states)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    UnloadedDir,
    PendingDir,
    Dir,
    File,
}

/// Single file tree entry
#[derive(Debug, Clone)]
pub struct FileEntrySnapshot {
    pub id: EntryId,
    pub path: PathBuf,
    pub name: String,
    pub kind: EntryKind,
    pub depth: usize,
    pub is_expanded: bool,
    pub is_selected: bool,
    pub size: Option<u64>,
    pub mtime: Option<u64>,
}

/// IDE state snapshot (lightweight, no file content)
#[derive(Debug, Clone, Default)]
pub struct IdeSnapshot {
    pub root_path: Option<PathBuf>,
    pub project_panel_visible: bool,
    pub visible_entries: Vec<FileEntrySnapshot>,
    pub open_files: Vec<OpenFileSnapshot>,
    pub active_file: Option<PathBuf>,
    pub recent_files: Vec<PathBuf>,
}

/// Open file metadata (no content - content sent via IdeEvent::FileOpened)
#[derive(Debug, Clone)]
pub struct OpenFileSnapshot {
    pub path: PathBuf,
    pub is_dirty: bool,
    pub version: u64,
    pub cursor_line: usize,
    pub cursor_col: usize,
}

/// IDE incremental events (for performance: avoid full snapshot)
#[derive(Debug, Clone)]
pub enum IdeEvent {
    // File tree
    TreeRootChanged { path: PathBuf },
    TreeEntriesUpdated { entries: Vec<FileEntrySnapshot>, parent_id: Option<EntryId> },
    TreeEntryExpanded { entry_id: EntryId, children: Vec<FileEntrySnapshot> },
    TreeEntryCollapsed { entry_id: EntryId },
    TreeEntrySelected { entry_id: Option<EntryId> },
    TreeLoading { entry_id: EntryId },

    // Documents
    FileOpened { path: PathBuf, content: String, version: u64 },
    FileClosed { path: PathBuf },
    FileChanged { path: PathBuf, version: u64, edits: Vec<TextEdit> },
    FileSaved { path: PathBuf, version: u64 },
    FileDirtyChanged { path: PathBuf, is_dirty: bool },
    ActiveFileChanged { path: Option<PathBuf> },
    CursorMoved { path: PathBuf, line: u32, col: u32 },
    DiagnosticsUpdated { path: PathBuf, diagnostics: Vec<DiagnosticSnapshot> },
    CompletionItems { path: PathBuf, items: Vec<String> },
    HoverText { path: PathBuf, markdown: String },
    DefinitionLocation { from: PathBuf, to: PathBuf, line: u32, col: u32 },
    FileError { path: PathBuf, error: String },

    // Search / Quick Open
    QuickOpenResults { items: Vec<QuickOpenItem> },
    GlobalSearchResults { matches: Vec<SearchMatch> },
    CommandPaletteResults { commands: Vec<CommandItem> },

    // Errors
    Error { message: String },
}

#[derive(Debug, Clone)]
pub struct DiagnosticSnapshot {
    pub message: String,
    pub severity: u8, // 1=Error,2=Warning,3=Info,4=Hint
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

/// Quick Open result item
#[derive(Debug, Clone)]
pub struct QuickOpenItem {
    pub path: PathBuf,
    pub name: String,
    pub icon: FileIcon,
    pub score: f64,
}

/// File icon type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FileIcon {
    #[default]
    File,
    Folder,
    FolderOpen,
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Json,
    Toml,
    Markdown,
    Image,
    Binary,
}

impl FileIcon {
    pub fn from_path(path: &std::path::Path) -> Self {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        match ext.to_lowercase().as_str() {
            "rs" => Self::Rust,
            "py" => Self::Python,
            "js" | "jsx" | "mjs" => Self::JavaScript,
            "ts" | "tsx" => Self::TypeScript,
            "json" => Self::Json,
            "toml" => Self::Toml,
            "md" | "markdown" => Self::Markdown,
            "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" => Self::Image,
            "exe" | "dll" | "so" | "dylib" | "wasm" => Self::Binary,
            _ => Self::File,
        }
    }
}

/// Global search match
#[derive(Debug, Clone)]
pub struct SearchMatch {
    pub path: PathBuf,
    pub line: usize,
    pub col: usize,
    pub text: String,
    pub match_start: usize,
    pub match_end: usize,
}

/// Command palette item
#[derive(Debug, Clone)]
pub struct CommandItem {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub keybinding: Option<String>,
}
