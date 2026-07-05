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
            "max_tokens": 400,
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
        // if it misbehaves, treat the whole message as the reply.
        Ok(serde_json::from_str::<Reply>(content).unwrap_or(Reply {
            transcription: String::new(),
            reply: content.trim().to_string(),
        }))
    }
}
