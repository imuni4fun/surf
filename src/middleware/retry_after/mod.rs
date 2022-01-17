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

use std::fmt::Arguments;
use std::time::Duration;

use crate::http::{headers, StatusCode};
use crate::middleware::{Middleware, Next, Request, Response};
use crate::{Client, Result};
// use chrono::*;
use async_std::task;
use chrono::NaiveDateTime;
use time;

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
    /// let client = surf::client().with(surf::middleware::RetryAfter::new(5, 30, 60));
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
        let mut count: u8 = 0;
        let mut accumulated_duration = Duration::ZERO;

        let mut base_url = req.url().clone();

        while count < self.attempts {
            let r: Request = req.clone();
            let res: Response = client.send(r).await?;
            if RETRY_AFTER_CODES.contains(&res.status()) {
                if let Some(retry) = res.header(headers::RETRY_AFTER) {
                    // header present, parse it to extract delay
                    let retry_header_value = retry.last().as_str();
                    print(
                        log::Level::Info,
                        format_args!(
                            "{} {} response contained retry header {} {}",
                            req.method(),
                            req.url(),
                            headers::RETRY_AFTER,
                            retry_header_value,
                        ),
                    );
                    let delay = if let Ok(delay_sec) = retry_header_value.parse::<u64>() {
                        Some(Duration::new(delay_sec, 0))
                    } else if let Ok(delay_sec) = delay_from_date_str(retry_header_value) {
                        Some(delay_sec)
                    } else {
                        // invalid retry-after header
                        None
                    };
                    match delay {
                        // delay valid, apply it unless it exceeds limits
                        Some(delay) => {
                            if (self.max_delay_sec as f32) < delay.as_secs_f32() {
                                break; // stop retry behavior
                            }
                            accumulated_duration += delay;
                            if (self.deadline_sec as f32) < accumulated_duration.as_secs_f32() {
                                break; // stop retry behavior
                            }
                            count += 1;
                            task::sleep(delay).await; // sleep an retry
                        }
                        // delay invalid, continue processing
                        None => {
                            break; // stop retry behavior
                                   // log.warn!(
                                   //     "Invalid retry-after header ('{}') in response {}",
                                   //     &retry_header_value,
                                   //     &res.status()
                                   // );
                        }
                    }
                }
            } else {
                // headers::RETRY_AFTER not present, no retry
                break;
            }
        }

        Ok(next.run(req, client).await?)
    }
}

/// If cannot parse, returns error.
/// If parsed value is in future, returns duration to wait.
/// If parsed value is not in future, returns zero duration.
/// reference: https://docs.rs/hyper/0.11.7/src/hyper/header/shared/httpdate.rs.html#35-44
fn delay_from_date_str(s: &str) -> Result<Duration> {
    match time::strptime(s, "%a, %d %b %Y %T %Z")
        .or_else(|_| time::strptime(s, "%A, %d-%b-%y %T %Z"))
        .or_else(|_| time::strptime(s, "%c"))
    {
        Ok(t) => Ok(NaiveDateTime::from_timestamp(t.to_timespec().sec, 0)
            .signed_duration_since(chrono::offset::Utc::now().naive_utc())
            .to_std()
            .unwrap_or(Duration::ZERO)),
        Err(e) => Err(http_types::Error::from_str(
            StatusCode::InternalServerError,
            format!("could not parse date: {}", s),
        )),
    }
}

impl Default for RetryAfter {
    /// Create a new instance of the RetryAfter middleware.
    fn default() -> Self {
        Self {
            attempts: 3,
            max_delay_sec: 30,
            deadline_sec: 60,
        }
    }
}

fn print(level: log::Level, msg: Arguments<'_>) {
    log::logger().log(
        &log::Record::builder()
            .args(msg)
            .level(level)
            .line(Some(line!()))
            .build(),
    );
}
