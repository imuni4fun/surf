//! HTTP Retry-after middleware.
//!
//! # Examples
//!
//! ```no_run
//! # #[async_std::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
//! let req = surf::get("https://httpbin.org/retry/2");
//! let client = surf::client().with(surf::middleware::Retry::new(5));
//! let mut res = client.send(req).await?;
//! dbg!(res.body_string().await?);
//! # Ok(()) }
//! ```

use crate::http::{headers, StatusCode};
use crate::middleware::{Middleware, Next, Request, Response};
use crate::{Client, Result};
use chrono::*;

// List of acceptible 300-series redirect codes.
const RETRY_AFTER_CODES: &[StatusCode] = &[
    StatusCode::MovedPermanently,
    StatusCode::TooManyRequests,
    StatusCode::ServiceUnavailable,
];

/// A middleware which retries throttled requests.
#[derive(Debug)]
pub struct RetryAfter {
    attempts: u8,
    max_delay_sec: u16,
    deadline_sec: u16,
}

impl RetryAfter {
    /// Create a new instance of the RetryAfter middleware, which retries throttled requests up
    /// to as many times as specified and while observing the individual and cumulative deadlines.
    ///
    /// This middleware attempts to comply with the following definition:
    /// https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Retry-After
    ///
    /// This middleware checks for a retry-after header upon receiving one of the following response codes:
    /// - 301 Moved Permanently
    /// - 429 Too Many Requests
    /// - 503 Service Unavailable
    ///
    /// # Errors
    ///
    /// An error will be passed through the middleware stack if the value of the `Retry-after`
    /// header is not a validly parsing integer or date.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # #[async_std::main]
    /// # async fn main() -> surf::Result<()> {
    /// let req = surf::get("https://httpbin.org/RetryAfter/2");
    /// let client = surf::client().with(surf::middleware::RetryAfter::new(5));
    /// let mut res = client.send(req).await?;
    /// dbg!(res.body_string().await?);
    /// # Ok(()) }
    /// ```
    pub fn new(attempts: u8, max_delay_sec: u16, deadline_sec: u16) -> Self {
        RetryAfter {
            attempts,
            max_delay_sec,
            deadline_sec,
        }
    }
}

#[async_trait::async_trait]
impl Middleware for RetryAfter {
    #[allow(missing_doc_code_examples)]
    async fn handle(&self, mut req: Request, client: Client, next: Next<'_>) -> Result<Response> {
        let mut RetryAfter_count: u8 = 0;

        let mut base_url = req.url().clone();

        while RetryAfter_count < self.attempts {
            RetryAfter_count += 1;
            let r: Request = req.clone();
            let res: Response = client.send(r).await?;
            if RETRY_AFTER_CODES.contains(&res.status()) {
                if let Some(retry) = res.header(headers::RETRY_AFTER) {
                    let retry_header_value = retry.last().as_str();
                    let delay = if let delay_sec = retry_header_value.parse::<u16>() {
                        Some(chrono::Duration::seconds(delay_sec as i64))
                    } else if let future_time = retry_header_value.parse::<chrono::DateTime>() {
                        let delay = future_time - chrono::DateTime::now();
                        if delay.milliseconds() > 0 {
                            Some(delay)
                        }
                    } else {
                        // invalid retry-after header
                        None
                    };
                    match delay {
                        Some(delay) => Task::delay(delay).await,
                        None => {
                            return Err(format!(
                                "Invalid retry-after header ('{}') in response {}",
                                &retry_header_value,
                                &res.status()
                            ))
                        }
                    }
                }
            } else {
                break;
            }
        }

        Ok(next.run(req, client).await?)
    }
}

impl Default for RetryAfter {
    /// Create a new instance of the RetryAfter middleware.
    fn default() -> Self {
        Self {
            attempts: 3,
            max_delay_sec: 30,
            deadline_sec: 60,
        };
    }
}
