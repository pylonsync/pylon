//! Pluggable file storage abstraction.
//!
//! Provides a trait for file storage operations that can be implemented
//! for local disk, S3, R2, GCS, or any other storage backend.

use serde::Serialize;

// ---------------------------------------------------------------------------
// File storage trait
// ---------------------------------------------------------------------------

/// Pluggable file storage backend.
pub trait FileStorage: Send + Sync {
    /// Store file content, returning a file ID and public URL.
    fn store(&self, name: &str, content: &[u8], content_type: &str)
        -> Result<StoredFile, FileStorageError>;

    /// Retrieve file content by ID.
    fn get(&self, id: &str) -> Result<Vec<u8>, FileStorageError>;

    /// Delete a file by ID.
    fn delete(&self, id: &str) -> Result<bool, FileStorageError>;

    /// Generate a presigned upload URL (for direct client uploads).
    /// Not all backends support this — returns None if unsupported.
    fn presigned_upload_url(
        &self,
        _name: &str,
        _content_type: &str,
        _expires_secs: u64,
    ) -> Result<Option<String>, FileStorageError> {
        Ok(None)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StoredFile {
    pub id: String,
    pub url: String,
    pub size: usize,
}

#[derive(Debug, Clone)]
pub struct FileStorageError {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for FileStorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for FileStorageError {}

// ---------------------------------------------------------------------------
// Local filesystem implementation
// ---------------------------------------------------------------------------

/// File storage backed by a local directory.
pub struct LocalFileStorage {
    dir: std::path::PathBuf,
    url_prefix: String,
}

impl LocalFileStorage {
    pub fn new(dir: &str, url_prefix: &str) -> Self {
        let path = std::path::PathBuf::from(dir);
        let _ = std::fs::create_dir_all(&path);
        Self {
            dir: path,
            url_prefix: url_prefix.to_string(),
        }
    }
}

impl FileStorage for LocalFileStorage {
    fn store(
        &self,
        name: &str,
        content: &[u8],
        _content_type: &str,
    ) -> Result<StoredFile, FileStorageError> {
        let id = format!(
            "file_{}_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            name.replace(['/', '\\', '.'], "_")
        );
        let path = self.dir.join(&id);
        std::fs::write(&path, content).map_err(|e| FileStorageError {
            code: "WRITE_FAILED".into(),
            message: format!("Failed to write file: {e}"),
        })?;

        Ok(StoredFile {
            url: format!("{}/{}", self.url_prefix, id),
            size: content.len(),
            id,
        })
    }

    fn get(&self, id: &str) -> Result<Vec<u8>, FileStorageError> {
        if id.contains("..") || id.contains('/') || id.contains('\\') {
            return Err(FileStorageError {
                code: "INVALID_ID".into(),
                message: "Invalid file ID".into(),
            });
        }
        let path = self.dir.join(id);
        std::fs::read(&path).map_err(|_| FileStorageError {
            code: "NOT_FOUND".into(),
            message: "File not found".into(),
        })
    }

    fn delete(&self, id: &str) -> Result<bool, FileStorageError> {
        if id.contains("..") || id.contains('/') || id.contains('\\') {
            return Err(FileStorageError {
                code: "INVALID_ID".into(),
                message: "Invalid file ID".into(),
            });
        }
        let path = self.dir.join(id);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(FileStorageError {
                code: "DELETE_FAILED".into(),
                message: format!("Failed to delete file: {e}"),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// S3-compatible storage (stub — needs an HTTP client at runtime)
// ---------------------------------------------------------------------------

/// Configuration for S3-compatible storage (S3, R2, GCS, MinIO).
#[derive(Debug, Clone)]
pub struct S3Config {
    pub bucket: String,
    pub region: String,
    pub endpoint: Option<String>,
    pub access_key: String,
    pub secret_key: String,
    pub public_url_prefix: Option<String>,
}

impl S3Config {
    /// Create from environment variables.
    ///
    /// Reads: STATECRAFT_S3_BUCKET, STATECRAFT_S3_REGION, STATECRAFT_S3_ENDPOINT,
    /// STATECRAFT_S3_ACCESS_KEY, STATECRAFT_S3_SECRET_KEY, STATECRAFT_S3_PUBLIC_URL
    pub fn from_env() -> Option<Self> {
        Some(Self {
            bucket: std::env::var("STATECRAFT_S3_BUCKET").ok()?,
            region: std::env::var("STATECRAFT_S3_REGION").unwrap_or_else(|_| "us-east-1".into()),
            endpoint: std::env::var("STATECRAFT_S3_ENDPOINT").ok(),
            access_key: std::env::var("STATECRAFT_S3_ACCESS_KEY").ok()?,
            secret_key: std::env::var("STATECRAFT_S3_SECRET_KEY").ok()?,
            public_url_prefix: std::env::var("STATECRAFT_S3_PUBLIC_URL").ok(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_store_and_get() {
        let dir = std::env::temp_dir().join(format!("statecraft_files_{}", std::process::id()));
        let storage = LocalFileStorage::new(dir.to_str().unwrap(), "/api/files");

        let stored = storage.store("test.txt", b"hello world", "text/plain").unwrap();
        assert_eq!(stored.size, 11);
        assert!(stored.url.starts_with("/api/files/"));

        let content = storage.get(&stored.id).unwrap();
        assert_eq!(content, b"hello world");

        let deleted = storage.delete(&stored.id).unwrap();
        assert!(deleted);

        let not_found = storage.get(&stored.id);
        assert!(not_found.is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_rejects_traversal() {
        let dir = std::env::temp_dir().join(format!("statecraft_files2_{}", std::process::id()));
        let storage = LocalFileStorage::new(dir.to_str().unwrap(), "/api/files");

        assert!(storage.get("../etc/passwd").is_err());
        assert!(storage.delete("../etc/passwd").is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
