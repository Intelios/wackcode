//! Retrying a request that failed for a reason that may pass.

use std::time::Duration;

use crate::{Delta, DeltaSink, LlmError, Provider, Request, Response};

const MAX_ATTEMPTS: u32 = 5;
const LONGEST_WAIT: Duration = Duration::from_secs(60);

/// Told to the caller before each wait, so the user can see why nothing is happening.
#[derive(Clone, Debug, PartialEq)]
pub struct RetryNotice {
    /// The attempt about to be made, counting the first as 1.
    pub attempt: u32,
    pub wait: Duration,
    pub reason: String,
}

/// Sends `request`, trying again after rate limits, provider failures and dropped connections.
///
/// A request that already delivered text or tool input is not retried: the caller has shown that
/// output, and a second attempt would produce a different answer under it.
pub async fn stream_with_retry(
    provider: &dyn Provider,
    request: &Request,
    deltas: DeltaSink<'_>,
    notify: &(dyn Fn(RetryNotice) + Send + Sync),
) -> Result<Response, LlmError> {
    let mut attempt = 1;
    loop {
        let delivered = std::sync::atomic::AtomicBool::new(false);
        let watch = |delta: Delta| {
            delivered.store(true, std::sync::atomic::Ordering::Relaxed);
            deltas(delta);
        };
        match provider.stream(request, &watch).await {
            Ok(response) => return Ok(response),
            Err(error) => {
                let delivered = delivered.load(std::sync::atomic::Ordering::Relaxed);
                if !error.retryable() || delivered || attempt >= MAX_ATTEMPTS {
                    return Err(error);
                }
                let wait = error.retry_after.unwrap_or(backoff(attempt)).min(LONGEST_WAIT);
                attempt += 1;
                notify(RetryNotice { attempt, wait, reason: error.message });
                tokio::time::sleep(wait).await;
            }
        }
    }
}

fn backoff(attempt: u32) -> Duration {
    Duration::from_secs(1 << (attempt - 1).min(5))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_double_each_time() {
        assert_eq!(backoff(1), Duration::from_secs(1));
        assert_eq!(backoff(2), Duration::from_secs(2));
        assert_eq!(backoff(4), Duration::from_secs(8));
    }
}
