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
//!   `Retry-After`, and uses decorrelated bounded backoff with jitter. The
//!   retry chain observes the caller's wait deadline and a cumulative
//!   retry-time budget, so a wait timeout or cancellation interrupts pending
//!   retry sleeps promptly.
//! - Every request (submit and poll) is issued through one shared time-based
//!   issue gate that enforces the hard 5-requests-per-second ceiling on
//!   request issue timing; configuration may only lower the rate.
//! - Wait timeouts and cancellation stop local observation only. No remote
//!   cancellation request is ever sent.
//! - Credentials, auth headers, submitted content, and raw response bodies
//!   never enter errors, `Debug` output, or serialized error details.
//!   Upstream-reported failure text is reduced (control sequences stripped,
//!   non-printable bytes removed, bounded length) before it can appear in
//!   canonical details.

mod config;
mod handle;
mod http;
mod normalize;
mod pacemaker;
mod task;
mod upstream;

pub use config::{AnalysisConfig, PollPolicy, RetryPolicy, WaitOptions};
pub use handle::{
    AnalysisProgress, Analyzer, InterruptedAnalysis, OperationIdentity, RunningAnalysis,
    StopObserving,
};
pub use task::{Accepted, AcceptedInput, AnalysisRequest, AnalysisResult, TaskError};
pub use upstream::{AnalysisError, SubmissionFailure, UpstreamClient, UpstreamEndpoints};

pub use crate::config::MAX_REQUESTS_PER_SECOND;
pub use tokio::time::{Duration, Instant};
