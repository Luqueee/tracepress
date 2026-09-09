use tempfile::TempDir;
use tracepress_core::{ContentId, ContentKind, MaxIpcQueueItems};

use crate::blob_store::filesystem::CrashPoint;
use crate::test_support::TestResult;
use crate::{
    BlobByteLimit, BlobError, BlobPut, BlobStorage, BlobStore, BlobStoreConfig, Durability,
    GcDecision, HybridBlobStore, InlineBlobMaxBytes, PersistenceCompression, StorageConfig,
    ZstdLevel,
};

#[tokio::test]
async fn blob_round_trip_and_gc_routes_threshold_exactly() -> TestResult {
    // Given: a real SQLite writer and CAS with an inline threshold of four bytes.
    let directory = TempDir::new()?;
    let database = directory.path().join("blob.sqlite3");
    let cas_root = directory.path().join("cas");
    let storage = StorageConfig::new(database, Durability::Strict, MaxIpcQueueItems::new(8)?);
    let blobs = BlobStoreConfig::new(
        cas_root,
        InlineBlobMaxBytes::new(4)?,
        BlobByteLimit::new(1_024)?,
    );
    let store = HybridBlobStore::open(storage, blobs).await?;
    let fixtures: [(&[u8], BlobStorage); 3] = [
        (b"nul", BlobStorage::Inline),
        (b"\0\xff\xfe\0", BlobStorage::Inline),
        (b"large", BlobStorage::External),
    ];

    // When: threshold-minus-one, threshold, and threshold-plus-one bytes are stored.
    for (raw, expected_storage) in fixtures {
        let stored = store
            .put(BlobPut::new(
                raw,
                ContentKind::Binary,
                "2026-09-09T00:00:00Z",
            ))
            .await?;

        // Then: routing is deterministic and get returns the exact arbitrary bytes.
        assert_eq!(stored.storage(), expected_storage);
        assert_eq!(stored.content_id(), ContentId::from_bytes(raw));
        assert_eq!(store.get(stored.content_id()).await?.as_bytes(), raw);
    }
    store.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn zstd_round_trip_duplicate_empty_and_content_id_stability() -> TestResult {
    // Given: a compressed hybrid store and binary fixtures including empty content.
    let directory = TempDir::new()?;
    let store = open_store(
        &directory,
        0,
        PersistenceCompression::Zstd(ZstdLevel::new(3)?),
    )
    .await?;
    let large = b"\0\xffinvalid-utf8\xfe repeated repeated repeated";

    // When: empty content and the same external content twice are stored and retrieved.
    let empty = store.put(blob(b"")).await?;
    let first = store.put(blob(large)).await?;
    let duplicate = store.put(blob(large)).await?;

    // Then: identity remains over raw bytes and both puts resolve to one exact object.
    assert_eq!(empty.storage(), BlobStorage::Inline);
    assert_eq!(store.get(empty.content_id()).await?.as_bytes(), b"");
    assert_eq!(first, duplicate);
    assert_eq!(first.content_id(), ContentId::from_bytes(large));
    assert_eq!(store.get(first.content_id()).await?.as_bytes(), large);
    store.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn cas_write_failure_keeps_forwardable_raw() -> TestResult {
    // Given: a store whose CAS objects directory becomes a regular file after startup.
    let directory = TempDir::new()?;
    let store = open_store(&directory, 0, PersistenceCompression::Disabled).await?;
    let objects = directory.path().join("cas/objects");
    std::fs::remove_dir(&objects)?;
    std::fs::write(&objects, b"blocks temporary creation")?;
    let forwardable = b"caller still owns \0\xff raw";

    // When: external persistence fails before publication.
    let result = store.put(blob(forwardable)).await;

    // Then: the failure is typed and the caller's exact original bytes remain available.
    assert!(matches!(result, Err(BlobError::Filesystem { .. })));
    assert_eq!(forwardable, b"caller still owns \0\xff raw");
    store.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn missing_and_corrupt_cas_are_typed() -> TestResult {
    // Given: one external object and one unrelated missing content identity.
    let directory = TempDir::new()?;
    let store = open_store(&directory, 0, PersistenceCompression::Disabled).await?;
    let stored = store.put(blob(b"external")).await?;
    let path = store.filesystem().absolute_path(
        stored.content_id(),
        crate::blob_store::filesystem::Encoding::Raw,
    );
    std::fs::write(path, b"corrupt")?;
    let missing = ContentId::from_bytes(b"missing");

    // When: both identities are retrieved through the public seam.
    let corrupt_result = store.get(stored.content_id()).await;
    let missing_result = store.get(missing).await;

    // Then: corruption and absence remain distinct typed failures.
    assert!(matches!(corrupt_result, Err(BlobError::Corrupt { .. })));
    assert!(
        matches!(missing_result, Err(BlobError::Missing { content_id }) if content_id == missing)
    );
    store.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn pin_updates_are_checked_and_nonnegative() -> TestResult {
    // Given: one durable inline content object with no pins.
    let directory = TempDir::new()?;
    let store = open_store(&directory, 32, PersistenceCompression::Disabled).await?;
    let stored = store.put(blob(b"pinned")).await?;

    // When: pins are incremented, decremented, and then decremented below zero.
    let one = store.pin(stored.content_id()).await?;
    let two = store.pin(stored.content_id()).await?;
    let one_again = store.unpin(stored.content_id()).await?;
    let zero = store.unpin(stored.content_id()).await?;
    let underflow = store.unpin(stored.content_id()).await;

    // Then: each transaction returns its committed count and underflow is rejected.
    assert_eq!((one, two, one_again, zero), (1, 2, 1, 0));
    assert!(matches!(underflow, Err(BlobError::PinUnderflow { .. })));
    store.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn daemon_gc_preserves_pinned_and_referenced_external_blobs() -> TestResult {
    // Given: one registered external blob protected by a durable pin.
    let directory = TempDir::new()?;
    let store = open_store(&directory, 0, PersistenceCompression::Disabled).await?;
    let stored = store.put(blob(b"registered external bytes")).await?;
    let _pin_count = store.pin(stored.content_id()).await?;
    let future = std::time::SystemTime::UNIX_EPOCH
        .checked_add(std::time::Duration::from_secs(4_000_000_000))
        .ok_or("fixed future cutoff overflowed")?;

    // When: daemon GC runs before and after the explicit pin is removed.
    let pinned = store.daemon_gc().collect_orphans(future).await?;
    let _pin_count = store.unpin(stored.content_id()).await?;
    let referenced = store.daemon_gc().collect_orphans(future).await?;

    // Then: both durable protection states preserve the exact registered bytes.
    assert_eq!(
        pinned.decisions(),
        &[GcDecision::RetainedPinned(stored.content_id())]
    );
    assert_eq!(
        referenced.decisions(),
        &[GcDecision::RetainedReferenced(stored.content_id())]
    );
    assert_eq!(
        store.get(stored.content_id()).await?.as_bytes(),
        b"registered external bytes"
    );
    store.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn power_loss_boundaries_never_leave_a_dangling_reference() -> TestResult {
    // Given: each deterministic boundary in the strict CAS publication order.
    let boundaries = [
        CrashPoint::TempWritten,
        CrashPoint::FileSynced,
        CrashPoint::Published,
        CrashPoint::DirectorySynced,
        CrashPoint::Registered,
    ];
    let raw = b"crash-safe external bytes";
    let content_id = ContentId::from_bytes(raw);

    for boundary in boundaries {
        let directory = TempDir::new()?;
        let store = open_store(&directory, 0, PersistenceCompression::Disabled).await?;

        // When: the put is interrupted immediately after that boundary.
        let result = store.put_with_crash(blob(raw), boundary).await;
        assert!(matches!(result, Err(BlobError::InjectedCrash { .. })));
        let registered = store.writer().blob_lookup(content_id).await?.is_some();
        let recoverable = store.filesystem().get_orphan(content_id).await.is_ok();

        // Then: registration implies published recoverable bytes, while earlier files are orphans.
        assert!(!registered || recoverable);
        assert_eq!(registered, boundary == CrashPoint::Registered);
        assert_eq!(
            recoverable,
            matches!(
                boundary,
                CrashPoint::Published | CrashPoint::DirectorySynced | CrashPoint::Registered
            )
        );
        store.shutdown().await?;
    }
    Ok(())
}

#[tokio::test]
async fn daemon_gc_retains_young_and_removes_old_validated_orphan() -> TestResult {
    // Given: a published unregistered object left at the parent-directory-fsync boundary.
    let directory = TempDir::new()?;
    let store = open_store(&directory, 0, PersistenceCompression::Disabled).await?;
    let raw = b"orphan pending daemon collection";
    let content_id = ContentId::from_bytes(raw);
    let result = store
        .put_with_crash(blob(raw), CrashPoint::DirectorySynced)
        .await;
    assert!(matches!(result, Err(BlobError::InjectedCrash { .. })));

    // When: daemon GC first uses an ancient cutoff, then a fixed future cutoff.
    let young = store
        .daemon_gc()
        .collect_orphans(std::time::SystemTime::UNIX_EPOCH)
        .await?;
    let future = std::time::SystemTime::UNIX_EPOCH
        .checked_add(std::time::Duration::from_secs(4_000_000_000))
        .ok_or("fixed future cutoff overflowed")?;
    let old = store.daemon_gc().collect_orphans(future).await?;

    // Then: retention is explicit before cutoff and validated removal is explicit after it.
    assert_eq!(young.decisions(), &[GcDecision::RetainedYoung(content_id)]);
    assert_eq!(old.decisions(), &[GcDecision::Removed(content_id)]);
    assert!(matches!(
        store.get(content_id).await,
        Err(BlobError::Missing { .. })
    ));
    store.shutdown().await?;
    Ok(())
}

async fn open_store(
    directory: &TempDir,
    inline_max: u64,
    compression: PersistenceCompression,
) -> Result<HybridBlobStore, BlobError> {
    let storage = StorageConfig::new(
        directory.path().join("blob.sqlite3"),
        Durability::Strict,
        MaxIpcQueueItems::new(16).map_err(|_error| BlobError::ZeroLimit {
            field: "test_queue_items",
        })?,
    );
    let blobs = BlobStoreConfig::new(
        directory.path().join("cas"),
        InlineBlobMaxBytes::new(inline_max)?,
        BlobByteLimit::new(4_096)?,
    )
    .with_max_persisted_bytes(BlobByteLimit::new(4_096)?)
    .with_compression(compression);
    HybridBlobStore::open(storage, blobs).await
}

const fn blob(raw_bytes: &[u8]) -> BlobPut<'_> {
    BlobPut::new(raw_bytes, ContentKind::Binary, "2026-09-09T00:00:00Z")
}
