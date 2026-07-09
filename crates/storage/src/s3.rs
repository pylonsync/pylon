//! S3-compatible file storage (AWS S3, Cloudflare R2, Tigris, MinIO, GCS via
//! the S3 interop endpoint) using AWS Signature Version 4 presigned URLs.
//!
//! # Why presigned URLs for everything
//!
//! Every operation is a SigV4 **presigned URL** plus a plain `ureq` request
//! against it. That gives a single signing path — query-string presigning —
//! so there is exactly one place to get the crypto right, with no separate
//! `Authorization`-header signer to keep in sync. It also means client
//! uploads and downloads go **direct to S3**: `init_upload` hands the browser
//! a presigned PUT URL and `GET /api/files/<id>` 302-redirects to a presigned
//! GET URL, so file bytes never transit pylon's process (a 200MB upload
//! doesn't balloon the server's RSS).
//!
//! # Ownership
//!
//! Unlike Stack0 — whose CDN mediates access via an API key — a private S3
//! bucket has no per-request auth in front of it. So pylon tracks the
//! per-file owner itself, in a sidecar object `<key>.owner`, and
//! [`S3FileStorage::requires_owner_check`] returns `true` for private
//! buckets. `GET`/`DELETE /api/files/<id>` then enforce the same IDOR
//! protection as the local-disk backend: verify the caller owns the file,
//! *then* 302 to the short-lived presigned URL.
//!
//! When `PYLON_S3_PUBLIC_URL` is set the bucket is declared public (objects
//! are served straight from a CDN / public endpoint), so ownership tracking
//! is skipped — the operator has opted every object into public reads, matching
//! Stack0's "the CDN mediates access" posture.
//!
//! The bucket endpoint is operator-configured (`PYLON_S3_ENDPOINT`), never
//! request-derived, so there is no SSRF surface here — unlike the image
//! optimizer / OIDC discovery paths that fetch user-influenced URLs.

use crate::files::{FileOwner, FileStorage, FileStorageError, StoredFile, UploadInit};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::io::Read;

type HmacSha256 = Hmac<Sha256>;

const SERVICE: &str = "s3";
const ALGORITHM: &str = "AWS4-HMAC-SHA256";

/// Presigned-URL lifetimes (seconds).
/// Client upload URL: generous window so a slow phone upload survives.
const UPLOAD_URL_EXPIRES: i64 = 3600;
/// Browser download redirect: short — the 302 is followed immediately.
const DOWNLOAD_URL_EXPIRES: i64 = 300;
/// Server-side one-shot ops (HEAD/PUT/GET/DELETE we issue ourselves).
const OP_URL_EXPIRES: i64 = 60;

/// Upper bound on bytes read by the rarely-used server-side [`FileStorage::get`]
/// path (the browser path 302s to S3 and never touches this). Guards against a
/// rogue multi-gigabyte object exhausting memory; exceeding it errors rather
/// than silently truncating.
const MAX_SERVER_GET_BYTES: u64 = 512 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for an S3-compatible bucket.
///
/// Read from environment variables:
/// - `PYLON_S3_BUCKET` (required)
/// - `PYLON_S3_ACCESS_KEY`, `PYLON_S3_SECRET_KEY` (required)
/// - `PYLON_S3_REGION` (default `us-east-1`)
/// - `PYLON_S3_ENDPOINT` (optional; set for R2/Tigris/MinIO — switches to
///   path-style addressing `<endpoint>/<bucket>/<key>`)
/// - `PYLON_S3_PUBLIC_URL` (optional; a public/CDN base URL. When set, the
///   bucket is treated as public and ownership tracking is skipped)
/// - `PYLON_S3_FOLDER` (optional key prefix)
/// - `PYLON_S3_SESSION_TOKEN` (optional; STS/temporary-credential token —
///   set alongside temporary access/secret keys, e.g. an assumed role)
///
/// Note: credentials are read from env only. Automatic IAM-role credential
/// discovery (ECS task-role endpoint / EC2 IMDS) is not implemented — on AWS,
/// use a scoped IAM user's static keys, or inject the role's temporary
/// credentials (access key + secret + `PYLON_S3_SESSION_TOKEN`) into env.
#[derive(Debug, Clone)]
pub struct S3Config {
    pub bucket: String,
    pub region: String,
    pub endpoint: Option<String>,
    pub access_key: String,
    pub secret_key: String,
    pub session_token: Option<String>,
    pub public_url_prefix: Option<String>,
    pub folder: Option<String>,
}

impl S3Config {
    /// Build from environment. Returns `None` if any required var is missing
    /// or empty — callers fail-fast rather than silently degrading to local
    /// disk (which would lose files on a stateless container). The boot-time
    /// [`crate::files::validate_provider_env`] surfaces the specific missing
    /// vars before the listener opens.
    pub fn from_env() -> Option<Self> {
        Some(Self {
            bucket: non_empty_env("PYLON_S3_BUCKET")?,
            region: non_empty_env("PYLON_S3_REGION").unwrap_or_else(|| "us-east-1".into()),
            endpoint: non_empty_env("PYLON_S3_ENDPOINT"),
            access_key: non_empty_env("PYLON_S3_ACCESS_KEY")?,
            secret_key: non_empty_env("PYLON_S3_SECRET_KEY")?,
            session_token: non_empty_env("PYLON_S3_SESSION_TOKEN"),
            public_url_prefix: non_empty_env("PYLON_S3_PUBLIC_URL"),
            folder: non_empty_env("PYLON_S3_FOLDER"),
        })
    }
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

// ---------------------------------------------------------------------------
// S3FileStorage
// ---------------------------------------------------------------------------

/// [`FileStorage`] backed by an S3-compatible bucket.
pub struct S3FileStorage {
    cfg: S3Config,
    /// Canonical host used in both the request URL authority and the SigV4
    /// `host` signed header — the two MUST match byte-for-byte or S3 rejects
    /// the signature. Includes a `:port` for non-default ports (MinIO).
    host: String,
    /// `https` for AWS/R2/Tigris; honours `http://` for a local MinIO endpoint.
    scheme: String,
    /// Path-style (`/<bucket>/<key>`) when a custom endpoint is set —
    /// R2/Tigris/MinIO use it. Virtual-hosted (`<bucket>.s3.<region>...`)
    /// otherwise, which is AWS's modern default.
    path_style: bool,
}

impl S3FileStorage {
    pub fn new(cfg: S3Config) -> Self {
        let (scheme, host, path_style) = match &cfg.endpoint {
            Some(ep) => {
                let (scheme, host) = split_scheme_host(ep);
                (scheme, host, true)
            }
            None => (
                "https".to_string(),
                format!("{}.s3.{}.amazonaws.com", cfg.bucket, cfg.region),
                false,
            ),
        };
        Self {
            cfg,
            host,
            scheme,
            path_style,
        }
    }

    /// Build from environment. See [`S3Config::from_env`].
    pub fn from_env() -> Option<Self> {
        S3Config::from_env().map(Self::new)
    }

    /// Whether the operator declared the bucket public via `PYLON_S3_PUBLIC_URL`.
    fn is_public(&self) -> bool {
        self.cfg.public_url_prefix.is_some()
    }

    /// Canonical (already percent-encoded) request path for an object key.
    /// Path-style prepends the bucket segment.
    fn canonical_uri(&self, key: &str) -> String {
        let enc_key = uri_encode(key, false); // keep `/` between key segments
        if self.path_style {
            format!("/{}/{}", uri_encode(&self.cfg.bucket, true), enc_key)
        } else {
            format!("/{enc_key}")
        }
    }

    /// The URL an app embeds to serve this object.
    /// - public bucket → the public/CDN URL (direct, no pylon hop);
    /// - private bucket → `/api/files/<key>`, which owner-checks then
    ///   302-redirects to a presigned GET URL.
    fn served_url(&self, key: &str) -> String {
        match &self.cfg.public_url_prefix {
            Some(prefix) => format!("{}/{}", prefix.trim_end_matches('/'), key),
            None => format!("/api/files/{key}"),
        }
    }

    /// Mint an unguessable object key. A 128-bit CSPRNG nonce (not just a
    /// timestamp) means keys can't be enumerated even on a public bucket.
    fn mint_key(&self, name: &str) -> String {
        use rand::RngCore;
        let mut nonce = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut nonce);
        let base = format!("file_{}_{}", hex_lower(&nonce), sanitize_name(name));
        match &self.cfg.folder {
            Some(folder) => format!("{}/{}", folder.trim_matches('/'), base),
            None => base,
        }
    }

    fn agent(&self) -> ureq::Agent {
        ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(10))
            .timeout_read(std::time::Duration::from_secs(60))
            .timeout_write(std::time::Duration::from_secs(120))
            .user_agent("pylon-storage/0.1")
            .build()
    }

    /// Presign a request for `method` on `key`, valid for `expires` seconds.
    fn presign(&self, method: &str, key: &str, expires: i64) -> String {
        let (amz_date, date_stamp) = now_timestamps();
        let params = PresignParams {
            method,
            scheme: &self.scheme,
            host: &self.host,
            canonical_uri: &self.canonical_uri(key),
            region: &self.cfg.region,
            access_key: &self.cfg.access_key,
            secret_key: &self.cfg.secret_key,
            amz_date: &amz_date,
            date_stamp: &date_stamp,
            expires,
            session_token: self.cfg.session_token.as_deref(),
        };
        presign_url(&params).0
    }
}

fn owner_key(key: &str) -> String {
    format!("{key}.owner")
}

impl FileStorage for S3FileStorage {
    fn init_upload(
        &self,
        name: &str,
        _content_type: &str,
        _size: usize,
    ) -> Result<UploadInit, FileStorageError> {
        // Presigned PUT: the client uploads bytes straight to S3. The payload
        // hash is `UNSIGNED-PAYLOAD` and only `host` is signed, so the client
        // may send any Content-Type — nothing else is pinned into the URL.
        let key = self.mint_key(name);
        let upload_url = self.presign("PUT", &key, UPLOAD_URL_EXPIRES);
        Ok(UploadInit {
            asset_id: key.clone(),
            upload_url,
            cdn_url: self.served_url(&key),
            expires_at: now_epoch_secs() + UPLOAD_URL_EXPIRES,
        })
    }

    fn confirm_upload(&self, asset_id: &str) -> Result<StoredFile, FileStorageError> {
        // Verify the client's direct-to-S3 PUT actually landed, and read back
        // the authoritative size, via a signed HEAD.
        let url = self.presign("HEAD", asset_id, OP_URL_EXPIRES);
        let resp = self
            .agent()
            .request("HEAD", &url)
            .call()
            .map_err(|e| match &e {
                ureq::Error::Status(404, _) => s3_err(
                    "NOT_FOUND",
                    "Upload did not complete — no object at the expected key",
                ),
                _ => s3_err("S3_HEAD_FAILED", e),
            })?;
        let size = resp
            .header("Content-Length")
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(0);
        Ok(StoredFile {
            id: asset_id.to_string(),
            url: self.served_url(asset_id),
            size,
        })
    }

    fn store(
        &self,
        name: &str,
        content: &[u8],
        content_type: &str,
    ) -> Result<StoredFile, FileStorageError> {
        // Server-side upload for callers holding bytes already (jobs, fixtures,
        // codegen). Presign a PUT and push the bytes ourselves.
        let key = self.mint_key(name);
        let url = self.presign("PUT", &key, OP_URL_EXPIRES);
        self.agent()
            .put(&url)
            .set("Content-Type", content_type)
            .send_bytes(content)
            .map_err(|e| s3_err("S3_PUT_FAILED", e))?;
        Ok(StoredFile {
            id: key.clone(),
            url: self.served_url(&key),
            size: content.len(),
        })
    }

    fn get(&self, id: &str) -> Result<Vec<u8>, FileStorageError> {
        // Rarely hit: the browser path 302-redirects via `direct_url`. Only an
        // internal caller that needs the actual bytes reaches here.
        let url = self.presign("GET", id, OP_URL_EXPIRES);
        let resp = self.agent().get(&url).call().map_err(|e| match &e {
            ureq::Error::Status(404, _) => s3_err("NOT_FOUND", "File not found"),
            _ => s3_err("S3_GET_FAILED", e),
        })?;
        let mut buf = Vec::new();
        resp.into_reader()
            .take(MAX_SERVER_GET_BYTES + 1)
            .read_to_end(&mut buf)
            .map_err(|e| s3_err("S3_GET_READ", e))?;
        if buf.len() as u64 > MAX_SERVER_GET_BYTES {
            return Err(s3_err(
                "S3_OBJECT_TOO_LARGE",
                format!("Object exceeds server-side read cap of {MAX_SERVER_GET_BYTES} bytes"),
            ));
        }
        Ok(buf)
    }

    fn delete(&self, id: &str) -> Result<bool, FileStorageError> {
        // S3 DeleteObject is idempotent (204 whether or not the key existed),
        // so we can't distinguish absent from removed. The router owner-checks
        // (via `owner_of`) before calling us on private buckets, so reaching
        // here means the object existed — report `true`. Best-effort tear down
        // the owner sidecar so it can't outlive the object.
        let url = self.presign("DELETE", id, OP_URL_EXPIRES);
        self.agent()
            .request("DELETE", &url)
            .call()
            .map_err(|e| s3_err("S3_DELETE_FAILED", e))?;
        if !self.is_public() {
            let owner_url = self.presign("DELETE", &owner_key(id), OP_URL_EXPIRES);
            let _ = self.agent().request("DELETE", &owner_url).call();
        }
        Ok(true)
    }

    fn direct_url(&self, id: &str) -> Result<Option<String>, FileStorageError> {
        // Always Some, so the router 302s and bytes bypass pylon:
        //  - public bucket → the plain public/CDN URL;
        //  - private bucket → a short-lived presigned GET URL.
        // The router runs the owner check (private buckets) BEFORE it calls
        // this, so a private object is only ever redirected to for its owner.
        match &self.cfg.public_url_prefix {
            Some(prefix) => Ok(Some(format!("{}/{}", prefix.trim_end_matches('/'), id))),
            None => Ok(Some(self.presign("GET", id, DOWNLOAD_URL_EXPIRES))),
        }
    }

    fn record_owner(&self, id: &str, owner: &FileOwner) -> Result<(), FileStorageError> {
        // Public buckets don't enforce ownership (see module docs), so skip the
        // sidecar write entirely — nothing reads it.
        if self.is_public() {
            return Ok(());
        }
        let body = serde_json::to_vec(owner).map_err(|e| s3_err("OWNERSHIP_ENCODE_FAILED", e))?;
        let url = self.presign("PUT", &owner_key(id), OP_URL_EXPIRES);
        self.agent()
            .put(&url)
            .set("Content-Type", "application/json")
            .send_bytes(&body)
            .map_err(|e| s3_err("S3_OWNER_WRITE_FAILED", e))?;
        Ok(())
    }

    fn owner_of(&self, id: &str) -> Result<Option<FileOwner>, FileStorageError> {
        if self.is_public() {
            return Ok(None);
        }
        let url = self.presign("GET", &owner_key(id), OP_URL_EXPIRES);
        match self.agent().get(&url).call() {
            Ok(resp) => {
                let mut buf = Vec::new();
                resp.into_reader()
                    .take(64 * 1024)
                    .read_to_end(&mut buf)
                    .map_err(|e| s3_err("S3_OWNER_READ", e))?;
                let owner: FileOwner = serde_json::from_slice(&buf)
                    .map_err(|e| s3_err("OWNERSHIP_DECODE_FAILED", e))?;
                Ok(Some(owner))
            }
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(e) => Err(s3_err("S3_OWNER_READ_FAILED", e)),
        }
    }

    fn requires_owner_check(&self) -> bool {
        // Private buckets: pylon is the only access gate, so enforce ownership.
        // Public buckets: objects are served straight from the CDN, so the
        // check would be theatre (anyone with the URL reads it anyway).
        !self.is_public()
    }
}

// ---------------------------------------------------------------------------
// SigV4 presigning — the one signing path
// ---------------------------------------------------------------------------

struct PresignParams<'a> {
    method: &'a str,
    scheme: &'a str,
    host: &'a str,
    /// Already percent-encoded request path (see [`S3FileStorage::canonical_uri`]).
    canonical_uri: &'a str,
    region: &'a str,
    access_key: &'a str,
    secret_key: &'a str,
    /// `YYYYMMDDTHHMMSSZ`.
    amz_date: &'a str,
    /// `YYYYMMDD`.
    date_stamp: &'a str,
    expires: i64,
    /// STS/temporary-credential session token, if any. Added to the signed
    /// canonical query as `X-Amz-Security-Token`.
    session_token: Option<&'a str>,
}

/// Compute a SigV4 presigned URL. Returns `(url, signature)`; the signature is
/// returned separately so tests can assert it against AWS's published vector.
///
/// Follows the AWS "Query String Request Authentication" algorithm: the
/// `X-Amz-*` query parameters are signed (host is the only signed header, the
/// payload hash is the literal `UNSIGNED-PAYLOAD`), and `X-Amz-Signature` is
/// appended to the canonical query string afterwards.
fn presign_url(p: &PresignParams) -> (String, String) {
    let credential_scope = format!("{}/{}/{}/aws4_request", p.date_stamp, p.region, SERVICE);
    let credential = format!("{}/{}", p.access_key, credential_scope);

    // Canonical query params, each key/value percent-encoded. A session token
    // (temporary/STS creds) is a *signed* parameter, so it goes in here before
    // signing — unlike X-Amz-Signature, which is appended afterwards.
    let mut query_params: Vec<(&str, String)> = vec![
        ("X-Amz-Algorithm", ALGORITHM.to_string()),
        ("X-Amz-Credential", credential),
        ("X-Amz-Date", p.amz_date.to_string()),
        ("X-Amz-Expires", p.expires.to_string()),
        ("X-Amz-SignedHeaders", "host".to_string()),
    ];
    if let Some(token) = p.session_token {
        query_params.push(("X-Amz-Security-Token", token.to_string()));
    }
    // SigV4 requires the canonical query sorted by encoded key. Every key here
    // is ASCII with no encodable bytes, so raw-key ordering is identical.
    query_params.sort_by(|a, b| a.0.cmp(b.0));
    let canonical_query = query_params
        .iter()
        .map(|(k, v)| format!("{}={}", uri_encode(k, true), uri_encode(v, true)))
        .collect::<Vec<_>>()
        .join("&");

    // Only `host` is signed. canonical_headers ends in a newline, so the
    // canonical request has the required blank line before signed_headers.
    let canonical_headers = format!("host:{}\n", p.host);
    let signed_headers = "host";
    let payload_hash = "UNSIGNED-PAYLOAD";

    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        p.method, p.canonical_uri, canonical_query, canonical_headers, signed_headers, payload_hash
    );

    let string_to_sign = format!(
        "{}\n{}\n{}\n{}",
        ALGORITHM,
        p.amz_date,
        credential_scope,
        sha256_hex(canonical_request.as_bytes())
    );

    let signing_key = derive_signing_key(p.secret_key, p.date_stamp, p.region);
    let signature = hex_lower(&hmac(&signing_key, string_to_sign.as_bytes()));

    let url = format!(
        "{}://{}{}?{}&X-Amz-Signature={}",
        p.scheme, p.host, p.canonical_uri, canonical_query, signature
    );
    (url, signature)
}

/// AWS4 signing key: HMAC chain over date → region → service → `aws4_request`.
fn derive_signing_key(secret: &str, date_stamp: &str, region: &str) -> Vec<u8> {
    let k_date = hmac(format!("AWS4{secret}").as_bytes(), date_stamp.as_bytes());
    let k_region = hmac(&k_date, region.as_bytes());
    let k_service = hmac(&k_region, SERVICE.as_bytes());
    hmac(&k_service, b"aws4_request")
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac =
        <HmacSha256 as Mac>::new_from_slice(key).expect("HMAC accepts a key of any length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn sha256_hex(data: &[u8]) -> String {
    hex_lower(&Sha256::digest(data))
}

const HEX_LOWER: &[u8; 16] = b"0123456789abcdef";
const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";

fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX_LOWER[(b >> 4) as usize] as char);
        s.push(HEX_LOWER[(b & 0x0f) as usize] as char);
    }
    s
}

/// RFC 3986 percent-encoding as SigV4 requires: unreserved bytes pass through,
/// everything else becomes `%XX` with UPPERCASE hex. `/` is left literal in
/// path context (`encode_slash = false`) and encoded in query context.
fn uri_encode(s: &str, encode_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b'/' if !encode_slash => out.push('/'),
            _ => {
                out.push('%');
                out.push(HEX_UPPER[(b >> 4) as usize] as char);
                out.push(HEX_UPPER[(b & 0x0f) as usize] as char);
            }
        }
    }
    out
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Split `https://host[:port]/maybe/path` into `(scheme, host[:port])`,
/// discarding any path. Defaults to `https` when no scheme is present.
fn split_scheme_host(endpoint: &str) -> (String, String) {
    let (scheme, rest) = match endpoint.split_once("://") {
        Some((s, r)) => (s.to_string(), r),
        None => ("https".to_string(), endpoint),
    };
    let host = rest.split('/').next().unwrap_or(rest).trim_end_matches('/');
    (scheme, host.to_string())
}

fn s3_err(code: &str, e: impl std::fmt::Display) -> FileStorageError {
    FileStorageError {
        code: code.into(),
        message: e.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Time — epoch → civil date, no chrono (an optional dep here)
// ---------------------------------------------------------------------------

fn now_epoch_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn now_timestamps() -> (String, String) {
    format_timestamps(now_epoch_secs())
}

/// Format epoch seconds as SigV4's `(amz_date, date_stamp)` in UTC.
fn format_timestamps(secs: i64) -> (String, String) {
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    (
        format!("{y:04}{m:02}{d:02}T{h:02}{mi:02}{s:02}Z"),
        format!("{y:04}{m:02}{d:02}"),
    )
}

/// Days-since-Unix-epoch → `(year, month, day)`. Howard Hinnant's civil-from-days.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(endpoint: Option<&str>, public: Option<&str>) -> S3Config {
        S3Config {
            bucket: "my-bucket".into(),
            region: "us-east-1".into(),
            endpoint: endpoint.map(|s| s.to_string()),
            access_key: "AKIAEXAMPLE".into(),
            secret_key: "secret".into(),
            session_token: None,
            public_url_prefix: public.map(|s| s.to_string()),
            folder: None,
        }
    }

    /// The canonical AWS SigV4 presigned-URL example
    /// (docs.aws.amazon.com "Signature Calculations — Query Parameters").
    /// If this reproduces AWS's published signature byte-for-byte, the whole
    /// signing path (canonical request, string-to-sign, key derivation,
    /// encoding) is correct.
    #[test]
    fn sigv4_matches_aws_published_vector() {
        let params = PresignParams {
            method: "GET",
            scheme: "https",
            host: "examplebucket.s3.amazonaws.com",
            canonical_uri: "/test.txt",
            region: "us-east-1",
            access_key: "AKIAIOSFODNN7EXAMPLE",
            secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            amz_date: "20130524T000000Z",
            date_stamp: "20130524",
            expires: 86400,
            session_token: None,
        };
        let (url, signature) = presign_url(&params);
        assert_eq!(
            signature, "aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404",
            "SigV4 signature must match AWS's published example"
        );
        assert!(url.contains(
            "X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20130524%2Fus-east-1%2Fs3%2Faws4_request"
        ));
        assert!(url.contains(
            "X-Amz-Signature=aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404"
        ));
        assert!(url.starts_with("https://examplebucket.s3.amazonaws.com/test.txt?"));
    }

    #[test]
    fn session_token_is_signed_and_encoded() {
        // A temporary-credential token must appear as a signed query param
        // (URI-encoded, incl. `/` and `+`), sorted before X-Amz-SignedHeaders,
        // and must change the signature vs the token-free request.
        let base = PresignParams {
            method: "GET",
            scheme: "https",
            host: "examplebucket.s3.amazonaws.com",
            canonical_uri: "/test.txt",
            region: "us-east-1",
            access_key: "AKIAIOSFODNN7EXAMPLE",
            secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            amz_date: "20130524T000000Z",
            date_stamp: "20130524",
            expires: 86400,
            session_token: None,
        };
        let (_, sig_none) = presign_url(&base);
        let with_token = PresignParams {
            session_token: Some("tok/en+abc=="),
            ..base
        };
        let (url, sig_token) = presign_url(&with_token);
        assert_ne!(
            sig_none, sig_token,
            "the token must be part of the signature"
        );
        assert!(url.contains("X-Amz-Security-Token=tok%2Fen%2Babc%3D%3D"));
        // Sorted: Security-Token precedes SignedHeaders in the canonical query.
        let st = url.find("X-Amz-Security-Token=").unwrap();
        let sh = url.find("X-Amz-SignedHeaders=").unwrap();
        assert!(st < sh, "Security-Token must sort before SignedHeaders");
    }

    #[test]
    fn timestamps_format_utc() {
        // 1369353600 = 2013-05-24T00:00:00Z (matches the vector above).
        assert_eq!(
            format_timestamps(1_369_353_600),
            ("20130524T000000Z".to_string(), "20130524".to_string())
        );
        // 1672531199 = 2022-12-31T23:59:59Z — exercises the time-of-day path.
        assert_eq!(
            format_timestamps(1_672_531_199),
            ("20221231T235959Z".to_string(), "20221231".to_string())
        );
    }

    #[test]
    fn uri_encode_matches_sigv4_rules() {
        assert_eq!(uri_encode("a/b c", false), "a/b%20c"); // slash kept in path
        assert_eq!(uri_encode("a/b c", true), "a%2Fb%20c"); // slash encoded in query
        assert_eq!(uri_encode("-_.~AZ09", true), "-_.~AZ09"); // unreserved untouched
    }

    #[test]
    fn virtual_hosted_vs_path_style_urls() {
        // No endpoint → virtual-hosted host, key at the root.
        let vh = S3FileStorage::new(cfg(None, None));
        assert_eq!(vh.host, "my-bucket.s3.us-east-1.amazonaws.com");
        assert!(!vh.path_style);
        assert_eq!(vh.canonical_uri("file_abc"), "/file_abc");

        // Custom endpoint (Tigris/R2/MinIO) → path-style, bucket in the path.
        let ps = S3FileStorage::new(cfg(Some("https://fly.storage.tigris.dev"), None));
        assert_eq!(ps.host, "fly.storage.tigris.dev");
        assert!(ps.path_style);
        assert_eq!(ps.canonical_uri("file_abc"), "/my-bucket/file_abc");

        // MinIO over http with a port — scheme + port preserved for the
        // signed host header.
        let minio = S3FileStorage::new(cfg(Some("http://localhost:9000"), None));
        assert_eq!(minio.scheme, "http");
        assert_eq!(minio.host, "localhost:9000");
    }

    #[test]
    fn private_bucket_enforces_ownership_public_does_not() {
        let private = S3FileStorage::new(cfg(None, None));
        assert!(private.requires_owner_check());
        // Public bucket: ownership is skipped; owner_of is a no-op None, and
        // direct_url is the plain public URL (no signing round-trip).
        let public = S3FileStorage::new(cfg(None, Some("https://cdn.example.com")));
        assert!(!public.requires_owner_check());
        assert!(public.owner_of("file_x").unwrap().is_none());
        assert_eq!(
            public.direct_url("file_x").unwrap(),
            Some("https://cdn.example.com/file_x".to_string())
        );
    }

    #[test]
    fn served_url_shape() {
        // Private → pylon-mediated endpoint (owner-checked, presign redirect).
        assert_eq!(
            S3FileStorage::new(cfg(None, None)).served_url("file_x"),
            "/api/files/file_x"
        );
        // Public → direct CDN URL, trailing slash normalised.
        assert_eq!(
            S3FileStorage::new(cfg(None, Some("https://cdn.example.com/"))).served_url("file_x"),
            "https://cdn.example.com/file_x"
        );
    }

    #[test]
    fn private_direct_url_is_presigned_get() {
        let private = S3FileStorage::new(cfg(None, None));
        let url = private.direct_url("file_x").unwrap().unwrap();
        assert!(url.starts_with("https://my-bucket.s3.us-east-1.amazonaws.com/file_x?"));
        assert!(url.contains("X-Amz-Algorithm=AWS4-HMAC-SHA256"));
        assert!(url.contains("X-Amz-Signature="));
        assert!(url.contains("X-Amz-Expires=300"));
    }

    #[test]
    fn mint_key_is_unguessable_and_prefixed() {
        let s = S3FileStorage::new(cfg(None, None));
        let k1 = s.mint_key("photo.jpg");
        let k2 = s.mint_key("photo.jpg");
        assert!(k1.starts_with("file_"));
        assert!(k1.ends_with("_photo.jpg"));
        assert_ne!(k1, k2, "each key carries a fresh CSPRNG nonce");

        // Folder prefix applied when configured.
        let mut c = cfg(None, None);
        c.folder = Some("avatars".into());
        let folded = S3FileStorage::new(c).mint_key("a.png");
        assert!(folded.starts_with("avatars/file_"));
    }

    #[test]
    fn sanitize_name_strips_path_separators() {
        assert_eq!(sanitize_name("../../etc/passwd"), ".._.._etc_passwd");
        assert_eq!(sanitize_name("ok-name_1.png"), "ok-name_1.png");
    }

    #[test]
    fn owner_key_suffix() {
        assert_eq!(owner_key("file_abc"), "file_abc.owner");
    }
}
