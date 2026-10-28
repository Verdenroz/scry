//! Claim verification: P(evidence supports claim) from a configurable model.

use serde::Deserialize;

use crate::config::{VerifyApi, VerifyConfig};
use crate::{Error, Result};

const DEFAULT_PROMPT: &str = "You check whether evidence supports a claim.

Evidence:
<<<
{evidence}
>>>

Claim: {claim}

Does the evidence, on its own, fully support every part of the claim? Answer no if the claim is only partly supported, needs facts not shown in the evidence, or is contradicted. Answer with exactly one word: yes or no.";

const SYSTEMONE_QUESTION: &str = "Does the evidence, on its own, fully support every part of the claim? Answer no if the claim is only partly supported, needs facts not shown in the evidence, or is contradicted.";

pub struct Verifier {
    client: reqwest::Client,
    config: VerifyConfig,
}

#[derive(Deserialize)]
struct ChatReply {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    #[serde(default)]
    message: ChatMessage,
    logprobs: Option<ChatLogprobs>,
}

#[derive(Deserialize, Default)]
struct ChatMessage {
    #[serde(default)]
    content: String,
}

#[derive(Deserialize)]
struct ChatLogprobs {
    content: Vec<TokenLogprobs>,
}

#[derive(Deserialize)]
struct TokenLogprobs {
    top_logprobs: Vec<TopLogprob>,
}

#[derive(Deserialize)]
struct TopLogprob {
    token: String,
    logprob: f64,
}

#[derive(Deserialize)]
struct SystemOneReply {
    answers: SystemOneAnswers,
}

#[derive(Deserialize)]
struct SystemOneAnswers {
    q: SystemOneAnswer,
}

#[derive(Deserialize)]
struct SystemOneAnswer {
    noul: f64,
}

impl Verifier {
    pub fn new(config: VerifyConfig) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(180))
                .build()
                .expect("client build"),
            config,
        }
    }

    pub fn threshold(&self) -> f64 {
        self.config.threshold
    }

    pub async fn score(&self, claim: &str, evidence: &str) -> Result<f64> {
        match self.config.api {
            VerifyApi::Chat => self.score_chat(claim, evidence).await,
            VerifyApi::SystemOne => self.score_systemone(claim, evidence).await,
        }
    }

    async fn score_chat(&self, claim: &str, evidence: &str) -> Result<f64> {
        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        // One token is the whole answer, so a thinking model would spend it on reasoning.
        let body = serde_json::json!({
            "model": self.config.model,
            "max_tokens": 1,
            "temperature": 0,
            "logprobs": true,
            "top_logprobs": 10,
            "messages": [{ "role": "user", "content": render_prompt(&self.config, claim, evidence) }],
            "chat_template_kwargs": { "enable_thinking": false },
        });
        let reply: ChatReply = self.post(&url, &body).await?;
        let choice = reply
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| Error::Verify("empty chat response".to_string()))?;
        chat_score(choice)
    }

    async fn score_systemone(&self, claim: &str, evidence: &str) -> Result<f64> {
        let url = format!(
            "{}/v1/systemone",
            self.config.base_url.trim_end_matches('/')
        );
        let body = serde_json::json!({
            "model": self.config.model,
            "state": { "claim": claim, "evidence": evidence },
            "questions": { "q": { "type": "noul", "instructions": SYSTEMONE_QUESTION } },
        });
        let reply: SystemOneReply = self.post(&url, &body).await?;
        Ok(reply.answers.q.noul.clamp(0.0, 1.0))
    }

    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> Result<T> {
        let response = self
            .client
            .post(url)
            .bearer_auth(&self.config.api_key)
            .json(body)
            .send()
            .await?
            .error_for_status()?;
        Ok(response.json().await?)
    }
}

fn render_prompt(config: &VerifyConfig, claim: &str, evidence: &str) -> String {
    let mut rest = config.prompt.as_deref().unwrap_or(DEFAULT_PROMPT);
    let mut out = String::new();
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if let Some(after) = tail.strip_prefix("{claim}") {
            out.push_str(claim);
            rest = after;
        } else if let Some(after) = tail.strip_prefix("{evidence}") {
            out.push_str(evidence);
            rest = after;
        } else {
            out.push('{');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
}

/// Yes/no probability mass from the first token's logprobs; the answer
/// text decides when the server returned none.
fn chat_score(choice: ChatChoice) -> Result<f64> {
    let first = choice
        .logprobs
        .and_then(|l| l.content.into_iter().next())
        .map(|t| t.top_logprobs);
    if let Some(top) = first {
        let mass = |word: &str| -> f64 {
            top.iter()
                .filter(|t| t.token.trim().eq_ignore_ascii_case(word))
                .map(|t| t.logprob.exp())
                .sum()
        };
        let (yes, no) = (mass("yes"), mass("no"));
        if yes + no > 0.0 {
            return Ok(yes / (yes + no));
        }
    }
    match choice.message.content.trim().to_lowercase().as_str() {
        "yes" => Ok(1.0),
        "no" => Ok(0.0),
        _ => Err(Error::Verify("model gave no yes/no answer".to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(json: &str) -> ChatChoice {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn logprob_mass_is_normalised_over_yes_and_no() {
        let c = choice(
            r#"{"message":{"content":"yes"},"logprobs":{"content":[{"top_logprobs":[
                {"token":"yes","logprob":-0.2231},{"token":" Yes","logprob":-3.0},
                {"token":"no","logprob":-1.6094},{"token":"maybe","logprob":-0.1}]}]}}"#,
        );
        let expected = (-0.2231f64).exp() + (-3.0f64).exp();
        let expected = expected / (expected + (-1.6094f64).exp());
        assert!((chat_score(c).unwrap() - expected).abs() < 1e-9);
    }

    #[test]
    fn text_answer_is_the_fallback_without_usable_logprobs() {
        let answer =
            |text: &str| chat_score(choice(&format!(r#"{{"message":{{"content":"{text}"}}}}"#)));
        assert_eq!(answer(" Yes\\n").unwrap(), 1.0);
        assert_eq!(answer("NO").unwrap(), 0.0);
        assert!(answer("The evidence shows").is_err());
        let no_mass = choice(
            r#"{"message":{"content":"yes"},"logprobs":{"content":[{"top_logprobs":[
                {"token":"maybe","logprob":-0.1}]}]}}"#,
        );
        assert_eq!(chat_score(no_mass).unwrap(), 1.0);
    }

    #[test]
    fn prompt_override_replaces_the_default() {
        let mut config: VerifyConfig = toml::from_str("base_url = \"u\"\nmodel = \"m\"\n").unwrap();
        let default = render_prompt(&config, "C1", "E1");
        assert!(default.contains("Claim: C1") && default.contains("E1"));
        config.prompt = Some("{evidence} => {claim}?".to_string());
        assert_eq!(render_prompt(&config, "C1", "E1"), "E1 => C1?");
        assert_eq!(
            render_prompt(&config, "{evidence}", "{claim}"),
            "{claim} => {evidence}?"
        );
    }
}
