//! The OpenAI-compatible chat-completions call (SPEC §10.1).

use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::{Value, json};

use crate::config::AiConfig;

#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub role: &'static str,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Message {
            role: "system",
            content: content.into(),
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Message {
            role: "user",
            content: content.into(),
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Message {
            role: "assistant",
            content: content.into(),
        }
    }
}

/// Tokens as the endpoint counted them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
}

impl std::ops::AddAssign for Usage {
    fn add_assign(&mut self, other: Usage) {
        self.input += other.input;
        self.output += other.output;
    }
}

#[derive(Debug, Clone)]
pub struct Reply {
    pub content: String,
    pub usage: Usage,
    pub seconds: f64,
}

/// Long answers with reasoning take a while; never wait forever.
const TIMEOUT: Duration = Duration::from_secs(240);

/***
 * Send one chat-completions request and return the answer's text.
 *
 * Asks for JSON output when `json` is set; an endpoint that rejects
 * `response_format` gets the same request without it. Errors carry the
 * endpoint's status and message, never the key.
 *
 * @id      docair-ai.client.chat
 * @flow    ask 4 "the question and the digest go to the model"
 * @calls   ai-endpoint (POST /chat/completions: model, messages) -> answer text
 ***/
pub fn chat(cfg: &AiConfig, messages: &[Message], json: bool) -> Result<Reply> {
    chat_with(cfg, messages, json, true)
}

/***
 * As `chat`, and with `think` false asks the model to reason little first (`reasoning_effort: low`; Together's GLM-5.3-Flash ignores `reasoning: {enabled: false}`): for answers a reasoning model would otherwise deliberate over until `max_tokens` runs out (the review, docair-8l6.36). An endpoint that rejects `reasoning_effort` gets the same request without it.
 *
 * @id docair-ai.client.chat_with
 ***/
pub fn chat_with(cfg: &AiConfig, messages: &[Message], json: bool, think: bool) -> Result<Reply> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .build()
        .into();
    let url = format!("{}/chat/completions", cfg.endpoint);
    let mut body = json!({
        "model": cfg.model,
        "messages": messages,
        "max_tokens": cfg.max_tokens,
        "temperature": 0.2,
    });
    if json {
        body["response_format"] = json!({ "type": "json_object" });
    }
    if !think {
        body["reasoning_effort"] = json!("low");
    }
    let started = Instant::now();
    let (mut status, mut text) = post(&agent, &url, cfg, &body)?;
    // What an endpoint does not take is left out, and the same request sent again.
    for field in ["reasoning_effort", "response_format"] {
        if status == 400 && text.contains(field) && body.get(field).is_some() {
            body.as_object_mut().unwrap().remove(field);
            (status, text) = post(&agent, &url, cfg, &body)?;
        }
    }
    if !(200..300).contains(&status) {
        bail!(
            "the AI endpoint {} answered {status}: {}",
            cfg.public_endpoint(),
            error_message(&text)
        );
    }
    answer_of(&text, cfg, started)
}

/// How long a streamed answer may take to start (a long input is read first) and to finish.
const STREAM_START: Duration = Duration::from_secs(15 * 60);
const STREAM_END: Duration = Duration::from_secs(60 * 60);

/***
 * One chat-completions request whose answer streams in (docair-8l6.79): `on` gets the answer's text so far as it grows, so a caller can use what is complete before the rest arrives. `reasoning` is the model's `reasoning_effort` (low, medium, high); none leaves it to the model. An endpoint that rejects `stream`, `reasoning_effort` or `response_format` gets the request without it; one that answers without streaming is read whole, and `on` gets it once.
 *
 * @id docair-ai.client.chat_streamed
 ***/
pub fn chat_streamed(
    cfg: &AiConfig,
    messages: &[Message],
    json: bool,
    reasoning: Option<&str>,
    on: &dyn Fn(&str),
) -> Result<Reply> {
    use std::io::BufRead;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_recv_response(Some(STREAM_START))
        .timeout_recv_body(Some(STREAM_END))
        .http_status_as_error(false)
        .build()
        .into();
    let url = format!("{}/chat/completions", cfg.endpoint);
    let mut body = json!({
        "model": cfg.model,
        "messages": messages,
        "max_tokens": cfg.max_tokens,
        "temperature": 0.2,
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    if json {
        body["response_format"] = json!({ "type": "json_object" });
    }
    if let Some(r) = reasoning {
        body["reasoning_effort"] = json!(r);
    }
    let started = Instant::now();
    let send = |body: &Value| {
        let mut req = agent.post(&url).header("Accept", "text/event-stream");
        if let Some(key) = &cfg.api_key {
            req = req.header("Authorization", &format!("Bearer {key}"));
        }
        req.send_json(body).map_err(|e| {
            anyhow::anyhow!(
                "cannot reach the AI endpoint {}: {e}",
                cfg.public_endpoint()
            )
        })
    };
    let read_all = |resp: &mut ureq::http::Response<ureq::Body>| {
        resp.body_mut()
            .with_config()
            .limit(20 * 1024 * 1024)
            .read_to_string()
            .context("reading the AI endpoint's answer")
    };
    let mut resp = send(&body)?;
    // What an endpoint does not take is left out, and the same request sent again.
    for field in ["stream", "reasoning_effort", "response_format"] {
        if resp.status().as_u16() == 400 && body.get(field).is_some() {
            let text = read_all(&mut resp)?;
            if !text.contains(field) {
                bail!(
                    "the AI endpoint {} answered 400: {}",
                    cfg.public_endpoint(),
                    error_message(&text)
                );
            }
            let o = body.as_object_mut().unwrap();
            o.remove(field);
            if field == "stream" {
                o.remove("stream_options");
            }
            resp = send(&body)?;
        }
    }
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        let text = read_all(&mut resp)?;
        bail!(
            "the AI endpoint {} answered {status}: {}",
            cfg.public_endpoint(),
            error_message(&text)
        );
    }
    let streamed = resp
        .body()
        .mime_type()
        .is_some_and(|m| m.contains("event-stream"));
    if !streamed {
        let text = read_all(&mut resp)?;
        let reply = answer_of(&text, cfg, started)?;
        on(&reply.content);
        return Ok(reply);
    }
    // Server-sent events: `data: {chunk}` lines, each with a piece of the answer, then `data: [DONE]`.
    let mut content = String::new();
    let mut usage = Usage::default();
    let mut finish = String::new();
    let reader = std::io::BufReader::new(resp.body_mut().with_config().limit(u64::MAX).reader());
    for line in reader.lines() {
        let line = line.context("reading the AI endpoint's answer")?;
        let Some(data) = line.strip_prefix("data:").map(str::trim) else {
            continue;
        };
        if data == "[DONE]" {
            break;
        }
        let Ok(v) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        if let Some(t) = v["choices"][0]["delta"]["content"].as_str()
            && !t.is_empty()
        {
            content.push_str(t);
            on(&content);
        }
        if let Some(f) = v["choices"][0]["finish_reason"].as_str() {
            finish = f.to_string();
        }
        if v["usage"].is_object() {
            usage = Usage {
                input: v["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
                output: v["usage"]["completion_tokens"].as_u64().unwrap_or(0),
            };
        }
    }
    if content.trim().is_empty() {
        if finish == "length" {
            bail!(
                "the model used up max_tokens ({}) before it answered; raise `max_tokens` in [ai]",
                cfg.max_tokens
            );
        }
        bail!("the model sent an empty answer");
    }
    Ok(Reply {
        content,
        usage,
        seconds: started.elapsed().as_secs_f64(),
    })
}

/// A whole (not streamed) chat-completions answer: its text and usage.
fn answer_of(text: &str, cfg: &AiConfig, started: Instant) -> Result<Reply> {
    let v: Value = serde_json::from_str(text)
        .with_context(|| format!("the AI endpoint {} sent no JSON", cfg.public_endpoint()))?;
    let choice = &v["choices"][0];
    let content = choice["message"]["content"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let usage = Usage {
        input: v["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
        output: v["usage"]["completion_tokens"].as_u64().unwrap_or(0),
    };
    if content.trim().is_empty() {
        if choice["finish_reason"] == "length" {
            bail!(
                "the model used up max_tokens ({}) before it answered; raise `max_tokens` in [ai]",
                cfg.max_tokens
            );
        }
        bail!("the model sent an empty answer");
    }
    Ok(Reply {
        content,
        usage,
        seconds: started.elapsed().as_secs_f64(),
    })
}

fn post(agent: &ureq::Agent, url: &str, cfg: &AiConfig, body: &Value) -> Result<(u16, String)> {
    let mut req = agent.post(url).header("Accept", "application/json");
    if let Some(key) = &cfg.api_key {
        req = req.header("Authorization", &format!("Bearer {key}"));
    }
    let mut resp = req.send_json(body).map_err(|e| {
        anyhow::anyhow!(
            "cannot reach the AI endpoint {}: {e}",
            cfg.public_endpoint()
        )
    })?;
    let status = resp.status().as_u16();
    let text = resp
        .body_mut()
        .with_config()
        .limit(20 * 1024 * 1024)
        .read_to_string()
        .context("reading the AI endpoint's answer")?;
    Ok((status, text))
}

/// The `error.message` of an OpenAI-style error body, else the body's start.
fn error_message(text: &str) -> String {
    let v: Option<Value> = serde_json::from_str(text).ok();
    let msg = v.as_ref().and_then(|v| {
        v["error"]["message"]
            .as_str()
            .or_else(|| v["error"].as_str())
            .or_else(|| v["message"].as_str())
            .map(str::to_string)
    });
    let mut msg = msg.unwrap_or_else(|| text.trim().to_string());
    if msg.len() > 300 {
        let cut = (0..=300)
            .rev()
            .find(|&i| msg.is_char_boundary(i))
            .unwrap_or(0);
        msg.truncate(cut);
        msg.push('…');
    }
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_messages_are_short_and_readable() {
        assert_eq!(
            error_message(r#"{"error":{"message":"Invalid API key","type":"auth"}}"#),
            "Invalid API key"
        );
        assert_eq!(error_message("upstream down"), "upstream down");
        assert!(error_message(&"x".repeat(1000)).chars().count() <= 301);
    }
}
