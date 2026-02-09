use serde::{Deserialize, Serialize};

// Image attachment for multimodal messages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageAttachment {
    pub mime_type: String, // "image/png", "image/jpeg", etc.
    pub data_b64: String,  // base64 encoded image data
    pub filename: String,  // original filename for display
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MessageState {
    Pending,
    Streaming,
    Done,
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkingSection {
    pub content: String,
    pub collapsed: bool, // UI 状态：是否折叠
    pub done: bool,      // 是否思考完毕
}

impl Default for ThinkingSection {
    fn default() -> Self {
        Self {
            content: String::new(),
            collapsed: false, // 默认展开，让用户看到思考过程
            done: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    User {
        text: String,
        #[serde(default)]
        images: Vec<ImageAttachment>,
    },
    Ai {
        thinking: Option<ThinkingSection>,
        content: String,
        state: MessageState,
    },
}

impl Message {
    pub fn new_user(text: impl Into<String>) -> Self {
        Self::User {
            text: text.into(),
            images: Vec::new(),
        }
    }
    pub fn new_user_with_images(text: impl Into<String>, images: Vec<ImageAttachment>) -> Self {
        Self::User {
            text: text.into(),
            images,
        }
    }

    pub fn new_ai() -> Self {
        Self::Ai {
            thinking: None,
            content: String::new(),
            state: MessageState::Pending,
        }
    }

    pub fn state(&self) -> MessageState {
        match self {
            Message::User { .. } => MessageState::Done,
            Message::Ai { state, .. } => state.clone(),
        }
    }
    pub fn user_text(&self) -> Option<&str> {
        match self {
            Message::User { text, .. } => Some(text),
            _ => None,
        }
    }
    pub fn user_images(&self) -> &[ImageAttachment] {
        match self {
            Message::User { images, .. } => images,
            _ => &[],
        }
    }

    /// Get text content for token estimation
    pub fn text_content(&self) -> String {
        match self {
            Message::User { text, .. } => text.clone(),
            Message::Ai { content, .. } => content.clone(),
        }
    }
}
