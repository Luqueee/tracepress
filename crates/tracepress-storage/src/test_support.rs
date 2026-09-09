#![allow(
    clippy::redundant_pub_crate,
    reason = "sibling test modules share this fallible test result"
)]

pub(crate) type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
