import Foundation
import PylonClient
import PylonSync

/// The signed-in state of the app. Owns the Pylon client and, while a user
/// is signed in, the sync engine and the live social data built on it.
@MainActor
final class AppModel: ObservableObject {
	enum Phase: Equatable {
		case launching
		case signedOut
		case needsProfile
		case ready
	}

	@Published private(set) var phase: Phase = .launching
	@Published private(set) var social: SocialStore?

	let client: PylonClient
	let media: MediaService
	let baseURL: URL
	private var engine: SyncEngine?
	private var engineStart: Task<Void, Never>?

	init() {
		// The simulator reaches the Mac's localhost. On a device, set
		// PYLON_BASE_URL in the scheme to the Mac's LAN address or to your
		// deployed API.
		let raw = ProcessInfo.processInfo.environment["PYLON_BASE_URL"] ?? "http://localhost:4321"
		guard let baseURL = URL(string: raw) else {
			fatalError("Invalid PYLON_BASE_URL: \(raw)")
		}
		self.baseURL = baseURL
		client = PylonClient(baseURL: baseURL, appName: "__APP_NAME_SNAKE__")
		media = MediaService(baseURL: baseURL, client: client)
	}

	func boot() async {
		guard phase == .launching else { return }
		// Demo content for a new install. The server writes it once and
		// returns at once after that. Delete this call with functions/seedDemo.ts.
		_ = try? await client.callFn("seedDemo", args: EmptyArgs(), as: SeedResult.self)

		guard await client.currentToken() != nil else {
			phase = .signedOut
			return
		}
		do {
			let session = try await client.me()
			guard let userId = session.userId else {
				await clearLocalState()
				return
			}
			await startSession(userId: userId)
		} catch let error as PylonError where error.httpStatus == 401 {
			await clearLocalState()
		} catch {
			// Offline launch: keep the stored session and show cached rows.
			if let userId = UserDefaults.standard.string(forKey: Self.lastUserIdKey) {
				await startSession(userId: userId)
			} else {
				phase = .signedOut
			}
		}
	}

	func signIn(email: String, password: String) async throws {
		let session = try await client.signInWithPassword(email: email, password: password)
		await startSession(userId: session.user_id)
	}

	func register(email: String, password: String) async throws {
		let session = try await client.registerWithPassword(email: email, password: password)
		await startSession(userId: session.user_id)
	}

	func signOut() async {
		try? await client.logout()
		await clearLocalState()
	}

	/// Called by the profile setup screen once the Profile row exists.
	func profileCreated(_ profile: Profile) {
		social?.myProfileId = profile.id
		UserDefaults.standard.set(profile.id, forKey: Self.profileIdKey(profile.userId))
		phase = .ready
	}

	/// Forgets the session and everything cached for it: the token, the
	/// sync engine, the on-disk replica, and the remembered profile id.
	private func clearLocalState() async {
		await client.clearSession()
		engineStart?.cancel()
		await engineStart?.value
		engineStart = nil
		if let engine {
			await engine.stop()
			await engine.resetReplica(wipeMutations: true)
		}
		engine = nil
		social = nil
		Self.deleteReplicaFiles()
		if let userId = UserDefaults.standard.string(forKey: Self.lastUserIdKey) {
			UserDefaults.standard.removeObject(forKey: Self.profileIdKey(userId))
		}
		UserDefaults.standard.removeObject(forKey: Self.lastUserIdKey)
		phase = .signedOut
	}

	private func startSession(userId: String?) async {
		if let previous = UserDefaults.standard.string(forKey: Self.lastUserIdKey), previous != userId {
			// A different account on this device: drop the last one's replica.
			Self.deleteReplicaFiles()
		}
		if let userId { UserDefaults.standard.set(userId, forKey: Self.lastUserIdKey) }

		var config = SyncEngineConfig(baseURL: baseURL, appName: "__APP_NAME_SNAKE__")
		config.transport = .websocket
		let engine = await SyncEngine(
			config: config,
			client: client,
			persistence: Self.makePersistence()
		)
		self.engine = engine
		// start() pulls the first snapshot, then keeps a WebSocket open so
		// rows written by any other client arrive here without a refresh.
		engineStart = Task { await engine.start() }

		let social = SocialStore(engine: engine, client: client, media: media)
		do {
			let me = try await client.callFn("myProfile", args: EmptyArgs(), as: Profile?.self)
			social.myProfileId = me?.id
			if let userId {
				if let id = me?.id {
					UserDefaults.standard.set(id, forKey: Self.profileIdKey(userId))
				} else {
					UserDefaults.standard.removeObject(forKey: Self.profileIdKey(userId))
				}
			}
		} catch {
			// Offline: use this user's profile id from the last online launch.
			social.myProfileId = userId.flatMap { UserDefaults.standard.string(forKey: Self.profileIdKey($0)) }
		}
		self.social = social
		phase = social.myProfileId == nil ? .needsProfile : .ready
	}

	private static let lastUserIdKey = "lastUserId"

	private static func profileIdKey(_ userId: String) -> String { "profileId.\(userId)" }

	private static var replicaURL: URL {
		FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
			.appendingPathComponent("pylon-sync.sqlite")
	}

	/// An on-disk replica, so a relaunch shows the feed before the network answers.
	private static func makePersistence() -> SQLitePersistence? {
		try? FileManager.default.createDirectory(
			at: replicaURL.deletingLastPathComponent(),
			withIntermediateDirectories: true
		)
		return try? SQLitePersistence(path: replicaURL.path)
	}

	private static func deleteReplicaFiles() {
		for suffix in ["", "-wal", "-shm"] {
			try? FileManager.default.removeItem(atPath: replicaURL.path + suffix)
		}
	}
}
