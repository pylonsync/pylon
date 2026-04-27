//! Pluggable file storage abstraction.
//!
//! Provides a trait for file storage operations that can be implemented
//! for local disk, S3, R2, GCS, or any other storage backend.

use serde::Serialize;
use std::io::Read;

// ---------------------------------------------------------------------------
// File storage trait
// ---------------------------------------------------------------------------

/// Pluggable file storage backend.
pub trait FileStorage: Send + Sync {
    /// Store file content, returning a file ID and public URL.
    fn store(
        &self,
        name: &str,
        content: &[u8],
        content_type: &str,
    ) -> Result<StoredFile, FileStorageError>;

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
    /// Reads: PYLON_S3_BUCKET, PYLON_S3_REGION, PYLON_S3_ENDPOINT,
    /// PYLON_S3_ACCESS_KEY, PYLON_S3_SECRET_KEY, PYLON_S3_PUBLIC_URL
    pub fn from_env() -> Option<Self> {
        Some(Self {
            bucket: std::env::var("PYLON_S3_BUCKET").ok()?,
            region: std::env::var("PYLON_S3_REGION").unwrap_or_else(|_| "us-east-1".into()),
            endpoint: std::env::var("PYLON_S3_ENDPOINT").ok(),
            access_key: std::env::var("PYLON_S3_ACCESS_KEY").ok()?,
            secret_key: std::env::var("PYLON_S3_SECRET_KEY").ok()?,
            public_url_prefix: std::env::var("PYLON_S3_PUBLIC_URL").ok(),
        })
    }
}

// ---------------------------------------------------------------------------
// Stack0 CDN/storage implementation
// ---------------------------------------------------------------------------

/// File storage backed by Stack0's CDN.
///
/// Uploads use a 3-step flow: POST `/cdn/upload` to mint a presigned URL,
/// PUT the bytes to that URL, then POST `/cdn/upload/{assetId}/confirm`.
/// `store()` returns the public `cdnUrl` so clients can embed it directly
/// without round-tripping through pylon.
pub struct Stack0FileStorage {
    api_key: String,
    /// Base API URL — typically `https://api.stack0.dev`. Configurable so
    /// tests can point at a local mock server.
    base_url: String,
    /// Optional folder/prefix for organizing uploads.
    folder: Option<String>,
}

impl Stack0FileStorage {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.stack0.dev".into(),
            folder: None,
        }
    }

    pub fn with_folder(mut self, folder: impl Into<String>) -> Self {
        self.folder = Some(folder.into());
        self
    }

    /// Override the API base URL (useful for tests or self-hosted Stack0).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Construct from environment variables.
    /// Reads: PYLON_STACK0_API_KEY (required), PYLON_STACK0_FOLDER (optional),
    /// PYLON_STACK0_BASE_URL (optional override).
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("PYLON_STACK0_API_KEY").ok()?;
        let mut s = Self::new(api_key);
        if let Ok(folder) = std::env::var("PYLON_STACK0_FOLDER") {
            s = s.with_folder(folder);
        }
        if let Ok(base) = std::env::var("PYLON_STACK0_BASE_URL") {
            s = s.with_base_url(base);
        }
        Some(s)
    }

    /// JSON body for the `/cdn/upload` init call. Pulled out so tests can
    /// pin the wire shape without exercising the network.
    pub fn build_upload_init_body(
        &self,
        filename: &str,
        content_type: &str,
        size: usize,
    ) -> serde_json::Value {
        let mut body = serde_json::json!({
            "filename": filename,
            "mimeType": content_type,
            "size": size,
        });
        if let Some(folder) = &self.folder {
            body["folder"] = serde_json::Value::String(folder.clone());
        }
        body
    }
}

fn stack0_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout_read(std::time::Duration::from_secs(30))
        .timeout_write(std::time::Duration::from_secs(30))
        .user_agent("pylon-storage/0.1")
        .build()
}

fn stack0_err(code: &str, e: impl std::fmt::Display) -> FileStorageError {
    FileStorageError {
        code: code.into(),
        message: e.to_string(),
    }
}

impl FileStorage for Stack0FileStorage {
    fn store(
        &self,
        name: &str,
        content: &[u8],
        content_type: &str,
    ) -> Result<StoredFile, FileStorageError> {
        let agent = stack0_agent();
        let init_body = self.build_upload_init_body(name, content_type, content.len());

        // 1. Mint presigned upload URL.
        let init_resp: serde_json::Value = agent
            .post(&format!("{}/cdn/upload", self.base_url))
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .set("Content-Type", "application/json")
            .send_string(&init_body.to_string())
            .map_err(|e| stack0_err("STACK0_UPLOAD_INIT_FAILED", e))?
            .into_json()
            .map_err(|e| stack0_err("STACK0_UPLOAD_INIT_PARSE", e))?;

        let upload_url = init_resp["uploadUrl"]
            .as_str()
            .ok_or_else(|| stack0_err("STACK0_UPLOAD_INIT_BAD_RESPONSE", "missing uploadUrl"))?;
        let asset_id = init_resp["assetId"]
            .as_str()
            .ok_or_else(|| stack0_err("STACK0_UPLOAD_INIT_BAD_RESPONSE", "missing assetId"))?
            .to_string();
        let cdn_url = init_resp["cdnUrl"]
            .as_str()
            .ok_or_else(|| stack0_err("STACK0_UPLOAD_INIT_BAD_RESPONSE", "missing cdnUrl"))?
            .to_string();

        // 2. PUT bytes to presigned URL. The presigned URL carries its own
        // signature, so we don't reattach the API key here.
        agent
            .put(upload_url)
            .set("Content-Type", content_type)
            .send_bytes(content)
            .map_err(|e| stack0_err("STACK0_UPLOAD_PUT_FAILED", e))?;

        // 3. Confirm upload so Stack0 marks the asset as ready.
        agent
            .post(&format!("{}/cdn/upload/{}/confirm", self.base_url, asset_id))
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .call()
            .map_err(|e| stack0_err("STACK0_UPLOAD_CONFIRM_FAILED", e))?;

        Ok(StoredFile {
            id: asset_id,
            url: cdn_url,
            size: content.len(),
        })
    }

    fn get(&self, id: &str) -> Result<Vec<u8>, FileStorageError> {
        // Two-step recovery path: look up the asset's cdnUrl, then fetch bytes.
        // Most callers should embed the cdnUrl returned from `store()` directly
        // and never hit this method.
        let agent = stack0_agent();
        let meta: serde_json::Value = agent
            .get(&format!("{}/cdn/assets/{}", self.base_url, id))
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .call()
            .map_err(|e| match &e {
                ureq::Error::Status(404, _) => stack0_err("NOT_FOUND", "Asset not found"),
                _ => stack0_err("STACK0_GET_FAILED", e),
            })?
            .into_json()
            .map_err(|e| stack0_err("STACK0_GET_PARSE", e))?;

        let cdn_url = meta["cdnUrl"]
            .as_str()
            .ok_or_else(|| stack0_err("STACK0_GET_BAD_RESPONSE", "missing cdnUrl"))?;

        let mut buf = Vec::new();
        agent
            .get(cdn_url)
            .call()
            .map_err(|e| stack0_err("STACK0_FETCH_FAILED", e))?
            .into_reader()
            .read_to_end(&mut buf)
            .map_err(|e| stack0_err("STACK0_FETCH_READ", e))?;
        Ok(buf)
    }

    fn delete(&self, id: &str) -> Result<bool, FileStorageError> {
        let agent = stack0_agent();
        match agent
            .delete(&format!("{}/cdn/assets/{}", self.base_url, id))
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .call()
        {
            Ok(_) => Ok(true),
            Err(ureq::Error::Status(404, _)) => Ok(false),
            Err(e) => Err(stack0_err("STACK0_DELETE_FAILED", e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_store_and_get() {
        let dir = std::env::temp_dir().join(format!("pylon_files_{}", std::process::id()));
        let storage = LocalFileStorage::new(dir.to_str().unwrap(), "/api/files");

        let stored = storage
            .store("test.txt", b"hello world", "text/plain")
            .unwrap();
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
        let dir = std::env::temp_dir().join(format!("pylon_files2_{}", std::process::id()));
        let storage = LocalFileStorage::new(dir.to_str().unwrap(), "/api/files");

        assert!(storage.get("../etc/passwd").is_err());
        assert!(storage.delete("../etc/passwd").is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stack0_upload_init_body_shape() {
        let storage = Stack0FileStorage::new("sk_test_123");
        let body = storage.build_upload_init_body("photo.jpg", "image/jpeg", 4096);
        assert_eq!(body["filename"], "photo.jpg");
        assert_eq!(body["mimeType"], "image/jpeg");
        assert_eq!(body["size"], 4096);
        assert!(body.get("folder").is_none());
    }

    #[test]
    fn stack0_upload_init_body_includes_folder() {
        let storage = Stack0FileStorage::new("sk_test_123").with_folder("avatars");
        let body = storage.build_upload_init_body("photo.jpg", "image/jpeg", 4096);
        assert_eq!(body["folder"], "avatars");
    }

    #[test]
    fn stack0_default_base_url() {
        let storage = Stack0FileStorage::new("sk_test_123");
        assert_eq!(storage.base_url, "https://api.stack0.dev");
    }

    #[test]
    fn stack0_with_base_url_override() {
        let storage = Stack0FileStorage::new("sk_test_123").with_base_url("http://localhost:9999");
        assert_eq!(storage.base_url, "http://localhost:9999");
    }
}
