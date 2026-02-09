//! Codex-style truncation helpers (token-budget first, byte-based implementation).

use super::context_config as cfg;

pub fn approx_token_count(text: &str) -> usize {
    ((text.chars().count() as f32) / cfg::TOKEN_EST_CHARS_PER_TOKEN).ceil() as usize
}

pub fn truncate_mid_tokens(text: &str, max_tokens: usize) -> String {
    if text.is_empty() || max_tokens == 0 {
        return trunc_marker("tokens", approx_token_count(text), approx_token_count(text));
    }
    let max_chars = (max_tokens as f32 * cfg::TOKEN_EST_CHARS_PER_TOKEN).ceil() as usize;
    truncate_mid_chars(text, max_chars, Some(max_tokens))
}

pub fn truncate_mid_chars(text: &str, max_chars: usize, max_tokens: Option<usize>) -> String {
    let s = text.trim();
    if s.is_empty() {
        return String::new();
    }
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let (l, r) = split_budget(max_chars);
    let left: String = s.chars().take(l).collect();
    let right: String = s
        .chars()
        .rev()
        .take(r)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let removed_chars = s
        .chars()
        .count()
        .saturating_sub(left.chars().count() + right.chars().count());
    let removed_tokens = max_tokens
        .map(|_| {
            approx_token_count(s)
                .saturating_sub(approx_token_count(&left) + approx_token_count(&right))
        })
        .unwrap_or(0);
    let marker = if let Some(_) = max_tokens {
        trunc_marker("tokens", removed_tokens, removed_chars)
    } else {
        trunc_marker("chars", removed_chars, removed_chars)
    };
    format!("{left}\n{marker}\n{right}")
}

fn split_budget(max_chars: usize) -> (usize, usize) {
    if max_chars <= 40 {
        return (max_chars, 0);
    }
    let half = max_chars / 2;
    (half, max_chars - half)
}

fn trunc_marker(unit: &str, removed_tokens: usize, removed_chars: usize) -> String {
    match unit {
        "tokens" => format!("…{removed_tokens} tokens truncated…"),
        _ => format!("…{removed_chars} chars truncated…"),
    }
}
