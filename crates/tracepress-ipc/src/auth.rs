use std::fmt;

use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

#[allow(
    clippy::redundant_pub_crate,
    reason = "the private auth module shares this wire width with its sibling transport module"
)]
pub(crate) const CREDENTIAL_BYTES: usize = 32;

/// Opaque authentication material for one local IPC endpoint.
#[derive(Clone)]
pub struct Credential(Zeroizing<[u8; CREDENTIAL_BYTES]>);

impl Credential {
    /// Wraps exact credential bytes without exposing them through formatting.
    #[must_use]
    pub fn new(bytes: [u8; CREDENTIAL_BYTES]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    pub(crate) fn bytes(&self) -> &[u8; CREDENTIAL_BYTES] {
        &self.0
    }
}

impl fmt::Debug for Credential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Credential([REDACTED])")
    }
}

impl PartialEq for Credential {
    fn eq(&self, other: &Self) -> bool {
        bool::from(self.bytes().ct_eq(other.bytes()))
    }
}

impl Eq for Credential {}
