use serde::Deserialize;

use crate::config::ChatConfig;
use crate::{Error, Result};

pub struct ChatClient {
    client: reqwest::Client,
    config: ChatConfig,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Deserialize)]
struct ChatMessage {
    content: String,
}

impl ChatClient {
    pub fn new(config: ChatConfig) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(180))
                .build()
                .expect("client build"),
            config,
        }
    }

    pub async fn complete(&self, prompt: &str, max_tokens: u32) -> Result<String> {
        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        let mut body = serde_json::json!({
            "model": self.config.model,
            "max_tokens": max_tokens,
            "messages": [{ "role": "user", "content": prompt }],
        });
        if !self.config.thinking {
            // llama.cpp switch for Qwen-style thinking models; a budget spent
            // on reasoning_content leaves content empty. Others ignore it.
            body["chat_template_kwargs"] = serde_json::json!({ "enable_thinking": false });
        }
        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.config.api_key)
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        let parsed: ChatResponse = response.json().await?;
        parsed
            .choices
            .into_iter()
            .next()
            .map(|choice| choice.message.content)
            .ok_or_else(|| Error::Chat("empty chat response".to_string()))
    }
}
