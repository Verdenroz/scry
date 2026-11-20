//! Claim verification: P(yes) to a question about a claim and its evidence,
//! from a configurable model.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::config::{VerifyApi, VerifyConfig};
use crate::{Error, Result};

/// What the verifier is asked about a claim and its evidence.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Question {
    /// Does the evidence bear out the claim's main finding. Gates capture.
    #[default]
    Grounded,
    /// Does the current code still contain what a note says about it.
    Holds,
    /// Does the evidence show the claim is no longer true.
    Contradiction,
}

impl Question {
    fn intro(self) -> &'static str {
        match self {
            Question::Grounded => "You check whether evidence grounds a claim.",
            Question::Holds => {
                "You check whether a note about a codebase still matches the current code."
            }
            Question::Contradiction => "You check whether evidence contradicts a claim.",
        }
    }

    fn labels(self) -> (&'static str, &'static str) {
        match self {
            Question::Holds => ("Current code", "Note"),
            _ => ("Evidence", "Claim"),
        }
    }

    fn instructions(self) -> &'static str {
        match self {
            Question::Grounded => {
                "Does the evidence directly show the main finding of the claim? Answer yes if the evidence is an observation of the thing the claim is about and bears out its main point, even if the claim adds detail or explanation the evidence does not show. Answer no if the evidence is about something else, shows nothing that bears on the main point, or conflicts with the claim."
            }
            Question::Holds => {
                "A note can mix statements about the code with history, measurements and advice that code cannot show; judge only what the note says about the code. Find each function, constant, setting, hook or behaviour the note says the code has. Answer yes only if every one of them is visible in the code shown and nothing there conflicts with the note. Answer no if any of them is missing from the code shown, even when related code is still there, or if the code conflicts with the note."
            }
            Question::Contradiction => {
                "Does the evidence show that the claim is no longer true? Answer yes only if the evidence directly conflicts with the claim. Answer no if the claim is merely not shown, not mentioned, or cannot be judged from the evidence."
            }
        }
    }

    fn prompt(self) -> String {
        let (evidence, claim) = self.labels();
        format!(
            "{}\n\n{evidence}:\n<<<\n{{evidence}}\n>>>\n\n{claim}: {{claim}}\n\n{} Answer with exactly one word: yes or no.",
            self.intro(),
            self.instructions()
        )
    }
}

#[async_trait]
pub trait ClaimScorer: Send + Sync {
    async fn ask(&self, question: Question, claim: &str, evidence: &str) -> Result<f64>;
}

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
    content: Option<String>,
}

#[derive(Deserialize)]
struct ChatLogprobs {
    content: Option<Vec<TokenLogprobs>>,
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

    pub fn threshold(&self, question: Question) -> f64 {
        match question {
            Question::Grounded => self.config.threshold,
            Question::Holds => self.config.holds_threshold,
            Question::Contradiction => self.config.contradiction_threshold,
        }
    }

    pub async fn ask(&self, question: Question, claim: &str, evidence: &str) -> Result<f64> {
        match self.config.api {
            VerifyApi::Chat => {
                let prompt = match (&self.config.prompt, question) {
                    (Some(prompt), Question::Grounded) => prompt.clone(),
                    _ => question.prompt(),
                };
                self.score_chat(&prompt, claim, evidence).await
            }
            VerifyApi::SystemOne => {
                self.score_systemone(question.instructions(), claim, evidence)
                    .await
            }
        }
    }

    async fn score_chat(&self, prompt: &str, claim: &str, evidence: &str) -> Result<f64> {
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
            "messages": [{ "role": "user", "content": render_prompt(prompt, claim, evidence) }],
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

    async fn score_systemone(&self, question: &str, claim: &str, evidence: &str) -> Result<f64> {
        let url = format!("{}/systemone", self.config.base_url.trim_end_matches('/'));
        let body = serde_json::json!({
            "model": self.config.model,
            "state": { "claim": claim, "evidence": evidence },
            "questions": { "q": { "type": "noul", "instructions": question } },
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

#[async_trait]
impl ClaimScorer for Verifier {
    async fn ask(&self, question: Question, claim: &str, evidence: &str) -> Result<f64> {
        Verifier::ask(self, question, claim, evidence).await
    }
}

fn render_prompt(template: &str, claim: &str, evidence: &str) -> String {
    let mut rest = template;
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
        .and_then(|l| l.content?.into_iter().next())
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
    match choice
        .message
        .content
        .unwrap_or_default()
        .trim()
        .to_lowercase()
        .as_str()
    {
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
        let nulls = choice(r#"{"message":{"content":null},"logprobs":{"content":null}}"#);
        assert!(chat_score(nulls).is_err());
    }

    #[test]
    fn each_question_renders_its_own_prompt() {
        let grounded = render_prompt(&Question::Grounded.prompt(), "C1", "E1");
        assert!(grounded.contains("Evidence:\n<<<\nE1\n>>>\n\nClaim: C1\n\n"));
        assert!(grounded.ends_with("Answer with exactly one word: yes or no."));
        let holds = render_prompt(&Question::Holds.prompt(), "C1", "E1");
        assert!(holds.contains("Current code:\n<<<\nE1\n>>>\n\nNote: C1\n\n"));
        assert_ne!(
            Question::Contradiction.instructions(),
            Question::Grounded.instructions()
        );
    }

    #[test]
    fn prompt_override_fills_both_placeholders() {
        let custom = "{evidence} => {claim}?";
        assert_eq!(render_prompt(custom, "C1", "E1"), "E1 => C1?");
        assert_eq!(
            render_prompt(custom, "{evidence}", "{claim}"),
            "{claim} => {evidence}?"
        );
    }
}
