import SwiftUI

struct ContentView: View {
    @EnvironmentObject var session: AppSession

    var body: some View {
        if session.signedIn, let engine = session.engine {
            TodoListView(engine: engine, client: session.client)
        } else {
            SignInView()
        }
    }
}
