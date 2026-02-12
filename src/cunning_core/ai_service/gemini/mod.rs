pub mod client;
pub mod copilot_host;
pub mod api_key;
pub mod parser;

pub use client::GeminiClient;

/// Normalizes Gemini model names across the app.
pub fn normalize_model_name(model: &str) -> String {
    let m = model.trim();
    if m.is_empty() { return String::new(); }
    let m = m.strip_prefix("models/").unwrap_or(m);
    match m {
        "gemini-3-pro" => "gemini-3-pro-preview".to_string(),
        _ => m.to_string(),
    }
}
