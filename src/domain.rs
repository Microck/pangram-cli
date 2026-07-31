//! Canonical values shared by every Pangram adapter.
//!
//! Validation belongs at construction and deserialization boundaries. Once a
//! value exists, downstream code can trust its invariants without checking it
//! again.

use std::fmt;
use std::ops::Deref;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::{Uuid, Version};

mod collection;
mod model;

pub use collection::*;
pub use model::*;

const ANALYSIS_ID_PATTERN: &str =
    r"^anl_[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$";
const BULK_ID_PATTERN: &str =
    r"^bulk_[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$";
const SHA256_PATTERN: &str = r"^[0-9a-f]{64}$";

/// Optional contract fields use omission for absence; explicit null is invalid.
pub(crate) fn deserialize_missing_only<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)?
        .map(Some)
        .ok_or_else(|| D::Error::custom("optional fields must be omitted, not null"))
}

/// A domain value failed its canonical contract.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("expected a canonical {0} UUIDv7 identifier")]
    InvalidLocalId(&'static str),
    #[error("{0} must not be empty")]
    EmptyValue(&'static str),
    #[error("expected a lowercase hexadecimal SHA-256 digest")]
    InvalidSha256,
    #[error("expected an RFC 3339 UTC timestamp ending in Z")]
    InvalidTimestamp,
    #[error("{0} is outside its allowed range")]
    OutOfRange(&'static str),
    #[error("at least one check is required")]
    EmptyChecks,
    #[error("an analysis supports at most two checks")]
    TooManyChecks,
    #[error("an analysis cannot contain duplicate checks")]
    DuplicateCheck,
    #[error("AI detection must precede plagiarism when both checks are present")]
    InvalidCheckOrder,
    #[error("serialized analysis status does not match its check states")]
    AnalysisStatusMismatch,
    #[error("invalid {0} state payload")]
    InvalidState(&'static str),
    #[error("submission outcome is inconsistent with status or upstream identity")]
    InvalidSubmissionOutcome,
    #[error("submission details require exactly one local operation identity")]
    InvalidSubmissionIdentity,
    #[error("an analysis cannot have both retry and rerun lineage")]
    ConflictingLineage,
    #[error("provenance cannot repeat an upstream task ID")]
    DuplicateUpstreamTaskId,
    #[error("bulk counters exceed the validated item total")]
    InvalidBulkCounters,
    #[error("bulk status does not match its exact counters")]
    InvalidBulkStatus,
    #[error("bulk page items must use strictly ascending source indexes")]
    UnorderedBulkItems,
}

macro_rules! local_id {
    ($name:ident, $prefix:literal, $kind:literal, $pattern:expr) => {
        #[derive(Clone, Copy, Debug, Hash, JsonSchema, PartialEq, Eq, PartialOrd, Ord)]
        #[schemars(transparent)]
        pub struct $name(#[schemars(with = "String", regex(pattern = $pattern))] Uuid);

        impl $name {
            /// Generates a time-sortable local identity.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            #[must_use]
            pub const fn uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, concat!($prefix, "{}"), self.0)
            }
        }

        impl FromStr for $name {
            type Err = DomainError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                let suffix = value
                    .strip_prefix($prefix)
                    .ok_or(DomainError::InvalidLocalId($kind))?;
                let uuid =
                    Uuid::parse_str(suffix).map_err(|_| DomainError::InvalidLocalId($kind))?;

                // Uuid accepts uppercase and non-hyphenated spellings. Requiring
                // its canonical display form also enforces lowercase hex.
                if uuid.get_version() != Some(Version::SortRand) || suffix != uuid.to_string() {
                    return Err(DomainError::InvalidLocalId($kind));
                }

                Ok(Self(uuid))
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                String::deserialize(deserializer)?
                    .parse()
                    .map_err(D::Error::custom)
            }
        }
    };
}

local_id!(AnalysisId, "anl_", "analysis", ANALYSIS_ID_PATTERN);
local_id!(BulkId, "bulk_", "bulk", BULK_ID_PATTERN);

/// A non-empty provider-authored identifier or state string.
#[derive(Clone, Debug, Hash, JsonSchema, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct NonEmptyString(#[schemars(length(min = 1))] String);

impl NonEmptyString {
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        if value.is_empty() {
            return Err(DomainError::EmptyValue("value"));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Display for NonEmptyString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl Deref for NonEmptyString {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl TryFrom<String> for NonEmptyString {
    type Error = DomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl FromStr for NonEmptyString {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for NonEmptyString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::try_from(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

macro_rules! upstream_id {
    ($name:ident, $field:literal) => {
        #[derive(Clone, Debug, Hash, JsonSchema, PartialEq, Eq, PartialOrd, Ord, Serialize)]
        #[serde(transparent)]
        pub struct $name(#[schemars(length(min = 1))] String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
                let value = value.into();
                if value.is_empty() {
                    return Err(DomainError::EmptyValue($field));
                }
                Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = DomainError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                Self::new(String::deserialize(deserializer)?).map_err(D::Error::custom)
            }
        }
    };
}

upstream_id!(UpstreamTaskId, "upstream task ID");
upstream_id!(UpstreamBulkId, "upstream bulk ID");

/// A lowercase SHA-256 digest.
#[derive(Clone, Copy, Debug, Hash, JsonSchema, PartialEq, Eq, PartialOrd, Ord)]
#[schemars(transparent)]
pub struct Sha256Hash(#[schemars(with = "String", regex(pattern = SHA256_PATTERN))] [u8; 32]);

impl Sha256Hash {
    #[must_use]
    pub fn digest(value: impl AsRef<[u8]>) -> Self {
        Self(Sha256::digest(value.as_ref()).into())
    }

    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for Sha256Hash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl FromStr for Sha256Hash {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        fn nibble(value: u8) -> Option<u8> {
            match value {
                b'0'..=b'9' => Some(value - b'0'),
                b'a'..=b'f' => Some(value - b'a' + 10),
                _ => None,
            }
        }

        let encoded = value.as_bytes();
        if encoded.len() != 64 {
            return Err(DomainError::InvalidSha256);
        }

        let mut bytes = [0_u8; 32];
        for (index, pair) in encoded.chunks_exact(2).enumerate() {
            let high = nibble(pair[0]).ok_or(DomainError::InvalidSha256)?;
            let low = nibble(pair[1]).ok_or(DomainError::InvalidSha256)?;
            bytes[index] = (high << 4) | low;
        }
        Ok(Self(bytes))
    }
}

impl Serialize for Sha256Hash {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Sha256Hash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(D::Error::custom)
    }
}

/// An absolute UTC timestamp in the canonical `Z` form.
#[derive(Clone, Copy, Debug, Hash, JsonSchema, PartialEq, Eq, PartialOrd, Ord)]
#[schemars(transparent)]
pub struct UtcTimestamp(#[schemars(regex(pattern = r"Z$"))] jiff::Timestamp);

impl UtcTimestamp {
    #[must_use]
    pub fn now() -> Self {
        Self(jiff::Timestamp::now())
    }

    #[must_use]
    pub const fn get(self) -> jiff::Timestamp {
        self.0
    }
}

impl fmt::Display for UtcTimestamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for UtcTimestamp {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if !value.ends_with('Z') {
            return Err(DomainError::InvalidTimestamp);
        }
        value
            .parse()
            .map(Self)
            .map_err(|_| DomainError::InvalidTimestamp)
    }
}

impl Serialize for UtcTimestamp {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for UtcTimestamp {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(D::Error::custom)
    }
}

/// A finite value from 0.0 through 1.0.
#[derive(Clone, Copy, Debug, JsonSchema, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
#[schemars(transparent)]
pub struct Fraction(#[schemars(range(min = 0.0, max = 1.0))] f64);

impl Fraction {
    pub fn new(value: f64) -> Result<Self, DomainError> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(DomainError::OutOfRange("fraction"));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl<'de> Deserialize<'de> for Fraction {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(f64::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

/// A finite percentage from 0.0 through 100.0.
#[derive(Clone, Copy, Debug, JsonSchema, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
#[schemars(transparent)]
pub struct Percentage(#[schemars(range(min = 0.0, max = 100.0))] f64);

impl Percentage {
    pub fn new(value: f64) -> Result<Self, DomainError> {
        if !value.is_finite() || !(0.0..=100.0).contains(&value) {
            return Err(DomainError::OutOfRange("percentage"));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl<'de> Deserialize<'de> for Percentage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(f64::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Hash, JsonSchema, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckKind {
    AiDetection,
    Plagiarism,
}

#[derive(Clone, Copy, Debug, Hash, JsonSchema, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
}

#[derive(Clone, Copy, Debug, Hash, JsonSchema, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Partial,
}

/// Derives the parent state without depending on check ordering.
pub fn derive_parent_status(statuses: &[CheckStatus]) -> Result<AnalysisStatus, DomainError> {
    if statuses.is_empty() {
        return Err(DomainError::EmptyChecks);
    }
    if statuses.contains(&CheckStatus::Running) {
        return Ok(AnalysisStatus::Running);
    }
    if statuses.contains(&CheckStatus::Queued) {
        return Ok(AnalysisStatus::Queued);
    }
    if statuses
        .iter()
        .all(|status| *status == CheckStatus::Succeeded)
    {
        return Ok(AnalysisStatus::Succeeded);
    }
    if statuses.iter().all(|status| *status == CheckStatus::Failed) {
        return Ok(AnalysisStatus::Failed);
    }
    Ok(AnalysisStatus::Partial)
}

/// Pangram 4 bills text detection in started 100-word units.
pub const TEXT_BILLING_UNIT_WORDS: u64 = 100;

/// Returns the Pangram 4 text billable units for a word count: one unit per
/// started 100-word block, with a minimum of one even for empty input.
///
/// The saturating ceiling division cannot overflow: `word_count` plus the
/// 99-word offset reaches `u64::MAX` without wrapping, and the minimum unit
/// keeps the result at one when the quotient is zero. The analysis module
/// uses this for single-text preflight and `--max-billable-units`
/// validation; bulk billing follows Pangram's still-undocumented Pangram 4
/// bulk rule and MUST NOT reuse this per-text formula.
#[must_use]
pub const fn text_billable_units(word_count: u64) -> u64 {
    let started_blocks =
        word_count.saturating_add(TEXT_BILLING_UNIT_WORDS - 1) / TEXT_BILLING_UNIT_WORDS;
    if started_blocks == 0 {
        1
    } else {
        started_blocks
    }
}

/// Supplies the discriminator used by [`OrderedChecks`].
pub trait OrderedCheck {
    fn check_kind(&self) -> CheckKind;
}

impl OrderedCheck for CheckKind {
    fn check_kind(&self) -> CheckKind {
        *self
    }
}

/// One or two unique checks in canonical display and serialization order.
#[derive(Clone, Debug, JsonSchema, PartialEq, Eq)]
#[schemars(transparent)]
pub struct OrderedChecks<T = CheckKind>(#[schemars(length(min = 1, max = 2))] Vec<T>);

impl<T: OrderedCheck> OrderedChecks<T> {
    pub fn new(checks: impl IntoIterator<Item = T>) -> Result<Self, DomainError> {
        let checks: Vec<_> = checks.into_iter().collect();
        match checks.as_slice() {
            [] => Err(DomainError::EmptyChecks),
            [_, _, ..] if checks.len() > 2 => Err(DomainError::TooManyChecks),
            [first, second] if first.check_kind() == second.check_kind() => {
                Err(DomainError::DuplicateCheck)
            }
            [first, second]
                if first.check_kind() != CheckKind::AiDetection
                    || second.check_kind() != CheckKind::Plagiarism =>
            {
                Err(DomainError::InvalidCheckOrder)
            }
            _ => Ok(Self(checks)),
        }
    }

    #[must_use]
    pub fn as_slice(&self) -> &[T] {
        &self.0
    }

    #[must_use]
    pub fn into_vec(self) -> Vec<T> {
        self.0
    }
}

impl<T> Deref for OrderedChecks<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: Serialize> Serialize for OrderedChecks<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de, T> Deserialize<'de> for OrderedChecks<T>
where
    T: Deserialize<'de> + OrderedCheck,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(Vec::<T>::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}
