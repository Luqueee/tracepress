use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use thiserror::Error;

use crate::{BindingId, ContentId, SessionId};

/// The per-session logical identity on which a frozen binding is unique.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BindingKey {
    session_id: SessionId,
    logical_content_id: ContentId,
}

impl BindingKey {
    /// Creates a session-scoped logical content key.
    #[must_use]
    pub const fn new(session_id: SessionId, logical_content_id: ContentId) -> Self {
        Self {
            session_id,
            logical_content_id,
        }
    }

    /// Returns the session in which the logical content is bound.
    #[must_use]
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// Returns the session-local logical content identity.
    #[must_use]
    pub const fn logical_content_id(&self) -> ContentId {
        self.logical_content_id
    }
}

/// Version labels that selected a rendered representation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BindingVersions {
    compressor_version: String,
    policy_version: String,
}

impl BindingVersions {
    /// Creates the exact compressor and policy version pair.
    #[must_use]
    pub fn new(compressor_version: impl Into<String>, policy_version: impl Into<String>) -> Self {
        Self {
            compressor_version: compressor_version.into(),
            policy_version: policy_version.into(),
        }
    }

    /// Returns the exact compressor version label.
    #[must_use]
    pub fn compressor_version(&self) -> &str {
        &self.compressor_version
    }

    /// Returns the exact policy version label.
    #[must_use]
    pub fn policy_version(&self) -> &str {
        &self.policy_version
    }
}

/// Raw and rendered identities plus the versions that chose the rendering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BindingRepresentation {
    raw_content_id: ContentId,
    rendered_content_id: ContentId,
    versions: BindingVersions,
}

impl BindingRepresentation {
    /// Creates a rendered content representation.
    #[must_use]
    pub const fn new(
        raw_content_id: ContentId,
        rendered_content_id: ContentId,
        versions: BindingVersions,
    ) -> Self {
        Self {
            raw_content_id,
            rendered_content_id,
            versions,
        }
    }

    /// Returns the original raw content identity.
    #[must_use]
    pub const fn raw_content_id(&self) -> ContentId {
        self.raw_content_id
    }

    /// Returns the selected rendered content identity.
    #[must_use]
    pub const fn rendered_content_id(&self) -> ContentId {
        self.rendered_content_id
    }

    /// Returns the versions that selected this representation.
    #[must_use]
    pub const fn versions(&self) -> &BindingVersions {
        &self.versions
    }
}

/// A proposed binding before it is frozen for a session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BindingCandidate {
    key: BindingKey,
    representation: BindingRepresentation,
}

impl BindingCandidate {
    /// Creates a proposed session-scoped representation.
    #[must_use]
    pub const fn new(key: BindingKey, representation: BindingRepresentation) -> Self {
        Self {
            key,
            representation,
        }
    }

    /// Returns the session-scoped logical key.
    #[must_use]
    pub const fn key(&self) -> &BindingKey {
        &self.key
    }

    /// Returns the proposed immutable representation.
    #[must_use]
    pub const fn representation(&self) -> &BindingRepresentation {
        &self.representation
    }
}

/// A representation selected once for a session and never replaceable.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContextBinding {
    binding_id: BindingId,
    session_id: SessionId,
    logical_content_id: ContentId,
    raw_content_id: ContentId,
    rendered_content_id: ContentId,
    compressor_version: String,
    policy_version: String,
    frozen: FrozenMarker,
}

impl ContextBinding {
    /// Freezes a candidate as the only representation for its session and logical content.
    #[must_use]
    pub fn freeze(binding_id: BindingId, candidate: BindingCandidate) -> Self {
        Self {
            binding_id,
            session_id: candidate.key.session_id,
            logical_content_id: candidate.key.logical_content_id,
            raw_content_id: candidate.representation.raw_content_id,
            rendered_content_id: candidate.representation.rendered_content_id,
            compressor_version: candidate.representation.versions.compressor_version,
            policy_version: candidate.representation.versions.policy_version,
            frozen: FrozenMarker,
        }
    }

    /// Returns the frozen binding identity.
    #[must_use]
    pub const fn binding_id(&self) -> BindingId {
        self.binding_id
    }

    /// Returns the session in which this binding is frozen.
    #[must_use]
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// Returns the session-local logical content identity.
    #[must_use]
    pub const fn logical_content_id(&self) -> ContentId {
        self.logical_content_id
    }

    /// Returns the original raw content identity.
    #[must_use]
    pub const fn raw_content_id(&self) -> ContentId {
        self.raw_content_id
    }

    /// Returns the selected rendered content identity.
    #[must_use]
    pub const fn rendered_content_id(&self) -> ContentId {
        self.rendered_content_id
    }

    /// Returns the exact compressor version label.
    #[must_use]
    pub fn compressor_version(&self) -> &str {
        &self.compressor_version
    }

    /// Returns the exact policy version label.
    #[must_use]
    pub fn policy_version(&self) -> &str {
        &self.policy_version
    }

    /// Accepts the identical candidate and rejects any attempted replacement.
    ///
    /// # Errors
    /// Returns [`ContextBindingError`] when the key differs or a frozen field would change.
    pub fn ensure_compatible(
        &self,
        candidate: &BindingCandidate,
    ) -> Result<(), ContextBindingError> {
        if self.session_id != candidate.key.session_id
            || self.logical_content_id != candidate.key.logical_content_id
        {
            return Err(ContextBindingError::KeyMismatch {
                binding_id: self.binding_id,
            });
        }
        if self.raw_content_id != candidate.representation.raw_content_id {
            return Err(self.conflict(BindingConflictField::RawContent));
        }
        if self.rendered_content_id != candidate.representation.rendered_content_id {
            return Err(self.conflict(BindingConflictField::RenderedContent));
        }
        if self.compressor_version != candidate.representation.versions.compressor_version {
            return Err(self.conflict(BindingConflictField::CompressorVersion));
        }
        if self.policy_version != candidate.representation.versions.policy_version {
            return Err(self.conflict(BindingConflictField::PolicyVersion));
        }
        Ok(())
    }

    const fn conflict(&self, field: BindingConflictField) -> ContextBindingError {
        ContextBindingError::FrozenConflict {
            binding_id: self.binding_id,
            field,
        }
    }
}

/// The frozen field a conflicting candidate attempted to replace.
#[allow(
    clippy::exhaustive_enums,
    reason = "callers must handle every binding conflict source explicitly"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BindingConflictField {
    /// The original raw content identity.
    RawContent,
    /// The rendered content identity.
    RenderedContent,
    /// The compressor version.
    CompressorVersion,
    /// The policy version.
    PolicyVersion,
}

/// Failure to apply a candidate to an existing frozen binding.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum ContextBindingError {
    /// The candidate did not address the binding's session-scoped logical content.
    #[error("candidate key does not match frozen binding {binding_id}")]
    KeyMismatch {
        /// The existing frozen binding.
        binding_id: BindingId,
    },
    /// The candidate attempted to replace one frozen representation field.
    #[error("candidate conflicts with {field:?} in frozen binding {binding_id}")]
    FrozenConflict {
        /// The existing frozen binding.
        binding_id: BindingId,
        /// The first conflicting field.
        field: BindingConflictField,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FrozenMarker;

impl Serialize for FrozenMarker {
    fn serialize<SerializerType>(
        &self,
        serializer: SerializerType,
    ) -> Result<SerializerType::Ok, SerializerType::Error>
    where
        SerializerType: Serializer,
    {
        serializer.serialize_bool(true)
    }
}

impl<'de> Deserialize<'de> for FrozenMarker {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: Deserializer<'de>,
    {
        if bool::deserialize(deserializer)? {
            Ok(Self)
        } else {
            Err(de::Error::custom("context binding must be frozen"))
        }
    }
}
