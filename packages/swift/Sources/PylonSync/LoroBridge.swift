import Foundation
import Loro
import PylonClient

/// Wire format for Pylon's binary CRDT broadcast frames.
///
/// Mirror of `packages/loro/src/wire.ts` (and `crates/router`'s
/// `encode_crdt_frame`). Frame layout:
///
///   `[type: u8] [entity_len: u16 BE] [entity utf8]`
///   `[row_id_len: u16 BE] [row_id utf8] [payload bytes]`
public struct PylonCrdtFrame: Sendable {
    public enum Kind: UInt8, Sendable {
        case snapshot = 0x10
        case update = 0x11
    }
    public let kind: Kind
    public let entity: String
    public let rowId: String
    public let payload: Data
}

public enum PylonCrdtWire {
    /// Decode a binary frame produced by Pylon's CRDT broadcast. Returns
    /// `nil` on framing errors so the receiver can drop unknown frames
    /// without crashing the WS loop.
    public static func decode(_ bytes: Data) -> PylonCrdtFrame? {
        // Header is at minimum: type(1) + entity_len(2) + row_id_len(2) = 5 bytes.
        guard bytes.count >= 5 else { return nil }
        let base = bytes.startIndex
        guard let kind = PylonCrdtFrame.Kind(rawValue: bytes[base]) else { return nil }

        let entityLen = Int(readU16BE(bytes, at: 1))
        let entityStart = 3
        let entityEnd = entityStart + entityLen
        guard entityEnd + 2 <= bytes.count else { return nil }

        let rowIdLen = Int(readU16BE(bytes, at: entityEnd))
        let rowIdStart = entityEnd + 2
        let rowIdEnd = rowIdStart + rowIdLen
        guard rowIdEnd <= bytes.count else { return nil }

        guard let entity = String(data: bytes[(base + entityStart)..<(base + entityEnd)], encoding: .utf8),
              let rowId = String(data: bytes[(base + rowIdStart)..<(base + rowIdEnd)], encoding: .utf8) else {
            return nil
        }

        let payload = bytes.subdata(in: (base + rowIdEnd)..<bytes.endIndex)
        return PylonCrdtFrame(kind: kind, entity: entity, rowId: rowId, payload: payload)
    }

    private static func readU16BE(_ bytes: Data, at relativeOffset: Int) -> UInt16 {
        let i = bytes.startIndex + relativeOffset
        let hi = UInt16(bytes[i])
        let lo = UInt16(bytes[i + 1])
        return (hi << 8) | lo
    }

    /// Encode a frame in the same wire format. Useful for tests and for
    /// sending CRDT updates back to peers when Pylon adds client-side
    /// publishing.
    public static func encode(_ frame: PylonCrdtFrame) -> Data {
        var out = Data()
        out.append(frame.kind.rawValue)
        let entityBytes = Data(frame.entity.utf8)
        appendU16BE(UInt16(entityBytes.count), to: &out)
        out.append(entityBytes)
        let rowIdBytes = Data(frame.rowId.utf8)
        appendU16BE(UInt16(rowIdBytes.count), to: &out)
        out.append(rowIdBytes)
        out.append(frame.payload)
        return out
    }

    private static func appendU16BE(_ value: UInt16, to data: inout Data) {
        data.append(UInt8(value >> 8))
        data.append(UInt8(value & 0xFF))
    }
}

/// Per-row Loro document handle. Owns a `LoroDoc` and applies incoming
/// CRDT frames as they arrive over the sync engine's binary channel.
///
/// Use `attach(to:)` to wire it into a `SyncEngine`. The engine handles
/// the subscribe/unsubscribe handshake; this type just owns the document
/// and routes payloads.
public final class PylonLoroDoc: @unchecked Sendable {
    public let entity: String
    public let rowId: String
    public let doc: LoroDoc

    private var unsubscribe: (() -> Void)?
    private var unsubscribeCrdt: (() async -> Void)?

    public init(entity: String, rowId: String, doc: LoroDoc = LoroDoc()) {
        self.entity = entity
        self.rowId = rowId
        self.doc = doc
    }

    deinit {
        unsubscribe?()
    }

    /// Wire this doc into a `SyncEngine` so CRDT updates land automatically.
    /// Call `detach()` (or let the doc go out of scope) to tear down.
    public func attach(to engine: SyncEngine) async {
        let entity = self.entity
        let rowId = self.rowId
        let doc = self.doc
        let myEntity = entity
        let myRowId = rowId
        let cancel = await engine.onBinaryFrame { [weak self] data in
            guard self != nil else { return }
            guard let frame = PylonCrdtWire.decode(data) else { return }
            guard frame.entity == myEntity, frame.rowId == myRowId else { return }
            do {
                switch frame.kind {
                case .snapshot, .update:
                    _ = try doc.import(bytes: frame.payload)
                }
            } catch {
                // Loro import failed — the frame was malformed or out of
                // order in a way we can't recover from. Drop it; the next
                // snapshot frame should resync.
            }
        }
        unsubscribe = cancel
        await engine.subscribeCrdt(entity: entity, rowId: rowId)
        unsubscribeCrdt = { [weak engine] in
            guard let engine else { return }
            await engine.unsubscribeCrdt(entity: entity, rowId: rowId)
        }
    }

    public func detach() async {
        unsubscribe?()
        unsubscribe = nil
        await unsubscribeCrdt?()
        unsubscribeCrdt = nil
    }
}
