//! Frozen binding conflict contract tests.

use tracepress_core::{
    BindingCandidate, BindingConflictField, BindingId, BindingKey, BindingRepresentation,
    BindingVersions, ContentId, ContextBinding, ContextBindingError, SessionId, UuidV7Generator,
    UuidV7Timestamp,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn frozen_binding_rejects_key_conflict() -> TestResult {
    // Given
    let fixture = BindingFixture::new()?;
    let binding = fixture.binding();
    let candidate = BindingCandidate::new(
        BindingKey::new(
            fixture.session,
            ContentId::from_bytes(b"other logical content"),
        ),
        fixture.representation(),
    );

    // When
    let result = binding.ensure_compatible(&candidate);

    // Then
    assert_eq!(
        result,
        Err(ContextBindingError::KeyMismatch {
            binding_id: fixture.binding,
        })
    );
    Ok(())
}

#[test]
fn frozen_binding_rejects_raw_content_conflict() -> TestResult {
    // Given
    let fixture = BindingFixture::new()?;
    let binding = fixture.binding();
    let candidate = BindingCandidate::new(
        fixture.key(),
        BindingRepresentation::new(
            ContentId::from_bytes(b"other raw content"),
            fixture.rendered_content,
            BindingFixture::versions(),
        ),
    );

    // When
    let result = binding.ensure_compatible(&candidate);

    // Then
    assert_eq!(
        result,
        Err(ContextBindingError::FrozenConflict {
            binding_id: fixture.binding,
            field: BindingConflictField::RawContent,
        })
    );
    Ok(())
}

#[test]
fn frozen_binding_rejects_rendered_content_conflict() -> TestResult {
    // Given
    let fixture = BindingFixture::new()?;
    let binding = fixture.binding();
    let candidate = BindingCandidate::new(
        fixture.key(),
        BindingRepresentation::new(
            fixture.raw_content,
            ContentId::from_bytes(b"other rendered content"),
            BindingFixture::versions(),
        ),
    );

    // When
    let result = binding.ensure_compatible(&candidate);

    // Then
    assert_eq!(
        result,
        Err(ContextBindingError::FrozenConflict {
            binding_id: fixture.binding,
            field: BindingConflictField::RenderedContent,
        })
    );
    Ok(())
}

#[test]
fn frozen_binding_rejects_compressor_version_conflict() -> TestResult {
    // Given
    let fixture = BindingFixture::new()?;
    let binding = fixture.binding();
    let candidate = BindingCandidate::new(
        fixture.key(),
        BindingRepresentation::new(
            fixture.raw_content,
            fixture.rendered_content,
            BindingVersions::new("compressor-v2", "policy-v1"),
        ),
    );

    // When
    let result = binding.ensure_compatible(&candidate);

    // Then
    assert_eq!(
        result,
        Err(ContextBindingError::FrozenConflict {
            binding_id: fixture.binding,
            field: BindingConflictField::CompressorVersion,
        })
    );
    Ok(())
}

#[test]
fn frozen_binding_rejects_policy_version_conflict() -> TestResult {
    // Given
    let fixture = BindingFixture::new()?;
    let binding = fixture.binding();
    let candidate = BindingCandidate::new(
        fixture.key(),
        BindingRepresentation::new(
            fixture.raw_content,
            fixture.rendered_content,
            BindingVersions::new("compressor-v1", "policy-v2"),
        ),
    );

    // When
    let result = binding.ensure_compatible(&candidate);

    // Then
    assert_eq!(
        result,
        Err(ContextBindingError::FrozenConflict {
            binding_id: fixture.binding,
            field: BindingConflictField::PolicyVersion,
        })
    );
    Ok(())
}

struct BindingFixture {
    binding: BindingId,
    session: SessionId,
    logical_content: ContentId,
    raw_content: ContentId,
    rendered_content: ContentId,
}

impl BindingFixture {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let generator = UuidV7Generator::new();
        let timestamp = UuidV7Timestamp::new(1_704_067_200, 0)?;
        Ok(Self {
            binding: BindingId::generate_at(&generator, timestamp),
            session: SessionId::generate_at(&generator, timestamp),
            logical_content: ContentId::from_bytes(b"logical content"),
            raw_content: ContentId::from_bytes(b"raw content"),
            rendered_content: ContentId::from_bytes(b"rendered content"),
        })
    }

    const fn key(&self) -> BindingKey {
        BindingKey::new(self.session, self.logical_content)
    }

    fn versions() -> BindingVersions {
        BindingVersions::new("compressor-v1", "policy-v1")
    }

    fn representation(&self) -> BindingRepresentation {
        BindingRepresentation::new(self.raw_content, self.rendered_content, Self::versions())
    }

    fn candidate(&self) -> BindingCandidate {
        BindingCandidate::new(self.key(), self.representation())
    }

    fn binding(&self) -> ContextBinding {
        ContextBinding::freeze(self.binding, self.candidate())
    }
}
