use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::Plugin;

// ---------------------------------------------------------------------------
// Timestamp helper
// ---------------------------------------------------------------------------

fn now() -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{ts}Z")
}

/// Generate a unique file ID from a timestamp and a monotonic counter.
fn generate_id(counter: &mut u64) -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    *counter += 1;
    format!("file_{ts}_{}", *counter)
}

// ---------------------------------------------------------------------------
// File ID validation (defense-in-depth against path traversal)
// ---------------------------------------------------------------------------

/// Validate that a file ID does not contain path traversal sequences or
/// separators. Even though IDs are internally generated, external callers
/// may provide arbitrary strings through the download/delete/metadata APIs.
fn validate_file_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("File ID must not be empty".into());
    }
    if id.contains("..") || id.contains('/') || id.contains('\\') || id.starts_with('.') {
        return Err("Invalid file ID: must not contain path separators or '..'".into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// Metadata associated with a stored file.
#[derive(Debug, Clone)]
pub struct FileMetadata {
    pub content_type: String,
    pub size: u64,
    pub created_at: String,
    pub original_name: String,
}

/// Information returned after a successful upload.
#[derive(Debug, Clone)]
pub struct FileInfo {
    pub id: String,
    pub url: String,
    pub size: u64,
    pub content_type: String,
}

// ---------------------------------------------------------------------------
// StorageBackend trait
// ---------------------------------------------------------------------------

/// Abstraction over file storage backends (local disk, S3, GCS, etc.).
pub trait StorageBackend: Send + Sync {
    /// Store data under the given ID with associated metadata.
    fn store(&self, id: &str, data: &[u8], metadata: &FileMetadata) -> Result<FileInfo, String>;

    /// Retrieve the raw bytes for a stored file.
    fn retrieve(&self, id: &str) -> Result<Vec<u8>, String>;

    /// Delete a stored file. Returns `true` if the file existed and was removed.
    fn delete(&self, id: &str) -> Result<bool, String>;

    /// Check whether a file exists.
    fn exists(&self, id: &str) -> bool;

    /// Retrieve metadata for a stored file, if it exists.
    fn metadata(&self, id: &str) -> Option<FileMetadata>;

    /// List all stored file IDs.
    fn list(&self) -> Vec<String>;
}

// ---------------------------------------------------------------------------
// LocalBackend — stores files on the local filesystem
// ---------------------------------------------------------------------------

/// A storage backend that persists files to a directory on disk and keeps a
/// metadata index in memory.
pub struct LocalBackend {
    root: PathBuf,
    index: Mutex<HashMap<String, FileMetadata>>,
}

impl LocalBackend {
    /// Create a new `LocalBackend` rooted at `dir`. The directory is created
    /// if it does not already exist.
    pub fn new(dir: impl AsRef<Path>) -> Result<Self, String> {
        let root = dir.as_ref().to_path_buf();
        fs::create_dir_all(&root).map_err(|e| format!("failed to create storage dir: {e}"))?;
        Ok(Self {
            root,
            index: Mutex::new(HashMap::new()),
        })
    }

    fn file_path(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }
}

impl StorageBackend for LocalBackend {
    fn store(&self, id: &str, data: &[u8], metadata: &FileMetadata) -> Result<FileInfo, String> {
        validate_file_id(id)?;
        let path = self.file_path(id);
        fs::write(&path, data).map_err(|e| format!("write failed: {e}"))?;

        let info = FileInfo {
            id: id.to_string(),
            url: format!("file://{}", path.display()),
            size: data.len() as u64,
            content_type: metadata.content_type.clone(),
        };

        self.index
            .lock()
            .unwrap()
            .insert(id.to_string(), metadata.clone());

        Ok(info)
    }

    fn retrieve(&self, id: &str) -> Result<Vec<u8>, String> {
        validate_file_id(id)?;
        let path = self.file_path(id);
        fs::read(&path).map_err(|e| format!("read failed for {id}: {e}"))
    }

    fn delete(&self, id: &str) -> Result<bool, String> {
        validate_file_id(id)?;
        let path = self.file_path(id);
        let existed = path.exists();
        if existed {
            fs::remove_file(&path).map_err(|e| format!("delete failed for {id}: {e}"))?;
        }
        self.index.lock().unwrap().remove(id);
        Ok(existed)
    }

    fn exists(&self, id: &str) -> bool {
        if validate_file_id(id).is_err() {
            return false;
        }
        self.file_path(id).exists()
    }

    fn metadata(&self, id: &str) -> Option<FileMetadata> {
        if validate_file_id(id).is_err() {
            return None;
        }
        self.index.lock().unwrap().get(id).cloned()
    }

    fn list(&self) -> Vec<String> {
        let index = self.index.lock().unwrap();
        let mut ids: Vec<String> = index.keys().cloned().collect();
        ids.sort();
        ids
    }
}

// ---------------------------------------------------------------------------
// FileStoragePlugin
// ---------------------------------------------------------------------------

/// Plugin that provides file upload, download, deletion, and metadata queries.
pub struct FileStoragePlugin {
    backend: Box<dyn StorageBackend + Send + Sync>,
    counter: Mutex<u64>,
}

impl FileStoragePlugin {
    /// Create a new `FileStoragePlugin` backed by the given storage backend.
    pub fn new(backend: Box<dyn StorageBackend + Send + Sync>) -> Self {
        Self {
            backend,
            counter: Mutex::new(0),
        }
    }

    /// Create a plugin using a `LocalBackend` rooted at `dir`.
    pub fn local(dir: impl AsRef<Path>) -> Result<Self, String> {
        let backend = LocalBackend::new(dir)?;
        Ok(Self::new(Box::new(backend)))
    }

    /// Upload a file. Returns a `FileInfo` describing the stored object.
    pub fn upload(
        &self,
        data: &[u8],
        content_type: &str,
        original_name: &str,
    ) -> Result<FileInfo, String> {
        let id = {
            let mut c = self.counter.lock().unwrap();
            generate_id(&mut c)
        };

        let metadata = FileMetadata {
            content_type: content_type.to_string(),
            size: data.len() as u64,
            created_at: now(),
            original_name: original_name.to_string(),
        };

        self.backend.store(&id, data, &metadata)
    }

    /// Download the raw bytes of a stored file.
    pub fn download(&self, id: &str) -> Result<Vec<u8>, String> {
        self.backend.retrieve(id)
    }

    /// Delete a file by ID. Returns `true` if it existed.
    pub fn delete(&self, id: &str) -> Result<bool, String> {
        self.backend.delete(id)
    }

    /// Get metadata for a file.
    pub fn get_metadata(&self, id: &str) -> Option<FileMetadata> {
        self.backend.metadata(id)
    }

    /// List all stored file IDs.
    pub fn list_files(&self) -> Vec<String> {
        self.backend.list()
    }

    /// Check whether a file exists.
    pub fn exists(&self, id: &str) -> bool {
        self.backend.exists(id)
    }
}

impl Plugin for FileStoragePlugin {
    fn name(&self) -> &str {
        "file-storage"
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    /// Create a temporary directory for test isolation.
    fn test_dir(suffix: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("pylon_file_storage_test_{suffix}"));
        // Ensure a clean slate.
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn make_plugin(suffix: &str) -> FileStoragePlugin {
        FileStoragePlugin::local(test_dir(suffix)).expect("should create local backend")
    }

    #[test]
    fn upload_and_download() {
        let plugin = make_plugin("upload_download");
        let data = b"hello, world";
        let info = plugin
            .upload(data, "text/plain", "hello.txt")
            .expect("upload should succeed");

        assert!(!info.id.is_empty());
        assert_eq!(info.size, data.len() as u64);
        assert_eq!(info.content_type, "text/plain");
        assert!(info.url.contains(&info.id));

        let downloaded = plugin.download(&info.id).expect("download should succeed");
        assert_eq!(downloaded, data);
    }

    #[test]
    fn delete_file() {
        let plugin = make_plugin("delete");
        let info = plugin
            .upload(b"temp", "application/octet-stream", "temp.bin")
            .expect("upload should succeed");

        assert!(plugin.exists(&info.id));

        let removed = plugin.delete(&info.id).expect("delete should succeed");
        assert!(removed);

        assert!(!plugin.exists(&info.id));

        // Deleting again returns false.
        let removed_again = plugin.delete(&info.id).expect("delete should succeed");
        assert!(!removed_again);
    }

    #[test]
    fn metadata_returned() {
        let plugin = make_plugin("metadata");
        let info = plugin
            .upload(b"abc", "image/png", "photo.png")
            .expect("upload should succeed");

        let meta = plugin
            .get_metadata(&info.id)
            .expect("metadata should exist");
        assert_eq!(meta.content_type, "image/png");
        assert_eq!(meta.original_name, "photo.png");
        assert_eq!(meta.size, 3);
        assert!(meta.created_at.ends_with('Z'));
    }

    #[test]
    fn exists_check() {
        let plugin = make_plugin("exists");
        assert!(!plugin.exists("nonexistent"));

        let info = plugin
            .upload(b"x", "text/plain", "x.txt")
            .expect("upload should succeed");
        assert!(plugin.exists(&info.id));
    }

    #[test]
    fn list_files() {
        let plugin = make_plugin("list");
        assert!(plugin.list_files().is_empty());

        let a = plugin
            .upload(b"a", "text/plain", "a.txt")
            .expect("upload a");
        let b = plugin
            .upload(b"b", "text/plain", "b.txt")
            .expect("upload b");

        let files = plugin.list_files();
        assert_eq!(files.len(), 2);
        assert!(files.contains(&a.id));
        assert!(files.contains(&b.id));
    }

    #[test]
    fn not_found() {
        let plugin = make_plugin("not_found");

        let result = plugin.download("does_not_exist");
        assert!(result.is_err());

        let meta = plugin.get_metadata("does_not_exist");
        assert!(meta.is_none());
    }

    #[test]
    fn plugin_name() {
        let plugin = make_plugin("name");
        assert_eq!(Plugin::name(&plugin), "file-storage");
    }

    // -- Path traversal defense tests --

    #[test]
    fn rejects_path_traversal_dotdot() {
        assert!(validate_file_id("../etc/passwd").is_err());
        assert!(validate_file_id("foo/../bar").is_err());
    }

    #[test]
    fn rejects_path_traversal_slash() {
        assert!(validate_file_id("foo/bar").is_err());
        assert!(validate_file_id("/etc/passwd").is_err());
    }

    #[test]
    fn rejects_path_traversal_backslash() {
        assert!(validate_file_id("foo\\bar").is_err());
    }

    #[test]
    fn rejects_hidden_files() {
        assert!(validate_file_id(".hidden").is_err());
        assert!(validate_file_id(".").is_err());
    }

    #[test]
    fn rejects_empty_id() {
        assert!(validate_file_id("").is_err());
    }

    #[test]
    fn accepts_valid_file_id() {
        assert!(validate_file_id("file_123456_1").is_ok());
        assert!(validate_file_id("my-file.txt").is_ok());
    }

    #[test]
    fn local_backend_rejects_traversal_on_retrieve() {
        let backend = LocalBackend::new(test_dir("traversal_retrieve")).unwrap();
        assert!(backend.retrieve("../etc/passwd").is_err());
    }

    #[test]
    fn local_backend_rejects_traversal_on_delete() {
        let backend = LocalBackend::new(test_dir("traversal_delete")).unwrap();
        assert!(backend.delete("../etc/passwd").is_err());
    }

    #[test]
    fn local_backend_rejects_traversal_on_exists() {
        let backend = LocalBackend::new(test_dir("traversal_exists")).unwrap();
        assert!(!backend.exists("../etc/passwd"));
    }

    #[test]
    fn local_backend_rejects_traversal_on_metadata() {
        let backend = LocalBackend::new(test_dir("traversal_metadata")).unwrap();
        assert!(backend.metadata("../etc/passwd").is_none());
    }
}
