//! Pluggable file storage abstraction.
//!
//! Provides a trait for file storage operations that can be implemented
//! for local disk, S3, R2, GCS, or any other storage backend.

use serde::{Deserialize, Serialize};
use std::io::Read;

// ---------------------------------------------------------------------------
// File ownership
// ---------------------------------------------------------------------------

/// Owner of a stored file. Recorded at upload time; checked on retrieval so
/// a logged-in caller cannot enumerate file IDs uploaded by other users.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileOwner {
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
}

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

    /// Record the owner of a stored file. Called by the upload path
    /// immediately after `store()` so ownership is persisted next to the bytes.
    /// Default impl is a noop for backends that delegate auth elsewhere
    /// (e.g., Stack0, where access is mediated by a CDN-level API key).
    fn record_owner(&self, _id: &str, _owner: &FileOwner) -> Result<(), FileStorageError> {
        Ok(())
    }

    /// Look up the owner of a stored file. Returns `Ok(None)` for backends
    /// that don't track ownership, or for files predating the ownership
    /// machinery (legacy uploads). Callers that require owner enforcement
    /// must combine this with `requires_owner_check()`.
    fn owner_of(&self, _id: &str) -> Result<Option<FileOwner>, FileStorageError> {
        Ok(None)
    }

    /// Whether the router should enforce per-file owner checks against this
    /// backend. Local disk: yes. Stack0: no (its CDN auth handles access).
    fn requires_owner_check(&self) -> bool {
        false
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
// Provider selection
// ---------------------------------------------------------------------------

/// Pick a [`FileStorage`] backend from environment variables. Single
/// source of truth for env-driven provider selection so every code
/// path (router-level `FileOpsAdapter`, the runtime's multipart
/// upload handler, plugin-driven callers) agrees on which backend
/// receives an upload.
///
/// Selects via `PYLON_FILES_PROVIDER`:
/// - `local` (default) — `LocalFileStorage` rooted at
///   `PYLON_FILES_DIR` (default `uploads/`), served at
///   `PYLON_FILES_URL_PREFIX` (default `/api/files`).
/// - `stack0` — `Stack0FileStorage` from `PYLON_STACK0_API_KEY`
///   (+ optional `PYLON_STACK0_FOLDER`, `PYLON_STACK0_BASE_URL`).
///   If the API key is missing, logs a warning and falls back to
///   local storage rather than failing uploads outright.
///
/// Real-world bug this exists to prevent: pylon ≤0.3.86 had the
/// runtime upload handler hardcoded to `LocalFileStorage::new(...)`,
/// so every upload landed on local disk regardless of provider env.
/// Apps set `PYLON_FILES_PROVIDER=stack0` thinking files would go to
/// the CDN, and got silent local storage instead.
pub fn select_from_env() -> Box<dyn FileStorage> {
    let provider = std::env::var("PYLON_FILES_PROVIDER").unwrap_or_else(|_| "local".into());
    match provider.as_str() {
        "stack0" => match Stack0FileStorage::from_env() {
            Some(s) => Box::new(s),
            None => {
                // Post-boot guarantee: server startup calls
                // `validate_provider_env()` and refuses to start when
                // stack0 vars are missing, so reaching this branch
                // post-startup means the env was mutated mid-run
                // (rare; container restart usually re-reads). Keep
                // the warn + fallback so the server doesn't crash on
                // every upload after that, but the user-visible
                // failure already fired at boot.
                tracing::warn!(
                    "PYLON_FILES_PROVIDER=stack0 but PYLON_STACK0_API_KEY / PYLON_STACK0_PROJECT_SLUG is not set; falling back to local storage"
                );
                Box::new(local_from_env())
            }
        },
        _ => Box::new(local_from_env()),
    }
}

/// Boot-time validation. Returns `Err(message)` when the file-storage
/// env vars are internally inconsistent — e.g. `PYLON_FILES_PROVIDER=stack0`
/// without `PYLON_STACK0_API_KEY` + `PYLON_STACK0_PROJECT_SLUG`. The
/// server's `start()` calls this before opening the listener, so a
/// misconfigured deploy fails loud at boot instead of producing
/// confusing per-upload 400s.
///
/// This exists because of the yapless rollout: v0.3.87 routed uploads
/// to Stack0 successfully but Stack0's `/v1/cdn/upload` requires
/// `projectSlug`, which pylon didn't pass; every upload 400'd. The
/// upstream symptom ("upload failed") gave no hint that the operator
/// had to set a new env var. Now they get
/// `PYLON_FILES_PROVIDER=stack0 requires PYLON_STACK0_PROJECT_SLUG`
/// at boot, before any user ever tries to upload.
pub fn validate_provider_env() -> Result<(), String> {
    let provider = std::env::var("PYLON_FILES_PROVIDER").unwrap_or_else(|_| "local".into());
    if provider == "stack0" {
        let missing: Vec<&str> = ["PYLON_STACK0_API_KEY", "PYLON_STACK0_PROJECT_SLUG"]
            .into_iter()
            .filter(|k| std::env::var(k).map(|v| v.is_empty()).unwrap_or(true))
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "PYLON_FILES_PROVIDER=stack0 requires {} to be set. \
                 Configure {} alongside PYLON_FILES_PROVIDER, or remove \
                 PYLON_FILES_PROVIDER (defaults to local disk).",
                missing.join(" + "),
                missing.join(" + "),
            ));
        }
    }
    Ok(())
}

/// Construct the local-disk backend from env. Public so callers that
/// explicitly want local storage (tests, the fallback path inside
/// `select_from_env`) can skip the provider dispatch.
pub fn local_from_env() -> LocalFileStorage {
    let dir = std::env::var("PYLON_FILES_DIR").unwrap_or_else(|_| "uploads".into());
    let url_prefix =
        std::env::var("PYLON_FILES_URL_PREFIX").unwrap_or_else(|_| "/api/files".into());
    LocalFileStorage::new(&dir, &url_prefix)
}

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
        let _ = std::fs::create_dir_all(path.join(".ownership"));
        Self {
            dir: path,
            url_prefix: url_prefix.to_string(),
        }
    }

    fn owner_sidecar_path(&self, id: &str) -> std::path::PathBuf {
        // id is validated by `validate_local_id` at every entry point that
        // accepts external input, so no traversal segments can leak in.
        self.dir.join(".ownership").join(format!("{id}.json"))
    }
}

/// Reject IDs that try to escape the storage dir or the ownership sidecar dir.
fn validate_local_id(id: &str) -> Result<(), FileStorageError> {
    if id.is_empty() || id.contains("..") || id.contains('/') || id.contains('\\') {
        return Err(FileStorageError {
            code: "INVALID_ID".into(),
            message: "Invalid file ID".into(),
        });
    }
    Ok(())
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
        validate_local_id(id)?;
        let path = self.dir.join(id);
        std::fs::read(&path).map_err(|_| FileStorageError {
            code: "NOT_FOUND".into(),
            message: "File not found".into(),
        })
    }

    fn delete(&self, id: &str) -> Result<bool, FileStorageError> {
        validate_local_id(id)?;
        let path = self.dir.join(id);
        let existed = match std::fs::remove_file(&path) {
            Ok(()) => true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => {
                return Err(FileStorageError {
                    code: "DELETE_FAILED".into(),
                    message: format!("Failed to delete file: {e}"),
                });
            }
        };
        // Best-effort sidecar cleanup so an owner record can't outlive its file.
        let _ = std::fs::remove_file(self.owner_sidecar_path(id));
        Ok(existed)
    }

    fn record_owner(&self, id: &str, owner: &FileOwner) -> Result<(), FileStorageError> {
        validate_local_id(id)?;
        let dir = self.dir.join(".ownership");
        std::fs::create_dir_all(&dir).map_err(|e| FileStorageError {
            code: "OWNERSHIP_DIR_FAILED".into(),
            message: format!("Failed to create ownership dir: {e}"),
        })?;
        let path = self.owner_sidecar_path(id);
        let body = serde_json::to_vec(owner).map_err(|e| FileStorageError {
            code: "OWNERSHIP_ENCODE_FAILED".into(),
            message: format!("Failed to serialize owner: {e}"),
        })?;
        std::fs::write(&path, body).map_err(|e| FileStorageError {
            code: "OWNERSHIP_WRITE_FAILED".into(),
            message: format!("Failed to write owner sidecar: {e}"),
        })?;
        Ok(())
    }

    fn owner_of(&self, id: &str) -> Result<Option<FileOwner>, FileStorageError> {
        validate_local_id(id)?;
        let path = self.owner_sidecar_path(id);
        match std::fs::read(&path) {
            Ok(bytes) => {
                let owner: FileOwner =
                    serde_json::from_slice(&bytes).map_err(|e| FileStorageError {
                        code: "OWNERSHIP_DECODE_FAILED".into(),
                        message: format!("Corrupt owner sidecar for {id}: {e}"),
                    })?;
                Ok(Some(owner))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(FileStorageError {
                code: "OWNERSHIP_READ_FAILED".into(),
                message: format!("Failed to read owner sidecar: {e}"),
            }),
        }
    }

    fn requires_owner_check(&self) -> bool {
        true
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
    /// Stack0 project slug — every `/cdn/upload` call must name the
    /// project the asset belongs to. Stack0's API rejects bodies
    /// without it with `400 Bad Request`. Read from
    /// `PYLON_STACK0_PROJECT_SLUG` at boot.
    project_slug: String,
    /// Base API URL — typically `https://api.stack0.dev/v1`. Configurable
    /// so tests can point at a local mock server, and so ops can hotfix
    /// via `PYLON_STACK0_BASE_URL` if Stack0 ever changes its API
    /// version prefix without forcing a pylon republish.
    ///
    /// The `/v1` suffix is part of the base because every Stack0
    /// endpoint pylon hits (`/cdn/upload`, `/cdn/upload/<id>/confirm`,
    /// `/cdn/assets/<id>`) sits under it. Without `/v1` every request
    /// 404s — caught by yapless when voice-clone uploads started
    /// 500'ing post-v0.3.87 (which was the release that finally
    /// wired Stack0 into the upload path at all).
    base_url: String,
    /// Optional folder/prefix for organizing uploads.
    folder: Option<String>,
}

impl Stack0FileStorage {
    pub fn new(api_key: impl Into<String>, project_slug: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            project_slug: project_slug.into(),
            base_url: "https://api.stack0.dev/v1".into(),
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
    ///
    /// Required: `PYLON_STACK0_API_KEY`, `PYLON_STACK0_PROJECT_SLUG`.
    /// Optional: `PYLON_STACK0_FOLDER`, `PYLON_STACK0_BASE_URL`.
    ///
    /// Returns `None` if any required var is missing — callers fail-fast
    /// rather than silently degrading to local storage, since the wrong
    /// backend means surprise 400s downstream.
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("PYLON_STACK0_API_KEY").ok()?;
        let project_slug = std::env::var("PYLON_STACK0_PROJECT_SLUG").ok()?;
        let mut s = Self::new(api_key, project_slug);
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
    ///
    /// `projectSlug` is REQUIRED by Stack0's API — emitting a body
    /// without it produces a `400 Bad Request` with no explanation
    /// from the upload URL minting step. Pre-0.3.89 pylon omitted
    /// it; every Stack0 upload silently 400'd.
    pub fn build_upload_init_body(
        &self,
        filename: &str,
        content_type: &str,
        size: usize,
    ) -> serde_json::Value {
        let mut body = serde_json::json!({
            "projectSlug": self.project_slug,
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
        //
        // `send_json({})` matters: Stack0 requires `Content-Type:
        // application/json` on the confirm POST and 415s otherwise.
        // ureq's `.call()` sends no body and no Content-Type. Pre-0.3.90
        // pylon used `.call()` here, so the upload bytes landed on the
        // CDN but the asset stayed in a half-confirmed state and the
        // store() call returned an error to the caller.
        agent
            .post(&format!(
                "{}/cdn/upload/{}/confirm",
                self.base_url, asset_id
            ))
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .send_json(serde_json::json!({}))
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
    fn local_owner_round_trip() {
        let dir = std::env::temp_dir().join(format!("pylon_files_owner_{}", std::process::id()));
        let storage = LocalFileStorage::new(dir.to_str().unwrap(), "/api/files");

        let stored = storage.store("a.txt", b"hi", "text/plain").unwrap();
        assert!(storage.requires_owner_check());
        assert!(storage.owner_of(&stored.id).unwrap().is_none());

        storage
            .record_owner(
                &stored.id,
                &FileOwner {
                    user_id: "u-alice".into(),
                    tenant_id: Some("t-1".into()),
                },
            )
            .unwrap();

        let owner = storage.owner_of(&stored.id).unwrap().unwrap();
        assert_eq!(owner.user_id, "u-alice");
        assert_eq!(owner.tenant_id.as_deref(), Some("t-1"));

        // Deleting the file also clears the sidecar.
        storage.delete(&stored.id).unwrap();
        assert!(storage.owner_of(&stored.id).unwrap().is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_owner_rejects_traversal() {
        let dir = std::env::temp_dir().join(format!(
            "pylon_files_owner_traversal_{}",
            std::process::id()
        ));
        let storage = LocalFileStorage::new(dir.to_str().unwrap(), "/api/files");

        let bad_owner = FileOwner {
            user_id: "u".into(),
            tenant_id: None,
        };
        assert!(storage.record_owner("../escape", &bad_owner).is_err());
        assert!(storage.owner_of("../escape").is_err());
        assert!(storage.owner_of("foo/bar").is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stack0_upload_init_body_shape() {
        let storage = Stack0FileStorage::new("sk_test_123", "yapless");
        let body = storage.build_upload_init_body("photo.jpg", "image/jpeg", 4096);
        assert_eq!(body["projectSlug"], "yapless");
        assert_eq!(body["filename"], "photo.jpg");
        assert_eq!(body["mimeType"], "image/jpeg");
        assert_eq!(body["size"], 4096);
        assert!(body.get("folder").is_none());
    }

    #[test]
    fn stack0_upload_init_body_always_includes_project_slug() {
        // Regression: pre-0.3.89 the wire shape omitted projectSlug,
        // which Stack0's API requires. Every upload 400'd. Lock the
        // field into the body so a future edit can't silently drop it.
        let storage = Stack0FileStorage::new("sk_test_123", "yapless");
        let body = storage.build_upload_init_body("photo.jpg", "image/jpeg", 4096);
        assert_eq!(
            body["projectSlug"], "yapless",
            "Stack0 /cdn/upload requires projectSlug; pre-0.3.89 pylon omitted it"
        );
        // Same assertion with a folder set — folder must not displace
        // projectSlug.
        let with_folder = Stack0FileStorage::new("sk_test_123", "yapless").with_folder("avatars");
        let body = with_folder.build_upload_init_body("photo.jpg", "image/jpeg", 4096);
        assert_eq!(body["projectSlug"], "yapless");
        assert_eq!(body["folder"], "avatars");
    }

    #[test]
    fn stack0_upload_init_body_includes_folder() {
        let storage = Stack0FileStorage::new("sk_test_123", "yapless").with_folder("avatars");
        let body = storage.build_upload_init_body("photo.jpg", "image/jpeg", 4096);
        assert_eq!(body["folder"], "avatars");
    }

    #[test]
    fn stack0_default_base_url() {
        let storage = Stack0FileStorage::new("sk_test_123", "yapless");
        assert_eq!(storage.base_url, "https://api.stack0.dev/v1");
    }

    #[test]
    fn stack0_base_url_includes_v1_prefix() {
        // Regression: pre-0.3.88 the base_url was `https://api.stack0.dev`
        // without `/v1`, so every `/cdn/*` request 404'd. This test
        // locks the prefix in so an accidental edit can't drift back.
        // Both the bare default AND any `from_env` path that doesn't
        // override PYLON_STACK0_BASE_URL must end up under `/v1`.
        let s = Stack0FileStorage::new("sk_test", "yapless");
        assert!(
            s.base_url.ends_with("/v1"),
            "default Stack0 base URL must end with `/v1` (was {:?})",
            s.base_url
        );
    }

    #[test]
    fn stack0_with_base_url_override() {
        let storage =
            Stack0FileStorage::new("sk_test_123", "yapless").with_base_url("http://localhost:9999");
        assert_eq!(storage.base_url, "http://localhost:9999");
    }

    // ---- select_from_env: regression coverage for the v0.3.86 bug ----

    /// Helper to set/clear several env vars atomically for the duration
    /// of a closure. Reverts on drop. Std `env::set_var` is process-wide
    /// so these tests can't actually run in parallel safely; cargo test
    /// runs them serially within the binary anyway, but we keep the
    /// helper terse so the intent is obvious.
    struct EnvGuard {
        saved: Vec<(String, Option<String>)>,
    }
    impl EnvGuard {
        fn new(keys: &[&str]) -> Self {
            let saved = keys
                .iter()
                .map(|k| (k.to_string(), std::env::var(k).ok()))
                .collect();
            for k in keys {
                std::env::remove_var(k);
            }
            Self { saved }
        }
        fn set(&self, key: &str, value: &str) {
            std::env::set_var(key, value);
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (k, v) in &self.saved {
                match v {
                    Some(val) => std::env::set_var(k, val),
                    None => std::env::remove_var(k),
                }
            }
        }
    }

    #[test]
    fn select_from_env_defaults_to_local() {
        let tmp = tempfile::tempdir().unwrap();
        let g = EnvGuard::new(&[
            "PYLON_FILES_PROVIDER",
            "PYLON_FILES_DIR",
            "PYLON_FILES_URL_PREFIX",
            "PYLON_STACK0_API_KEY",
        ]);
        g.set("PYLON_FILES_DIR", tmp.path().to_str().unwrap());
        let storage = select_from_env();
        // Local backend persists a real file when called. Stack0 would
        // hit the network and fail. A round-trip through store→get
        // confirms the local backend got picked.
        let stored = storage.store("a.txt", b"hello", "text/plain").unwrap();
        let bytes = storage.get(&stored.id).unwrap();
        assert_eq!(bytes, b"hello");
    }

    #[test]
    fn select_from_env_stack0_without_key_falls_back_to_local() {
        // Set provider=stack0 but no API key — should warn and use
        // local storage rather than failing. Critical for users who
        // misconfigure prod env: a missing key shouldn't 500 every
        // upload, just log and degrade. Pre-0.3.87 the runtime
        // hardcoded local storage AND never hit this branch — the
        // bug was silent. With select_from_env routing both paths,
        // this fallback contract becomes worth testing.
        let tmp = tempfile::tempdir().unwrap();
        let g = EnvGuard::new(&[
            "PYLON_FILES_PROVIDER",
            "PYLON_FILES_DIR",
            "PYLON_FILES_URL_PREFIX",
            "PYLON_STACK0_API_KEY",
        ]);
        g.set("PYLON_FILES_PROVIDER", "stack0");
        g.set("PYLON_FILES_DIR", tmp.path().to_str().unwrap());
        let storage = select_from_env();
        // If this had picked Stack0 the store call would attempt a
        // network request and fail. Local store always succeeds.
        let stored = storage.store("a.txt", b"hello", "text/plain").unwrap();
        assert!(storage.get(&stored.id).is_ok());
    }

    #[test]
    fn select_from_env_unknown_provider_defaults_to_local() {
        // Anything that isn't `stack0` falls into the local arm.
        // Defensive: typos like `PYLON_FILES_PROVIDER=Stack0` (capital S)
        // shouldn't silently degrade to local; document the contract.
        let tmp = tempfile::tempdir().unwrap();
        let g = EnvGuard::new(&["PYLON_FILES_PROVIDER", "PYLON_FILES_DIR"]);
        g.set("PYLON_FILES_PROVIDER", "Stack0"); // mis-cased
        g.set("PYLON_FILES_DIR", tmp.path().to_str().unwrap());
        let storage = select_from_env();
        let stored = storage.store("a.txt", b"hi", "text/plain").unwrap();
        assert!(storage.get(&stored.id).is_ok());
    }

    // ---- validate_provider_env: boot-time validation (v0.3.89) ----

    #[test]
    fn validate_provider_env_local_default_ok() {
        let _g = EnvGuard::new(&["PYLON_FILES_PROVIDER", "PYLON_STACK0_API_KEY"]);
        assert!(validate_provider_env().is_ok());
    }

    #[test]
    fn validate_provider_env_stack0_missing_api_key_errs() {
        let g = EnvGuard::new(&[
            "PYLON_FILES_PROVIDER",
            "PYLON_STACK0_API_KEY",
            "PYLON_STACK0_PROJECT_SLUG",
        ]);
        g.set("PYLON_FILES_PROVIDER", "stack0");
        g.set("PYLON_STACK0_PROJECT_SLUG", "yapless");
        let err = validate_provider_env().unwrap_err();
        assert!(err.contains("PYLON_STACK0_API_KEY"), "got: {err}");
        assert!(!err.contains("PYLON_STACK0_PROJECT_SLUG"), "got: {err}");
    }

    #[test]
    fn validate_provider_env_stack0_missing_project_slug_errs() {
        let g = EnvGuard::new(&[
            "PYLON_FILES_PROVIDER",
            "PYLON_STACK0_API_KEY",
            "PYLON_STACK0_PROJECT_SLUG",
        ]);
        g.set("PYLON_FILES_PROVIDER", "stack0");
        g.set("PYLON_STACK0_API_KEY", "sk_test_123");
        let err = validate_provider_env().unwrap_err();
        assert!(err.contains("PYLON_STACK0_PROJECT_SLUG"), "got: {err}");
    }

    #[test]
    fn validate_provider_env_stack0_both_set_ok() {
        let g = EnvGuard::new(&[
            "PYLON_FILES_PROVIDER",
            "PYLON_STACK0_API_KEY",
            "PYLON_STACK0_PROJECT_SLUG",
        ]);
        g.set("PYLON_FILES_PROVIDER", "stack0");
        g.set("PYLON_STACK0_API_KEY", "sk_test_123");
        g.set("PYLON_STACK0_PROJECT_SLUG", "yapless");
        assert!(validate_provider_env().is_ok());
    }

    #[test]
    fn validate_provider_env_stack0_empty_string_treated_as_missing() {
        // Setting a var to "" is the same as not setting it for our
        // purposes — a literal empty string in env is operator error,
        // and validate must catch it the same as omission.
        let g = EnvGuard::new(&[
            "PYLON_FILES_PROVIDER",
            "PYLON_STACK0_API_KEY",
            "PYLON_STACK0_PROJECT_SLUG",
        ]);
        g.set("PYLON_FILES_PROVIDER", "stack0");
        g.set("PYLON_STACK0_API_KEY", "sk_test_123");
        g.set("PYLON_STACK0_PROJECT_SLUG", "");
        let err = validate_provider_env().unwrap_err();
        assert!(err.contains("PYLON_STACK0_PROJECT_SLUG"), "got: {err}");
    }
}
