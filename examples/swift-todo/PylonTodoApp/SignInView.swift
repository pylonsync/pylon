import SwiftUI
import PylonClient

struct SignInView: View {
    @EnvironmentObject var session: AppSession

    @State private var email = ""
    @State private var code = ""
    @State private var stage: Stage = .email
    @State private var error: String?
    @State private var loading = false

    enum Stage { case email, code }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Email", text: $email)
                        .textContentType(.emailAddress)
                        #if os(iOS)
                        .keyboardType(.emailAddress)
                        .autocapitalization(.none)
                        #endif
                        .disabled(stage == .code)

                    if stage == .code {
                        TextField("6-digit code", text: $code)
                            #if os(iOS)
                            .keyboardType(.numberPad)
                            #endif
                            .textContentType(.oneTimeCode)
                    }
                } footer: {
                    if stage == .email {
                        Text("We'll email you a sign-in code. In dev mode the code is also printed to the Pylon server's stdout.")
                    } else {
                        Text("Check your email (or the Pylon dev server's stdout) for the code.")
                    }
                }

                if let error {
                    Section {
                        Text(error).foregroundStyle(.red).font(.callout)
                    }
                }

                Section {
                    Button(action: handlePrimary) {
                        if loading { ProgressView() }
                        else { Text(stage == .email ? "Send code" : "Verify and sign in") }
                    }
                    .disabled(loading || (stage == .email ? email.isEmpty : code.count < 4))

                    if stage == .code {
                        Button("Use a different email") {
                            stage = .email
                            code = ""
                            error = nil
                        }
                    }
                }
            }
            .navigationTitle("Sign in")
        }
    }

    private func handlePrimary() {
        Task {
            loading = true
            error = nil
            defer { loading = false }
            do {
                if stage == .email {
                    try await session.client.startMagicCode(email: email)
                    stage = .code
                } else {
                    _ = try await session.client.verifyMagicCode(email: email, code: code)
                    await session.didSignIn()
                }
            } catch let e as PylonError {
                self.error = e.description
            } catch let e {
                self.error = "\(e)"
            }
        }
    }
}
