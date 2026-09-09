use std::path::{Path, PathBuf};
use std::time::SystemTime;

use tracepress_core::ContentId;

use super::database::GcProtection;
use super::filesystem::{Encoding, FilesystemCasBlobStore, OBJECTS_DIRECTORY};
use super::retrieval::content_id_from_path;
use super::{BlobError, HybridBlobStore};

/// One reported orphan-collection decision.
#[allow(
    clippy::exhaustive_enums,
    reason = "daemon consumers must account for each safety decision"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GcDecision {
    /// An old validated orphan was removed.
    Removed(ContentId),
    /// A durable pin protected the object.
    RetainedPinned(ContentId),
    /// A recovery mapping protected the object.
    RetainedRecoverable(ContentId),
    /// A database row protected the object.
    RetainedReferenced(ContentId),
    /// The orphan is newer than the supplied cutoff.
    RetainedYoung(ContentId),
    /// Object identity could not be proven, so it was preserved.
    RetainedUncertain(ContentId),
}

/// Complete decisions from one daemon-owned collection pass.
#[derive(Debug, Default)]
pub struct GcReport {
    decisions: Vec<GcDecision>,
}

impl GcReport {
    /// Returns each decision in deterministic path order.
    #[must_use]
    pub fn decisions(&self) -> &[GcDecision] {
        &self.decisions
    }
}

/// Borrowed capability proving collection is initiated by the store owner.
#[derive(Debug)]
pub struct DaemonGc<'store> {
    store: &'store HybridBlobStore,
}

impl<'store> DaemonGc<'store> {
    pub(crate) const fn new(store: &'store HybridBlobStore) -> Self {
        Self { store }
    }

    /// Collects validated unreferenced objects older than `cutoff`.
    ///
    /// # Errors
    /// Returns a typed database or filesystem error when protection cannot be checked or scanning
    /// cannot complete safely.
    pub async fn collect_orphans(self, cutoff: SystemTime) -> Result<GcReport, BlobError> {
        collect(self.store, cutoff).await
    }
}

async fn collect(store: &HybridBlobStore, cutoff: SystemTime) -> Result<GcReport, BlobError> {
    let _guard = store.cas_guard().lock().await;
    let filesystem = store.filesystem().clone();
    let candidates = tokio::task::spawn_blocking(move || candidates(&filesystem))
        .await
        .map_err(|source| BlobError::Database(crate::StorageError::ShutdownJoin(source)))??;
    let mut decisions = Vec::new();
    decisions
        .try_reserve_exact(candidates.len())
        .map_err(|_error| BlobError::Allocation {
            requested: candidates.len(),
        })?;

    for candidate in candidates {
        let content_id = candidate.content_id;
        let protection = store.writer().blob_protection(content_id).await?;
        let decision = match protection {
            GcProtection::Pinned => GcDecision::RetainedPinned(content_id),
            GcProtection::Recoverable => GcDecision::RetainedRecoverable(content_id),
            GcProtection::Referenced => GcDecision::RetainedReferenced(content_id),
            GcProtection::Orphan => classify_orphan(store.filesystem(), candidate, cutoff).await?,
        };
        decisions.push(decision);
    }

    Ok(GcReport { decisions })
}

async fn classify_orphan(
    filesystem: &FilesystemCasBlobStore,
    candidate: Candidate,
    cutoff: SystemTime,
) -> Result<GcDecision, BlobError> {
    let backend = filesystem.clone();
    tokio::task::spawn_blocking(move || {
        let metadata = std::fs::symlink_metadata(&candidate.path)
            .map_err(|source| BlobError::io("inspect CAS orphan", &candidate.path, source))?;
        let modified = metadata.modified().map_err(|source| {
            BlobError::io("read CAS orphan timestamp", &candidate.path, source)
        })?;
        if modified > cutoff {
            return Ok(GcDecision::RetainedYoung(candidate.content_id));
        }
        if backend
            .read_path_blocking(&candidate.path, candidate.encoding, None)
            .is_err()
        {
            return Ok(GcDecision::RetainedUncertain(candidate.content_id));
        }
        std::fs::remove_file(&candidate.path)
            .map_err(|source| BlobError::io("remove CAS orphan", &candidate.path, source))?;
        Ok(GcDecision::Removed(candidate.content_id))
    })
    .await
    .map_err(|source| BlobError::Database(crate::StorageError::ShutdownJoin(source)))?
}

#[derive(Debug)]
struct Candidate {
    path: PathBuf,
    content_id: ContentId,
    encoding: Encoding,
}

fn candidates(filesystem: &FilesystemCasBlobStore) -> Result<Vec<Candidate>, BlobError> {
    let objects = filesystem.root.join(OBJECTS_DIRECTORY);
    let mut paths = Vec::new();
    collect_paths(&objects, &mut paths)?;
    paths.sort_unstable();
    paths
        .into_iter()
        .filter_map(|path| candidate(path).transpose())
        .collect()
}

fn collect_paths(directory: &Path, paths: &mut Vec<PathBuf>) -> Result<(), BlobError> {
    let entries = std::fs::read_dir(directory)
        .map_err(|source| BlobError::io("scan CAS directory", directory, source))?;
    for entry in entries {
        let entry =
            entry.map_err(|source| BlobError::io("read CAS directory entry", directory, source))?;
        let file_type = entry.file_type().map_err(|source| {
            BlobError::io("inspect CAS directory entry", &entry.path(), source)
        })?;
        if file_type.is_dir() {
            collect_paths(&entry.path(), paths)?;
        } else if file_type.is_file() {
            paths
                .try_reserve(1)
                .map_err(|_error| BlobError::Allocation {
                    requested: paths.len().saturating_add(1),
                })?;
            paths.push(entry.path());
        }
    }
    Ok(())
}

fn candidate(path: PathBuf) -> Result<Option<Candidate>, BlobError> {
    let encoding = match path.extension().and_then(std::ffi::OsStr::to_str) {
        Some("blob") => Encoding::Raw,
        Some("zst") => Encoding::Zstd,
        _ => return Ok(None),
    };
    let content_id = content_id_from_path(&path)?;
    Ok(Some(Candidate {
        path,
        content_id,
        encoding,
    }))
}
