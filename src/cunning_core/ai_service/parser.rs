//! Streaming parser for `<think>` tags (Chain of Thought). Ported from Oxide-Lab.
use crate::tabs_registry::ai_workspace::session::event::SessionEvent;

const THINK_OPEN: &str = "<think>";
const THINK_CLOSE: &str = "</think>";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThinkingState {
    LookingForOpening,               // 寻找开标签，还没看到非空白内容
    ThinkingStartedEatingWhitespace, // 看到开标签，吃空白
    CollectingThinking,              // 在 thinking 块内，收集内容
    ThinkingDoneEatingWhitespace,    // 看到闭标签，吃空白
    CollectingContent,               // thinking 完成，收集正文
}

pub struct StreamParser {
    state: ThinkingState,
    buffer: String,
    started_thinking_emitted: bool, // 是否已发出 StartedThoughtProcess
}

impl StreamParser {
    pub fn new() -> Self {
        Self {
            state: ThinkingState::LookingForOpening,
            buffer: String::new(),
            started_thinking_emitted: false,
        }
    }

    /// 从 thinking 模式开始（当 prompt 已以 `<think>` 结尾时用）
    pub fn new_thinking() -> Self {
        Self {
            state: ThinkingState::CollectingThinking,
            buffer: String::new(),
            started_thinking_emitted: true,
        }
    }

    pub fn parse(&mut self, token: &str) -> Vec<SessionEvent> {
        self.buffer.push_str(token);
        let mut events = Vec::new();
        loop {
            let (evs, cont) = self.eat();
            events.extend(evs);
            if !cont {
                break;
            }
        }
        events
    }

    fn eat(&mut self) -> (Vec<SessionEvent>, bool) {
        let mut events = Vec::new();
        let buf = self.buffer.clone();
        if buf.is_empty() {
            return (events, false);
        }

        match self.state {
            ThinkingState::LookingForOpening => {
                let trimmed = buf.trim_start();
                if trimmed.starts_with(THINK_OPEN) {
                    let after = trimmed
                        .strip_prefix(THINK_OPEN)
                        .unwrap_or("")
                        .trim_start()
                        .to_string();
                    self.buffer.clear();
                    self.buffer.push_str(&after);
                    if !self.started_thinking_emitted {
                        events.push(SessionEvent::StartedThoughtProcess);
                        self.started_thinking_emitted = true;
                    }
                    self.state = if after.is_empty() {
                        ThinkingState::ThinkingStartedEatingWhitespace
                    } else {
                        ThinkingState::CollectingThinking
                    };
                    (events, true)
                } else if THINK_OPEN.starts_with(trimmed) && !trimmed.is_empty() {
                    (events, false) // 部分标签，继续缓冲
                } else if trimmed.is_empty() {
                    (events, false) // 纯空白
                } else {
                    // 非空白出现在 <think> 之前，跳过 thinking
                    self.state = ThinkingState::CollectingContent;
                    let content = std::mem::take(&mut self.buffer);
                    events.push(SessionEvent::Text(content));
                    (events, false)
                }
            }
            ThinkingState::ThinkingStartedEatingWhitespace => {
                let trimmed = buf.trim_start().to_string();
                self.buffer.clear();
                if trimmed.is_empty() {
                    (events, false)
                } else {
                    self.state = ThinkingState::CollectingThinking;
                    self.buffer.push_str(&trimmed);
                    (events, true)
                }
            }
            ThinkingState::CollectingThinking => {
                if buf.contains(THINK_CLOSE) {
                    let parts: Vec<&str> = buf.splitn(2, THINK_CLOSE).collect();
                    let thinking = parts[0].trim_end().to_string();
                    let remaining = parts
                        .get(1)
                        .map(|s| s.trim_start())
                        .unwrap_or("")
                        .to_string();
                    self.buffer.clear();
                    if !thinking.is_empty() {
                        events.push(SessionEvent::Thinking(thinking));
                    }
                    events.push(SessionEvent::EndedThoughtProcess);
                    if remaining.is_empty() {
                        self.state = ThinkingState::ThinkingDoneEatingWhitespace;
                    } else {
                        self.state = ThinkingState::CollectingContent;
                        self.buffer.push_str(&remaining);
                    }
                    (events, true)
                } else if let Some(ol) = overlap(&buf, THINK_CLOSE) {
                    // 部分闭标签在末尾，缓冲歧义部分
                    let before = &buf[..buf.len() - ol];
                    let ws_len = trailing_ws_len(before);
                    let safe = &buf[..before.len() - ws_len];
                    let ambig = &buf[before.len() - ws_len..];
                    self.buffer.clear();
                    self.buffer.push_str(ambig);
                    if !safe.is_empty() {
                        events.push(SessionEvent::Thinking(safe.to_string()));
                    }
                    (events, false)
                } else {
                    // 纯 thinking 内容，但保留尾部空白
                    let ws_len = trailing_ws_len(&buf);
                    let safe = &buf[..buf.len() - ws_len];
                    let ambig = &buf[buf.len() - ws_len..];
                    self.buffer.clear();
                    self.buffer.push_str(ambig);
                    if !safe.is_empty() {
                        events.push(SessionEvent::Thinking(safe.to_string()));
                    }
                    (events, false)
                }
            }
            ThinkingState::ThinkingDoneEatingWhitespace => {
                let trimmed = buf.trim_start().to_string();
                self.buffer.clear();
                if !trimmed.is_empty() {
                    self.state = ThinkingState::CollectingContent;
                    self.buffer.push_str(&trimmed);
                }
                (events, !trimmed.is_empty())
            }
            ThinkingState::CollectingContent => {
                self.buffer.clear();
                if !buf.is_empty() {
                    events.push(SessionEvent::Text(buf));
                }
                (events, false)
            }
        }
    }

    pub fn finish(&mut self) -> Vec<SessionEvent> {
        if self.buffer.is_empty() {
            return vec![];
        }
        let buf = std::mem::take(&mut self.buffer);
        match self.state {
            ThinkingState::CollectingThinking | ThinkingState::ThinkingStartedEatingWhitespace => {
                vec![
                    SessionEvent::Thinking(buf),
                    SessionEvent::EndedThoughtProcess,
                ]
            }
            ThinkingState::LookingForOpening => vec![SessionEvent::Text(buf)],
            _ => vec![SessionEvent::Text(buf)],
        }
    }
}

fn overlap(s: &str, delim: &str) -> Option<usize> {
    let max = std::cmp::min(delim.len(), s.len());
    (1..=max).rev().find(|&i| s.ends_with(&delim[..i]))
}

fn trailing_ws_len(s: &str) -> usize {
    s.chars()
        .rev()
        .take_while(|c| c.is_whitespace())
        .map(|c| c.len_utf8())
        .sum()
}
