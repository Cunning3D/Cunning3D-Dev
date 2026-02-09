//! Global console log system

use bevy::prelude::*;
use once_cell::sync::OnceCell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// A single log entry
#[derive(Clone, Debug)]
pub struct LogEntry {
    pub level: LogLevel,
    pub message: String,
    pub timestamp: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warning,
    Error,
    Debug,
}

impl LogLevel {
    pub fn color(&self) -> bevy_egui::egui::Color32 {
        use bevy_egui::egui::Color32;
        match self {
            LogLevel::Info => Color32::from_rgb(200, 200, 200),
            LogLevel::Warning => Color32::from_rgb(255, 200, 0),
            LogLevel::Error => Color32::from_rgb(255, 100, 100),
            LogLevel::Debug => Color32::from_rgb(150, 150, 255),
        }
    }
}

/// Global console log storage
#[derive(Resource, Clone)]
pub struct ConsoleLog {
    entries: Arc<Mutex<Vec<LogEntry>>>,
    rev: Arc<AtomicU64>,
}

static GLOBAL_CONSOLE: OnceCell<ConsoleLog> = OnceCell::new();

pub fn global_console() -> Option<&'static ConsoleLog> {
    GLOBAL_CONSOLE.get()
}

pub fn init_global_console(log: Res<ConsoleLog>) {
    let _ = GLOBAL_CONSOLE.set(log.clone());
}

impl Default for ConsoleLog {
    fn default() -> Self {
        Self {
            entries: Arc::new(Mutex::new(Vec::new())),
            rev: Arc::new(AtomicU64::new(1)),
        }
    }
}

impl ConsoleLog {
    #[inline(always)]
    pub fn revision(&self) -> u64 {
        self.rev.load(Ordering::Relaxed)
    }

    pub fn log(&self, level: LogLevel, message: impl Into<String>) {
        // Fix timestamp on WASM
        #[cfg(not(target_arch = "wasm32"))]
        let timestamp = format!(
            "{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() % 86400) // Seconds since midnight
                .unwrap_or(0)
        );

        #[cfg(target_arch = "wasm32")]
        let timestamp = "WASM_TIME".to_string();

        let entry = LogEntry {
            level,
            message: message.into(),
            timestamp,
        };

        if let Ok(mut entries) = self.entries.lock() {
            entries.push(entry);

            // Keep only last 1000 entries
            let len = entries.len();
            if len > 1000 {
                entries.drain(0..len - 1000);
            }
        }
        self.rev.fetch_add(1, Ordering::Relaxed);
    }

    pub fn info(&self, message: impl Into<String>) {
        self.log(LogLevel::Info, message);
    }

    pub fn warning(&self, message: impl Into<String>) {
        self.log(LogLevel::Warning, message);
    }

    pub fn error(&self, message: impl Into<String>) {
        self.log(LogLevel::Error, message);
    }

    pub fn debug(&self, message: impl Into<String>) {
        self.log(LogLevel::Debug, message);
    }

    pub fn get_entries(&self) -> Vec<LogEntry> {
        self.entries
            .lock()
            .ok()
            .map(|entries| entries.clone())
            .unwrap_or_default()
    }

    pub fn clear(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.clear();
        }
        self.rev.fetch_add(1, Ordering::Relaxed);
    }

    pub fn get_all_text(&self) -> String {
        self.get_entries()
            .iter()
            .map(|entry| format!("[{}] {:?}: {}", entry.timestamp, entry.level, entry.message))
            .collect::<Vec<_>>()
            .join("\n")
    }
}
