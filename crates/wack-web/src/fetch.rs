//! The network half of `web_fetch`: one GET, following redirects by hand so each hop is checked.
//!
//! The client's DNS resolver is [`PublicOnly`], so a connection only ever reaches addresses the
//! guard approved. Nothing about the user travels with the request: no cookies (the client has no
//! cookie store), no auth headers, no proxy (environment proxies are ignored). The body is
//! decompressed as it streams and capped after decoding, so a small compression bomb cannot grow
//! past the cap.

use std::{sync::Arc, time::Duration};

use reqwest::{StatusCode, header};
use url::Url;

use crate::guard::{PublicOnly, Refused, check_url};
pub use crate::guard::Policy;

pub const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
/// Decoded body bytes kept; the rest of a larger response is dropped.
pub const MAX_BODY_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_REDIRECTS: usize = 5;
const USER_AGENT: &str = "WackCode-web_fetch/1.0";
const ACCEPT: &str = "text/html,application/xhtml+xml,text/markdown,text/plain;q=0.9,application/json;q=0.9,*/*;q=0.5";

pub struct Fetched {
    pub final_url: Url,
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
    /// The body reached the cap and was cut off there.
    pub truncated: bool,
}

/// One HTTP client for reading public pages, shared by every fetch of a run.
pub struct Fetcher {
    client: reqwest::Client,
    policy: Policy,
    timeout: Duration,
    max_bytes: usize,
}

impl Fetcher {
    pub fn new(policy: Policy) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .dns_resolver(Arc::new(PublicOnly(policy.clone())))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|error| format!("Could not set up web access: {error}."))?;
        Ok(Self { client, policy, timeout: FETCH_TIMEOUT, max_bytes: MAX_BODY_BYTES })
    }

    #[cfg(test)]
    pub fn with_limits(mut self, timeout: Duration, max_bytes: usize) -> Self {
        self.timeout = timeout;
        self.max_bytes = max_bytes;
        self
    }

    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    /// Fetches `raw`, following at most [`MAX_REDIRECTS`] redirects. Every error is a sentence
    /// safe to show the model.
    pub async fn get(&self, raw: &str) -> Result<Fetched, String> {
        let seconds = self.timeout.as_secs();
        match tokio::time::timeout(self.timeout, self.follow(raw)).await {
            Ok(result) => result,
            Err(_) => Err(format!("Timed out after {seconds} s fetching {}.", raw.trim())),
        }
    }

    async fn follow(&self, raw: &str) -> Result<Fetched, String> {
        let mut url = check_url(raw, &self.policy)?;
        for hop in 0.. {
            let mut response = self
                .client
                .get(url.clone())
                .header(header::ACCEPT, ACCEPT)
                .send()
                .await
                .map_err(|error| describe(&error, &url))?;
            let status = response.status();
            if is_redirect(status)
                && let Some(location) = response.headers().get(header::LOCATION).and_then(|value| value.to_str().ok())
            {
                if hop >= MAX_REDIRECTS {
                    return Err(format!("Stopped after {MAX_REDIRECTS} redirects."));
                }
                let next = url.join(location).map_err(|_| format!("{url} redirected to an invalid URL."))?;
                url = check_url(next.as_str(), &self.policy)?;
                continue;
            }
            if !status.is_success() {
                let reason = status.canonical_reason().map(|reason| format!(" {reason}")).unwrap_or_default();
                return Err(format!("{url} returned HTTP {}{reason}.", status.as_u16()));
            }
            let content_type = response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_string();
            let mut body = Vec::new();
            let mut truncated = false;
            while let Some(chunk) = response.chunk().await.map_err(|error| describe(&error, &url))? {
                if body.len() + chunk.len() > self.max_bytes {
                    body.extend_from_slice(&chunk[..self.max_bytes - body.len()]);
                    truncated = true;
                    break;
                }
                body.extend_from_slice(&chunk);
            }
            return Ok(Fetched { final_url: url, status: status.as_u16(), content_type, body, truncated });
        }
        unreachable!("the loop returns")
    }
}

fn is_redirect(status: StatusCode) -> bool {
    matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
}

/// A reqwest error as a sentence: the guard's own refusal when that is the cause.
fn describe(error: &reqwest::Error, url: &Url) -> String {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(cause) = source {
        if let Some(refused) = cause.downcast_ref::<Refused>() {
            return refused.0.clone();
        }
        source = cause.source();
    }
    let text = format!("{error:?}").to_ascii_lowercase();
    let reason = if text.contains("certificate") || text.contains("tls") {
        "its TLS certificate could not be verified."
    } else if text.contains("connection refused") {
        "the connection was refused."
    } else if text.contains("connection reset") {
        "the connection was reset."
    } else if error.is_timeout() {
        "it took too long to answer."
    } else if error.is_connect() {
        "the connection failed."
    } else if error.is_decode() || error.is_body() {
        "the page could not be read to the end."
    } else {
        "the request failed."
    };
    format!("Could not fetch {url}: {reason}")
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::Write as _;

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;

    /// A tiny HTTP/1.1 server on loopback that answers each path the way the tests need.
    pub async fn serve() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { return };
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut buffer = [0u8; 1024];
                    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                        match socket.read(&mut buffer).await {
                            Ok(0) | Err(_) => return,
                            Ok(read) => request.extend_from_slice(&buffer[..read]),
                        }
                    }
                    let line = String::from_utf8_lossy(&request);
                    let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                    let (status, headers, body): (&str, Vec<String>, Vec<u8>) = match path.as_str() {
                        "/page" => ("200 OK", vec!["content-type: text/html; charset=utf-8".into()], b"<html><head><title>Hi</title></head><body><p>Hello</p></body></html>".to_vec()),
                        "/json" => ("200 OK", vec!["content-type: application/json".into()], br#"{"ok":true}"#.to_vec()),
                        "/gzip" => {
                            let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
                            encoder.write_all(b"compressed text").unwrap();
                            ("200 OK", vec!["content-type: text/plain".into(), "content-encoding: gzip".into()], encoder.finish().unwrap())
                        }
                        "/redirect" => ("302 Found", vec!["location: /json".into()], Vec::new()),
                        "/redirect-private" => ("301 Moved", vec!["location: http://10.0.0.1/admin".into()], Vec::new()),
                        "/redirect-file" => ("301 Moved", vec!["location: file:///etc/passwd".into()], Vec::new()),
                        loop_path if loop_path.starts_with("/loop") => ("302 Found", vec![format!("location: /loop{}", loop_path.len())], Vec::new()),
                        "/big" => ("200 OK", vec!["content-type: text/plain".into()], vec![b'x'; 10_000]),
                        "/image" => ("200 OK", vec!["content-type: image/png".into()], vec![0x89, b'P', b'N', b'G', 0, 0, 0]),
                        "/slow" => {
                            let head = "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: 100\r\n\r\nstart";
                            let _ = socket.write_all(head.as_bytes()).await;
                            // Never finishes; the client's timeout must.
                            tokio::time::sleep(Duration::from_secs(60)).await;
                            return;
                        }
                        _ => ("404 Not Found", vec!["content-type: text/plain".into()], b"nope".to_vec()),
                    };
                    let mut head = format!("HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n", body.len());
                    for header in headers {
                        head.push_str(&header);
                        head.push_str("\r\n");
                    }
                    head.push_str("\r\n");
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(&body).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        base
    }

    fn loopback() -> Fetcher {
        Fetcher::new(Policy::loopback_for_tests()).unwrap()
    }

    #[tokio::test]
    async fn refuses_loopback_under_the_production_policy() {
        let base = serve().await;
        let error = Fetcher::new(Policy::public()).unwrap().get(&format!("{base}/page")).await.err().unwrap();
        assert!(error.contains("local or private network address"), "{error}");
    }

    #[tokio::test]
    async fn reads_a_page_decodes_gzip_and_follows_a_redirect() {
        let base = serve().await;
        let fetcher = loopback();
        let page = fetcher.get(&format!("{base}/page")).await.unwrap();
        assert_eq!(page.status, 200);
        assert!(page.content_type.contains("text/html"));
        assert!(String::from_utf8_lossy(&page.body).contains("<p>Hello</p>"));

        assert_eq!(fetcher.get(&format!("{base}/gzip")).await.unwrap().body, b"compressed text");

        let redirected = fetcher.get(&format!("{base}/redirect")).await.unwrap();
        assert_eq!(redirected.final_url.as_str(), format!("{base}/json"));
        assert_eq!(redirected.body, br#"{"ok":true}"#);
    }

    #[tokio::test]
    async fn checks_every_redirect_hop() {
        let base = serve().await;
        let fetcher = loopback();
        let private = fetcher.get(&format!("{base}/redirect-private")).await.err().unwrap();
        assert!(private.contains("10.0.0.1 is a local or private"), "{private}");
        let file = fetcher.get(&format!("{base}/redirect-file")).await.err().unwrap();
        assert!(file.contains("Only http and https"), "{file}");
        let looped = fetcher.get(&format!("{base}/loop")).await.err().unwrap();
        assert!(looped.contains("Stopped after 5 redirects"), "{looped}");
    }

    #[tokio::test]
    async fn caps_the_body_times_out_and_reports_http_errors() {
        let base = serve().await;
        let big = loopback().with_limits(FETCH_TIMEOUT, 1_000).get(&format!("{base}/big")).await.unwrap();
        assert_eq!(big.body.len(), 1_000);
        assert!(big.truncated);

        let slow = loopback().with_limits(Duration::from_millis(300), MAX_BODY_BYTES).get(&format!("{base}/slow")).await;
        assert!(slow.err().unwrap().contains("Timed out"));

        let missing = loopback().get(&format!("{base}/missing")).await.err().unwrap();
        assert!(missing.contains("HTTP 404"), "{missing}");
    }
}
