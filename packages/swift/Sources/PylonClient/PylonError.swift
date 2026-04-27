import Foundation

/// Errors thrown by the Pylon client.
public enum PylonError: Error, Sendable {
    /// Non-2xx HTTP response. `code` mirrors the Pylon error code from the
    /// JSON body when present (e.g. `RESYNC_REQUIRED`, `RATE_LIMITED`).
    case http(status: Int, code: String?, message: String?)

    /// Network or transport failure (timeout, DNS, TLS).
    case transport(any Error)

    /// Server returned a body that didn't match the expected shape.
    case decoding(any Error)

    /// Caller misuse — invalid argument, no session, etc.
    case invalidArgument(String)

    /// Underlying I/O — file ops, persistence, etc.
    case io(any Error)
}

extension PylonError: CustomStringConvertible {
    public var description: String {
        switch self {
        case .http(let status, let code, let message):
            let codePart = code.map { " \($0)" } ?? ""
            let msgPart = message.map { " — \($0)" } ?? ""
            return "PylonError.http(\(status))\(codePart)\(msgPart)"
        case .transport(let e):
            return "PylonError.transport(\(e))"
        case .decoding(let e):
            return "PylonError.decoding(\(e))"
        case .invalidArgument(let s):
            return "PylonError.invalidArgument(\(s))"
        case .io(let e):
            return "PylonError.io(\(e))"
        }
    }
}

extension PylonError {
    /// HTTP status, if this is a `.http` error.
    public var httpStatus: Int? {
        if case .http(let status, _, _) = self { return status }
        return nil
    }

    /// Server-supplied error code, if this is a `.http` error and the body
    /// included one.
    public var code: String? {
        if case .http(_, let code, _) = self { return code }
        return nil
    }
}
