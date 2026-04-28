import SwiftUI
import PylonClient
import PylonSync

@main
struct PylonTodoApp: App {
    @StateObject private var session = AppSession()

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(session)
                .task { await session.bootstrap() }
        }
    }
}

/// Owns the `PylonClient` + `SyncEngine` for the lifetime of the app.
/// Republishes auth state via `signedIn` so views can route on it.
@MainActor
final class AppSession: ObservableObject {
    @Published var signedIn = false
    @Published var bootError: String?

    let client: PylonClient
    private(set) var engine: SyncEngine?

    init() {
        // Local dev. Swap to https://your-app.pylon.app for Pylon Cloud.
        let baseURL = URL(string: "http://localhost:4321")!
        self.client = PylonClient(baseURL: baseURL)
    }

    func bootstrap() async {
        // Resolve the existing session if any; if invalid, fall through to sign-in.
        do {
            let me = try await client.me()
            signedIn = me.userId != nil
        } catch {
            signedIn = false
        }
        if signedIn {
            await startEngine()
        }
    }

    func startEngine() async {
        let dbPath = NSSearchPathForDirectoriesInDomains(.documentDirectory, .userDomainMask, true)[0] + "/pylon-todo.db"
        let persistence: SQLitePersistence?
        do {
            persistence = try SQLitePersistence(path: dbPath)
        } catch {
            bootError = "could not open local DB: \(error)"
            persistence = nil
        }

        let baseURL = await client.config.baseURL
        let cfg = SyncEngineConfig(baseURL: baseURL)
        let e = await SyncEngine(config: cfg, client: client, persistence: persistence)
        engine = e
        await e.start()
    }

    func didSignIn() async {
        signedIn = true
        await startEngine()
    }

    func signOut() async {
        try? await client.logout()
        await engine?.stop()
        engine = nil
        signedIn = false
    }
}
