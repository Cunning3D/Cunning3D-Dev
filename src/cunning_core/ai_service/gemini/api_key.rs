//! Gemini API key resolution helpers.

/// Returns the first non-empty env key among the supported Gemini key names.
pub fn read_gemini_api_key_env() -> String {
    for k in ["GEMINI_API_KEY", "GOOGLE_API_KEY", "GOOGLE_GENERATIVE_AI_API_KEY", "GENAI_API_KEY"] {
        if let Ok(v) = std::env::var(k) {
            let v = v.trim().to_string();
            if !v.is_empty() {
                return v;
            }
        }
    }
    String::new()
}

