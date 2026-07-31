//! The shared Pangram 4 text-analysis module.
//!
//! This module is the sole owner of Pangram protocol behavior: explicit
//! Pangram 4 text submission, safe task polling, upstream response
//! normalization, rate limiting, safe-GET retry with bounded backoff, local
//! wait timeouts, and local cancellation. CLI, TUI, and MCP adapters call
//! the typed [`Analyzer`] surface; they never construct HTTP requests
//! themselves.
//!
//! Safety invariants:
//!
//! - Production requests go only to the fixed Pangram 4 text endpoints with
//!   `x-api-key` authentication, TLS through rustls and the platform
//!   verifier, system proxy discovery, and redirect following disabled.
//!   There is no production endpoint, environment, or flag override.
//! - A billable POST is never replayed after an ambiguous send outcome.
//!   Ambiguity yields `submission_outcome_unknown` (non-retryable) with the
//!   fixed duplicate-billing recovery.
//! - Safe GET polling retries bounded transient failures, honors
//!   `Retry-After`, and uses decorrelated bounded backoff with jitter.
//! - Client throughput never exceeds 5 requests per second; configuration
//!   may only lower it.
//! - Wait timeouts and cancellation stop local observation only. No remote
//!   cancellation request is ever sent.
//! - Credentials, auth headers, submitted content, and raw response bodies
//!   never enter errors, `Debug` output, or serialized error details.

mod config;
mod handle;
mod http;
mod normalize;
mod semaphore;
mod task;
mod upstream;

pub use config::{AnalysisConfig, PollPolicy, RetryPolicy, WaitOptions};
pub use handle::{AnalysisProgress, Analyzer, InterruptedAnalysis, RunningAnalysis, StopObserving};
pub use task::{Accepted, AcceptedInput, AnalysisRequest, AnalysisResult, TaskError};
pub use upstream::{AnalysisError, UpstreamClient, UpstreamEndpoints};

pub use crate::config::MAX_REQUESTS_PER_SECOND;
pub use tokio::time::{Duration, Instant};
