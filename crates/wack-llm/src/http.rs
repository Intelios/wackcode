//! The one HTTP client behind all three wire formats.

use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;

use crate::{
    DeltaSink, LlmError, Provider, Request, Response, WireFormat, anthropic, chat, error::ErrorKind, responses,
    sse::SseParser,
};

/// A provider reached over HTTP. It holds the key in memory for as long as it lives and puts it
/// nowhere but the request's authorization header.
pub struct HttpProvider {
    format: WireFormat,
    base_url: String,
    api_key: String,
    client: reqwest::Client,
}

impl HttpProvider {
    pub fn new(format: WireFormat, base_url: &str, api_key: &str) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(30))
            // A reply can take many minutes, but silence for this long means the connection died.
            .read_timeout(Duration::from_secs(300))
            .redirect(same_origin_redirects())
            // The web tools turn on reqwest's decompression for the whole build; provider streams
            // stay uncompressed, as they always were.
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .build()
            .map_err(|error| format!("Could not set up the network client: {error}"))?;
        Ok(Self {
            format,
            base_url: base_url.trim().trim_end_matches('/').to_string(),
            api_key: api_key.trim().to_string(),
            client,
        })
    }

    fn url(&self) -> String {
        match self.format {
            WireFormat::AnthropicMessages if self.base_url.ends_with("/v1") => format!("{}/messages", self.base_url),
            WireFormat::AnthropicMessages => format!("{}/v1/messages", self.base_url),
            WireFormat::OpenaiResponses => format!("{}/responses", self.base_url),
            WireFormat::OpenaiChat => format!("{}/chat/completions", self.base_url),
        }
    }

    fn host_is(&self, host: &str) -> bool {
        reqwest::Url::parse(&self.base_url).ok().and_then(|url| url.host_str().map(|h| h == host)).unwrap_or(false)
    }

    /// Removes the key from text that may be shown or stored, in case a server echoes it back.
    fn redact(&self, text: &str) -> String {
        if self.api_key.is_empty() { text.to_string() } else { text.replace(&self.api_key, "[key removed]") }
    }

    fn fail(&self, mut error: LlmError) -> LlmError {
        error.message = self.redact(&error.message);
        error
    }
}

enum Decoder {
    Anthropic(anthropic::Decoder),
    Chat(chat::Decoder),
    Responses(responses::Decoder),
}

#[async_trait]
impl Provider for HttpProvider {
    async fn stream(&self, request: &Request, deltas: DeltaSink<'_>) -> Result<Response, LlmError> {
        let body = match self.format {
            WireFormat::AnthropicMessages => anthropic::body(request, self.native_tool_search()),
            WireFormat::OpenaiChat => chat::body(request, self.host_is("api.openai.com")),
            WireFormat::OpenaiResponses => responses::body(request),
        };
        let mut call = self.client.post(self.url()).header("accept", "text/event-stream").json(&body);
        call = match self.format {
            WireFormat::AnthropicMessages => {
                call.header("x-api-key", &self.api_key).header("anthropic-version", anthropic::VERSION)
            }
            _ => call.bearer_auth(&self.api_key),
        };
        let reply = call.send().await.map_err(|error| self.fail(network(error)))?;

        let status = reply.status();
        if status.is_redirection() {
            return Err(self.fail(moved(&reply)));
        }
        if !status.is_success() {
            let retry_after = reply
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.trim().parse::<u64>().ok())
                .map(Duration::from_secs);
            let text = reply.text().await.unwrap_or_default();
            return Err(self.fail(LlmError::from_status(status.as_u16(), &error_detail(&text), retry_after)));
        }

        let mut decoder = match self.format {
            WireFormat::AnthropicMessages => Decoder::Anthropic(anthropic::Decoder::default()),
            WireFormat::OpenaiChat => Decoder::Chat(chat::Decoder::default()),
            WireFormat::OpenaiResponses => Decoder::Responses(responses::Decoder::default()),
        };
        let mut parser = SseParser::default();
        let mut bytes = reply.bytes_stream();
        while let Some(chunk) = bytes.next().await {
            let chunk = chunk.map_err(|error| self.fail(network(error)))?;
            for event in parser.push(&chunk) {
                if event.data == "[DONE]" {
                    continue;
                }
                let Ok(data) = serde_json::from_str::<Value>(&event.data) else {
                    continue;
                };
                match &mut decoder {
                    Decoder::Anthropic(decoder) => decoder.event(&data, deltas),
                    Decoder::Chat(decoder) => decoder.event(&data, deltas),
                    Decoder::Responses(decoder) => decoder.event(&data, deltas),
                }
                .map_err(|error| self.fail(error))?;
            }
        }
        match decoder {
            Decoder::Anthropic(decoder) => decoder.finish(),
            Decoder::Chat(decoder) => decoder.finish(),
            Decoder::Responses(decoder) => decoder.finish(),
        }
    }

    /// Only Anthropic's own endpoint is known to run tool search; a proxy speaking the same format
    /// may not, and the agent's own search tool works everywhere.
    fn native_tool_search(&self) -> bool {
        self.format == WireFormat::AnthropicMessages && self.host_is("api.anthropic.com")
    }
}

/// Redirects are followed only within the address's own origin, five at most. reqwest strips the
/// standard credential headers on a hop to another host, but not Anthropic's `x-api-key`, so a
/// key would otherwise follow a redirect anywhere (docs/security.md).
pub(crate) fn same_origin_redirects() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        let leaves = attempt.previous().first().is_some_and(|first| first.origin() != attempt.url().origin());
        if leaves || attempt.previous().len() > 5 { attempt.stop() } else { attempt.follow() }
    })
}

/// A redirect that was not followed: to another host, or one too many.
fn moved(reply: &reqwest::Response) -> LlmError {
    let to = reply
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .and_then(|location| reply.url().join(location).ok())
        .and_then(|url| url.host_str().map(str::to_string));
    let message = match to {
        Some(host) => format!(
            "The provider redirected the request to {host}. WackCode sends a key only to the address you set, so change the connection's base URL to the final address."
        ),
        None => "The provider kept redirecting the request. Change the connection's base URL to the final address.".to_string(),
    };
    LlmError::new(ErrorKind::Invalid, message)
}

fn network(error: reqwest::Error) -> LlmError {
    let message = if error.is_timeout() {
        "The provider stopped answering.".to_string()
    } else if error.is_connect() {
        "Could not connect to the provider.".to_string()
    } else {
        // reqwest names the URL in its message; the URL holds no key, but the detail is noise.
        format!("The connection to the provider failed: {}", error.without_url())
    };
    LlmError::new(ErrorKind::Network, message)
}

/// The provider's own words from an error body, whichever shape it used.
fn error_detail(body: &str) -> String {
    let Ok(json) = serde_json::from_str::<Value>(body) else {
        return body.chars().take(400).collect();
    };
    json["error"]["message"]
        .as_str()
        .or(json["message"].as_str())
        .or(json["error"].as_str())
        .or(json["detail"].as_str())
        .map(str::to_string)
        .unwrap_or_else(|| body.chars().take(400).collect())
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;
    use crate::{Message, StopReason, stream_with_retry, testing};

    /// A one-file HTTP server: answers each connection with the next canned reply and keeps what
    /// it was sent.
    async fn serve(replies: Vec<String>) -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let record = seen.clone();
        tokio::spawn(async move {
            for reply in replies {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0u8; 4096];
                loop {
                    let read = socket.read(&mut buffer).await.unwrap();
                    request.extend_from_slice(&buffer[..read]);
                    let text = String::from_utf8_lossy(&request);
                    if let Some(split) = text.find("\r\n\r\n") {
                        let length = text
                            .lines()
                            .find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap()))
                            .unwrap_or(0);
                        if request.len() >= split + 4 + length {
                            break;
                        }
                    }
                    if read == 0 {
                        break;
                    }
                }
                record.lock().unwrap().push(String::from_utf8_lossy(&request).to_string());
                socket.write_all(reply.as_bytes()).await.unwrap();
                socket.shutdown().await.ok();
            }
        });
        (address, seen)
    }

    fn sse(body: &str) -> String {
        format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}")
    }

    fn request() -> Request {
        Request {
            model: "m".into(),
            system: vec![],
            messages: vec![Message::user("hi")],
            tools: vec![],
            max_output_tokens: None,
            effort: None,
        }
    }

    const CHAT_REPLY: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";

    #[tokio::test]
    async fn retries_a_rate_limit_then_streams_the_reply() {
        let limited = "HTTP/1.1 429 Too Many Requests\r\nretry-after: 0\r\ncontent-length: 43\r\nconnection: close\r\n\r\n{\"error\":{\"message\":\"Slow down, sk-secret\"}}".to_string();
        let (address, seen) = serve(vec![limited, sse(CHAT_REPLY)]).await;
        let provider = HttpProvider::new(WireFormat::OpenaiChat, &address, "sk-secret").unwrap();
        let collected = testing::Collected::default();
        let notices = AtomicUsize::new(0);
        let reasons = std::sync::Mutex::new(Vec::new());
        let response = stream_with_retry(&provider, &request(), &|delta| collected.push(delta), &|notice| {
            notices.fetch_add(1, Ordering::Relaxed);
            reasons.lock().unwrap().push(notice.reason);
        })
        .await
        .unwrap();
        assert_eq!(response.stop, StopReason::EndTurn);
        assert_eq!(collected.text(), "Hello");
        assert_eq!(notices.load(Ordering::Relaxed), 1);
        assert!(!reasons.lock().unwrap()[0].contains("sk-secret"), "the key must never reach an error message");
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(seen[0].starts_with("POST /chat/completions "));
        assert!(seen[0].to_ascii_lowercase().contains("authorization: bearer sk-secret"));
    }

    #[tokio::test]
    async fn anthropic_requests_carry_the_key_and_version_headers() {
        let reply = sse("data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n");
        let (address, seen) = serve(vec![reply]).await;
        let provider = HttpProvider::new(WireFormat::AnthropicMessages, &address, "key").unwrap();
        assert!(!provider.native_tool_search());
        provider.stream(&request(), &|_| {}).await.unwrap();
        let sent = seen.lock().unwrap()[0].to_ascii_lowercase();
        assert!(sent.starts_with("post /v1/messages "));
        assert!(sent.contains("x-api-key: key"));
        assert!(sent.contains("anthropic-version: 2026-10-21"));
    }

    #[tokio::test]
    async fn an_overflow_is_not_retried() {
        let overflow = "HTTP/1.1 400 Bad Request\r\ncontent-length: 48\r\nconnection: close\r\n\r\n{\"error\":{\"message\":\"prompt is too long: 1 > 0\"}}".to_string();
        let (address, seen) = serve(vec![overflow]).await;
        let provider = HttpProvider::new(WireFormat::OpenaiResponses, &address, "key").unwrap();
        let error = stream_with_retry(&provider, &request(), &|_| {}, &|_| {}).await.unwrap_err();
        assert_eq!(error.kind, ErrorKind::ContextOverflow);
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_redirect_to_another_host_is_not_followed() {
        let (elsewhere, reached) = serve(vec![sse(CHAT_REPLY)]).await;
        let redirect = format!(
            "HTTP/1.1 307 Temporary Redirect\r\nlocation: {elsewhere}/v1/messages\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
        );
        let (address, _) = serve(vec![redirect]).await;
        let provider = HttpProvider::new(WireFormat::AnthropicMessages, &address, "sk-secret").unwrap();
        let error = provider.stream(&request(), &|_| {}).await.unwrap_err();
        assert!(error.message.contains("redirected the request to 127.0.0.1"), "{}", error.message);
        assert!(reached.lock().unwrap().is_empty(), "the key must not follow a redirect to another origin");
    }

    #[tokio::test]
    async fn a_redirect_within_the_origin_is_followed() {
        let redirect = "HTTP/1.1 307 Temporary Redirect\r\nlocation: /v2/chat/completions\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_string();
        let (address, seen) = serve(vec![redirect, sse(CHAT_REPLY)]).await;
        let provider = HttpProvider::new(WireFormat::OpenaiChat, &address, "sk-secret").unwrap();
        provider.stream(&request(), &|_| {}).await.unwrap();
        let seen = seen.lock().unwrap();
        assert!(seen[1].starts_with("POST /v2/chat/completions "));
        assert!(seen[1].to_ascii_lowercase().contains("authorization: bearer sk-secret"));
    }

    #[test]
    fn native_tool_search_is_only_for_anthropics_own_host() {
        let own = HttpProvider::new(WireFormat::AnthropicMessages, "https://api.anthropic.com/", "k").unwrap();
        assert!(own.native_tool_search());
        assert_eq!(own.url(), "https://api.anthropic.com/v1/messages");
        let proxy = HttpProvider::new(WireFormat::AnthropicMessages, "https://proxy.example/v1", "k").unwrap();
        assert!(!proxy.native_tool_search());
        assert_eq!(proxy.url(), "https://proxy.example/v1/messages");
    }
}
