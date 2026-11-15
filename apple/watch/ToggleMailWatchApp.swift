// ToggleMail on Apple Watch: every inbox at a glance, read a message, mark it read, delete it, or reply.
import SwiftUI

@main
struct ToggleMailWatchApp: App {
    @StateObject private var store = Store()

    var body: some Scene {
        WindowGroup {
            NavigationStack { AccountsView() }
                .environmentObject(store)
                .tint(Color("Accent"))
        }
    }
}
