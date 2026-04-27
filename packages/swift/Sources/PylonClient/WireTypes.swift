import Foundation

// ---------------------------------------------------------------------------
// Sync wire types
//
// Mirrors the Rust router shape (snake_case JSON). Decoded and encoded
// with the framework-default JSONDecoder/JSONEncoder (no key strategy)
// so the field names below need to match the wire exactly.
// ---------------------------------------------------------------------------

/// Server-emitted change event delivered over `/api/sync/pull` and the
/// real-time WebSocket stream.
public struct ChangeEvent: Sendable, Codable, Hashable {
    public var seq: Int64
    public var entity: String
    public var row_id: String
    public var kind: ChangeKind
    public var data: [String: JSONValue]?
    public var timestamp: String

    public init(
        seq: Int64,
        entity: String,
        row_id: String,
        kind: ChangeKind,
        data: [String: JSONValue]? = nil,
        timestamp: String = ""
    ) {
        self.seq = seq
        self.entity = entity
        self.row_id = row_id
        self.kind = kind
        self.data = data
        self.timestamp = timestamp
    }
}

public enum ChangeKind: String, Sendable, Codable, Hashable {
    case insert
    case update
    case delete
}

/// Cursor position for the sync pull stream.
public struct SyncCursor: Sendable, Codable, Hashable {
    public var last_seq: Int64

    public init(last_seq: Int64 = 0) {
        self.last_seq = last_seq
    }
}

/// `GET /api/sync/pull?since={seq}` response shape.
public struct PullResponse: Sendable, Codable {
    public var changes: [ChangeEvent]
    public var cursor: SyncCursor
    public var has_more: Bool
}

/// Outgoing mutation in `POST /api/sync/push`.
public struct ClientChange: Sendable, Codable, Hashable {
    public var entity: String
    public var row_id: String
    public var kind: ChangeKind
    public var data: [String: JSONValue]?
    /// Client-minted idempotency key. Server tracks recently-seen op_ids
    /// and returns a no-op success for replays.
    public var op_id: String?

    public init(
        entity: String,
        row_id: String,
        kind: ChangeKind,
        data: [String: JSONValue]? = nil,
        op_id: String? = nil
    ) {
        self.entity = entity
        self.row_id = row_id
        self.kind = kind
        self.data = data
        self.op_id = op_id
    }
}

public struct PushRequest: Sendable, Codable {
    public var changes: [ClientChange]
    public var client_id: String

    public init(changes: [ClientChange], client_id: String) {
        self.changes = changes
        self.client_id = client_id
    }
}

public struct PushResponse: Sendable, Codable {
    public var applied: Int
    public var errors: [String]
    public var cursor: SyncCursor
}

// ---------------------------------------------------------------------------
// Auth wire types
// ---------------------------------------------------------------------------

/// `/api/auth/me` response. `userId == nil` means anonymous.
public struct ResolvedSession: Sendable, Codable, Hashable {
    public var userId: String?
    public var tenantId: String?
    public var isAdmin: Bool
    public var roles: [String]

    public init(
        userId: String? = nil,
        tenantId: String? = nil,
        isAdmin: Bool = false,
        roles: [String] = []
    ) {
        self.userId = userId
        self.tenantId = tenantId
        self.isAdmin = isAdmin
        self.roles = roles
    }

    private enum CodingKeys: String, CodingKey {
        case userId = "user_id"
        case tenantId = "tenant_id"
        case isAdmin = "is_admin"
        case roles
    }
}

public struct SessionResponse: Sendable, Codable {
    public var session_token: String
    public var user_id: String?
}

public struct StartMagicCodeRequest: Sendable, Codable {
    public var email: String
    public init(email: String) { self.email = email }
}

public struct VerifyMagicCodeRequest: Sendable, Codable {
    public var email: String
    public var code: String
    public init(email: String, code: String) {
        self.email = email
        self.code = code
    }
}

public struct PasswordSignInRequest: Sendable, Codable {
    public var email: String
    public var password: String
    public init(email: String, password: String) {
        self.email = email
        self.password = password
    }
}

public struct OAuthGoogleRequest: Sendable, Codable {
    public var id_token: String
    public init(id_token: String) { self.id_token = id_token }
}

public struct OAuthGitHubRequest: Sendable, Codable {
    public var code: String
    public init(code: String) { self.code = code }
}

// ---------------------------------------------------------------------------
// Files wire types
// ---------------------------------------------------------------------------

public struct FileUploadResponse: Sendable, Codable {
    public var id: String
    public var url: String?
    public var size: Int?
}

// ---------------------------------------------------------------------------
// Hydration / SSR
// ---------------------------------------------------------------------------

/// Bundle handed from server to client to skip the initial pull.
public struct HydrationData: Sendable, Codable {
    public var entities: [String: [[String: JSONValue]]]
    public var cursor: SyncCursor?

    public init(
        entities: [String: [[String: JSONValue]]] = [:],
        cursor: SyncCursor? = nil
    ) {
        self.entities = entities
        self.cursor = cursor
    }
}
