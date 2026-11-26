//! Reading models.dev's feed, only when the user presses Refresh (or the maintainer runs `cargo
//! catalog`). It sends no key, no cookie and nothing about the user: a plain GET with a generic
//! User-Agent. A redirect is refused rather than followed, and the body is read only up to a cap.

use std::time::Duration;

/// How long a download may take, and how much of it is read.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub timeout: Duration,
    pub cap: usize,
}

impl Default for Limits {
    /// Roomy on purpose: the feed was 7.4 MB on 2026-11-26 and grows. A cap it outgrows would
    /// fail every refresh, as version 2's 16 MiB one eventually would have.
    fn default() -> Self {
        Self { timeout: Duration::from_secs(60), cap: 32 * 1024 * 1024 }
    }
}

/// The feed's bytes. Each error is one sentence naming models.dev.
pub async fn download(url: &str, limits: Limits) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .timeout(limits.timeout)
        .user_agent(concat!("WackCode/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| format!("Could not set up the network client: {error}"))?;
    let mut reply = client.get(url).header("accept", "application/json").send().await.map_err(|error| {
        if error.is_timeout() {
            "models.dev didn't answer in time.".to_string()
        } else if error.is_connect() {
            "Couldn't reach models.dev. Check the connection to the internet.".to_string()
        } else {
            format!("The request to models.dev failed: {}", error.without_url())
        }
    })?;
    let status = reply.status();
    if status.is_redirection() {
        return Err("models.dev sent the request somewhere else, so it wasn't followed.".to_string());
    }
    if !status.is_success() {
        return Err(format!("models.dev answered with HTTP {}.", status.as_u16()));
    }
    let too_big = || format!("models.dev's list is larger than the {} MB WackCode reads.", limits.cap / (1024 * 1024));
    if reply.content_length().is_some_and(|length| length > limits.cap as u64) {
        return Err(too_big());
    }
    let mut body = Vec::new();
    loop {
        let chunk = reply.chunk().await.map_err(|error| {
            if error.is_timeout() {
                "models.dev stopped sending its list part-way.".to_string()
            } else {
                format!("The list from models.dev broke off: {}", error.without_url())
            }
        })?;
        let Some(chunk) = chunk else { break };
        body.extend_from_slice(&chunk);
        if body.len() > limits.cap {
            return Err(too_big());
        }
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;

    /// Answers one connection with `reply`, then closes.
    async fn serve(reply: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0u8; 4096];
            let _ = socket.read(&mut buffer).await;
            let _ = socket.write_all(&reply).await;
            let _ = socket.shutdown().await;
        });
        address
    }

    fn limits(cap: usize) -> Limits {
        Limits { timeout: Duration::from_secs(5), cap }
    }

    #[tokio::test]
    async fn reads_the_body() {
        let address = serve(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}".to_vec()).await;
        assert_eq!(download(&address, limits(1024)).await.unwrap(), b"{}");
    }

    #[tokio::test]
    async fn refuses_a_redirect_a_failure_and_an_oversized_list() {
        let moved = serve(b"HTTP/1.1 302 Found\r\nlocation: http://elsewhere.invalid/\r\ncontent-length: 0\r\n\r\n".to_vec()).await;
        assert!(download(&moved, limits(1024)).await.unwrap_err().contains("somewhere else"));
        let down = serve(b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\n\r\n".to_vec()).await;
        assert_eq!(download(&down, limits(1024)).await.unwrap_err(), "models.dev answered with HTTP 503.");
        let stated = serve(b"HTTP/1.1 200 OK\r\ncontent-length: 4000000\r\n\r\n".to_vec()).await;
        assert!(download(&stated, limits(1024 * 1024)).await.unwrap_err().contains("larger than the 1 MB"));
        let mut streamed = b"HTTP/1.1 200 OK\r\nconnection: close\r\n\r\n".to_vec();
        streamed.extend(std::iter::repeat_n(b'x', 3 * 1024 * 1024));
        let streamed = serve(streamed).await;
        assert!(download(&streamed, limits(1024 * 1024)).await.unwrap_err().contains("larger than"), "counted while reading");
    }

    #[tokio::test]
    async fn a_server_that_isnt_there_is_said_plainly() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        assert!(download(&address, limits(1024)).await.unwrap_err().starts_with("Couldn't reach models.dev"));
    }
}
