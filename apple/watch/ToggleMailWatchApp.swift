// ToggleMail on Apple Watch: every inbox at a glance, read a message, mark it read, delete it, or reply.
import SwiftUI

@main
struct ToggleMailWatchApp: App {
    @StateObject private var store = Store()
    @State private var path = ToggleMailWatchApp.startPath()

    var body: some Scene {
        WindowGroup {
            NavigationStack(path: $path) { AccountsView() }
                .environmentObject(store)
                .tint(Color("Accent"))
        }
    }

    /// Where the app opens: the list of accounts, or (for screenshots) a demo screen further in.
    private static func startPath() -> NavigationPath {
        var path = NavigationPath()
        guard let screen = Demo.screen, screen != "accounts" else { return path }
        path.append(Demo.account)
        if screen == "message" || screen == "reply", let first = Demo.snapshot.accounts.first?.messages.first {
            path.append(first)
        }
        if screen == "reply" { path.append(ReplyTarget(account: Demo.account, item: Demo.snapshot.accounts[0].messages[0])) }
        return path
    }
}

/// Opens the reply screen through the navigation path (used by the reply screenshot).
struct ReplyTarget: Hashable {
    let account: String
    let item: Item
}
