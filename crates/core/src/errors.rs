//! Centralized error code constants.
//!
//! Using string constants (instead of an enum) preserves the existing
//! `{code: String, message: String}` wire shape while giving Rust callers
//! and clients a single source of truth for the vocabulary.
//!
//! The TypeScript codegen emits these as a typed union so clients can
//! exhaustively match on error codes.

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

pub const AUTH_REQUIRED: &str = "AUTH_REQUIRED";
pub const AUTH_UPGRADE_FAILED: &str = "UPGRADE_FAILED";
pub const INVALID_CODE: &str = "INVALID_CODE";
pub const OAUTH_INVALID_STATE: &str = "OAUTH_INVALID_STATE";
pub const OAUTH_TOKEN_EXCHANGE_FAILED: &str = "OAUTH_TOKEN_EXCHANGE_FAILED";
pub const PROVIDER_NOT_FOUND: &str = "PROVIDER_NOT_FOUND";
pub const SESSION_EXPIRED: &str = "SESSION_EXPIRED";
pub const SESSION_NOT_FOUND: &str = "SESSION_NOT_FOUND";
pub const FORBIDDEN: &str = "FORBIDDEN";
pub const POLICY_DENIED: &str = "POLICY_DENIED";
pub const UNAUTHORIZED: &str = "UNAUTHORIZED";

// ---------------------------------------------------------------------------
// Request validation
// ---------------------------------------------------------------------------

pub const INVALID_JSON: &str = "INVALID_JSON";
pub const INVALID_ARGS: &str = "INVALID_ARGS";
pub const INVALID_COLUMN: &str = "INVALID_COLUMN";
pub const INVALID_QUERY: &str = "INVALID_QUERY";
pub const INVALID_DATA: &str = "INVALID_DATA";
pub const INVALID_FILE_ID: &str = "INVALID_FILE_ID";
pub const MISSING_EMAIL: &str = "MISSING_EMAIL";
pub const MISSING_USER_ID: &str = "MISSING_USER_ID";
pub const MISSING_CODE: &str = "MISSING_CODE";
pub const MISSING_FIELD: &str = "MISSING_FIELD";
pub const MISSING_ROOM: &str = "MISSING_ROOM";
pub const MISSING_TOPIC: &str = "MISSING_TOPIC";
pub const MISSING_NAME: &str = "MISSING_NAME";
pub const MISSING_OPERATIONS: &str = "MISSING_OPERATIONS";
pub const PAYLOAD_TOO_LARGE: &str = "PAYLOAD_TOO_LARGE";

// ---------------------------------------------------------------------------
// Data / lookup
// ---------------------------------------------------------------------------

pub const NOT_FOUND: &str = "NOT_FOUND";
pub const ENTITY_NOT_FOUND: &str = "ENTITY_NOT_FOUND";
pub const ACTION_NOT_FOUND: &str = "ACTION_NOT_FOUND";
pub const FN_NOT_FOUND: &str = "FN_NOT_FOUND";
pub const FILE_NOT_FOUND: &str = "FILE_NOT_FOUND";
pub const SHARD_NOT_FOUND: &str = "SHARD_NOT_FOUND";
pub const RELATION_NOT_FOUND: &str = "RELATION_NOT_FOUND";

// ---------------------------------------------------------------------------
// Database / internal
// ---------------------------------------------------------------------------

pub const QUERY_FAILED: &str = "QUERY_FAILED";
pub const INSERT_FAILED: &str = "INSERT_FAILED";
pub const UPDATE_FAILED: &str = "UPDATE_FAILED";
pub const DELETE_FAILED: &str = "DELETE_FAILED";
pub const SCHEMA_INIT_FAILED: &str = "SCHEMA_INIT_FAILED";
pub const LOCK_FAILED: &str = "LOCK_FAILED";
pub const NESTED_TRANSACTION: &str = "NESTED_TRANSACTION";
pub const EXPORT_FAILED: &str = "EXPORT_FAILED";
pub const NOT_SUPPORTED: &str = "NOT_SUPPORTED";
pub const NOT_IMPLEMENTED: &str = "NOT_IMPLEMENTED";
pub const NOT_AVAILABLE: &str = "NOT_AVAILABLE";

// ---------------------------------------------------------------------------
// Transport / platform
// ---------------------------------------------------------------------------

pub const RATE_LIMITED: &str = "RATE_LIMITED";
pub const METHOD_NOT_ALLOWED: &str = "METHOD_NOT_ALLOWED";
pub const PROTOCOL_ERROR: &str = "PROTOCOL_ERROR";
pub const RUNNER_EXITED: &str = "RUNNER_EXITED";
pub const RUNNER_NOT_STARTED: &str = "RUNNER_NOT_STARTED";
pub const IO_ERROR: &str = "IO_ERROR";
pub const AI_NOT_CONFIGURED: &str = "AI_NOT_CONFIGURED";
pub const AI_REQUEST_FAILED: &str = "AI_REQUEST_FAILED";
pub const EMAIL_SEND_FAILED: &str = "EMAIL_SEND_FAILED";

// ---------------------------------------------------------------------------
// Workflows / jobs / shards
// ---------------------------------------------------------------------------

pub const WORKFLOW_START_FAILED: &str = "WORKFLOW_START_FAILED";
pub const WORKFLOW_ADVANCE_FAILED: &str = "WORKFLOW_ADVANCE_FAILED";
pub const WORKFLOW_EVENT_FAILED: &str = "WORKFLOW_EVENT_FAILED";
pub const WORKFLOW_CANCEL_FAILED: &str = "WORKFLOW_CANCEL_FAILED";
pub const INPUT_REJECTED: &str = "INPUT_REJECTED";
pub const SUBSCRIBE_FAILED: &str = "SUBSCRIBE_FAILED";
pub const SHARDS_NOT_AVAILABLE: &str = "SHARDS_NOT_AVAILABLE";

/// All error codes, ordered for codegen.
pub const ALL_CODES: &[&str] = &[
    AUTH_REQUIRED,
    AUTH_UPGRADE_FAILED,
    INVALID_CODE,
    OAUTH_INVALID_STATE,
    OAUTH_TOKEN_EXCHANGE_FAILED,
    PROVIDER_NOT_FOUND,
    SESSION_EXPIRED,
    SESSION_NOT_FOUND,
    FORBIDDEN,
    POLICY_DENIED,
    UNAUTHORIZED,
    INVALID_JSON,
    INVALID_ARGS,
    INVALID_COLUMN,
    INVALID_QUERY,
    INVALID_DATA,
    INVALID_FILE_ID,
    MISSING_EMAIL,
    MISSING_USER_ID,
    MISSING_CODE,
    MISSING_FIELD,
    MISSING_ROOM,
    MISSING_TOPIC,
    MISSING_NAME,
    MISSING_OPERATIONS,
    PAYLOAD_TOO_LARGE,
    NOT_FOUND,
    ENTITY_NOT_FOUND,
    ACTION_NOT_FOUND,
    FN_NOT_FOUND,
    FILE_NOT_FOUND,
    SHARD_NOT_FOUND,
    RELATION_NOT_FOUND,
    QUERY_FAILED,
    INSERT_FAILED,
    UPDATE_FAILED,
    DELETE_FAILED,
    SCHEMA_INIT_FAILED,
    LOCK_FAILED,
    NESTED_TRANSACTION,
    EXPORT_FAILED,
    NOT_SUPPORTED,
    NOT_IMPLEMENTED,
    NOT_AVAILABLE,
    RATE_LIMITED,
    METHOD_NOT_ALLOWED,
    PROTOCOL_ERROR,
    RUNNER_EXITED,
    RUNNER_NOT_STARTED,
    IO_ERROR,
    AI_NOT_CONFIGURED,
    AI_REQUEST_FAILED,
    EMAIL_SEND_FAILED,
    WORKFLOW_START_FAILED,
    WORKFLOW_ADVANCE_FAILED,
    WORKFLOW_EVENT_FAILED,
    WORKFLOW_CANCEL_FAILED,
    INPUT_REJECTED,
    SUBSCRIBE_FAILED,
    SHARDS_NOT_AVAILABLE,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_codes_nonempty() {
        assert!(!ALL_CODES.is_empty());
    }

    #[test]
    fn all_codes_unique() {
        let mut seen = std::collections::HashSet::new();
        for code in ALL_CODES {
            assert!(seen.insert(*code), "duplicate code: {code}");
        }
    }

    #[test]
    fn all_codes_uppercase_snake() {
        for code in ALL_CODES {
            assert!(
                code.chars()
                    .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit()),
                "code not UPPER_SNAKE_CASE: {code}"
            );
        }
    }
}
