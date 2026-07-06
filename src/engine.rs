//! The mind inside the diary: sends an image of the vanished handwriting to
//! an OpenAI vision model and gets back a transcription plus an in-character
//! reply. The transcript of the whole session is kept so the diary remembers.

use anyhow::{anyhow, Context, Result};
use base64::Engine as _;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Clone, Deserialize)]
pub struct Reply {
    #[serde(default)]
    pub transcription: String,
    pub reply: String,
}

#[derive(Clone)]
pub struct Engine {
    pub model: String,
    pub base_url: String,
    pub api_key: String,
    pub system_prompt: String,
    history: Vec<(String, String)>, // (writer's words, diary's words)
}

impl Engine {
    pub fn new(model: String, base_url: String, api_key: String, system_prompt: String) -> Self {
        Engine {
            model,
            base_url,
            api_key,
            system_prompt,
            history: Vec::new(),
        }
    }

    pub fn remember(&mut self, wrote: String, replied: String) {
        self.history.push((wrote, replied));
    }

    /// Blocking call; run it on a worker thread while the dots animate.
    pub fn converse(&self, page_png: &[u8]) -> Result<Reply> {
        let image_uri = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(page_png)
        );

        let mut messages = vec![json!({"role": "system", "content": self.system_prompt})];
        for (wrote, replied) in &self.history {
            messages.push(json!({"role": "user", "content": wrote}));
            messages.push(json!({"role": "assistant", "content": replied}));
        }
        messages.push(json!({
            "role": "user",
            "content": [
                {"type": "text",
                 "text": "New words have been written on the page. The image shows exactly what the writer wrote by hand."},
                {"type": "image_url", "image_url": {"url": image_uri}}
            ]
        }));

        let body = json!({
            "model": self.model,
            "messages": messages,
            "max_tokens": 700,
            "response_format": {"type": "json_object"}
        });

        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let resp = ureq::post(&url)
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .set("Content-Type", "application/json")
            .timeout(std::time::Duration::from_secs(20))
            .send_json(body);

        let value: serde_json::Value = match resp {
            Ok(r) => r.into_json().context("decoding API response")?,
            Err(ureq::Error::Status(code, r)) => {
                let text = r.into_string().unwrap_or_default();
                return Err(anyhow!("API error {code}: {text}"));
            }
            Err(e) => return Err(anyhow!("network error: {e}")),
        };

        let message = &value["choices"][0]["message"];
        // The model occasionally answers via the `refusal` field with null
        // content; treat that text as the reply rather than an error.
        let content = message["content"]
            .as_str()
            .or_else(|| message["refusal"].as_str())
            .ok_or_else(|| anyhow!("unexpected API response shape: {value}"))?;

        // The model is instructed to answer with {"transcription", "reply"};
        // if it misbehaves, salvage the reply rather than writing raw JSON
        // onto the page in ink.
        Ok(extract_reply(content))
    }
}

/// Parse the model's answer defensively: well-formed JSON first, then
/// fence-stripped JSON, then a hand salvage of the "reply" string (which
/// also recovers from JSON truncated mid-string by max_tokens), and only
/// as a last resort the whole content with JSON punctuation shaved off.
fn extract_reply(content: &str) -> Reply {
    let trimmed = content.trim();
    let unfenced = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .map(|s| s.trim_end_matches("```").trim())
        .unwrap_or(trimmed);

    if let Ok(r) = serde_json::from_str::<Reply>(unfenced) {
        return r;
    }
    if let Some(reply) = extract_json_string(unfenced, "reply") {
        return Reply {
            transcription: extract_json_string(unfenced, "transcription").unwrap_or_default(),
            reply,
        };
    }
    Reply {
        transcription: String::new(),
        reply: unfenced
            .trim_matches(|c| c == '{' || c == '}' || c == '"' || c == '`')
            .trim()
            .to_string(),
    }
}

/// Pull the string value of `key` out of JSON-ish text, tolerating a
/// missing closing quote (truncation). JSON escapes are unescaped; \n and
/// \t become spaces — the quill writes a single flowing paragraph.
fn extract_json_string(s: &str, key: &str) -> Option<String> {
    let kpos = s.find(&format!("\"{key}\""))?;
    let rest = &s[kpos + key.len() + 2..];
    let rest = rest[rest.find(':')? + 1..].trim_start();
    let rest = rest.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => break,
            '\\' => match chars.next() {
                Some('n') | Some('t') | Some('r') => out.push(' '),
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32)
                    {
                        out.push(ch);
                    }
                }
                Some(other) => out.push(other),
                None => break,
            },
            _ => out.push(c),
        }
    }
    let out = out.trim().to_string();
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_clean_json() {
        let r = extract_reply(r#"{"transcription": "hoi", "reply": "Goedendag."}"#);
        assert_eq!(r.reply, "Goedendag.");
        assert_eq!(r.transcription, "hoi");
    }

    #[test]
    fn parses_fenced_json() {
        let r = extract_reply("```json\n{\"transcription\": \"hi\", \"reply\": \"Hello.\"}\n```");
        assert_eq!(r.reply, "Hello.");
    }

    #[test]
    fn salvages_truncated_json() {
        let r = extract_reply(r#"{"transcription": "a very long page", "reply": "The diary rememb"#);
        assert_eq!(r.reply, "The diary rememb");
        assert_eq!(r.transcription, "a very long page");
    }

    #[test]
    fn keeps_valid_json_reply_verbatim() {
        let r = extract_reply(r#"{"transcription": "x", "reply": "One.\nTwo \"quoted\"."}"#);
        assert_eq!(r.reply, "One.\nTwo \"quoted\".");
    }

    #[test]
    fn salvage_unescapes_and_flattens_newlines() {
        let r = extract_reply(r#"{"transcription": "x", "reply": "One.\nTwo \"quoted\""#);
        assert_eq!(r.reply, "One. Two \"quoted\"");
    }

    #[test]
    fn falls_back_to_plain_text() {
        let r = extract_reply("Just a plain sentence.");
        assert_eq!(r.reply, "Just a plain sentence.");
    }
}
