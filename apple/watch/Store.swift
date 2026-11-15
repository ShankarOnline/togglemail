// What the watch knows: the last inbox snapshot from the iPhone, and the way to ask the iPhone for more.
import Foundation
import WatchConnectivity

struct Snapshot: Codable {
    var accounts: [Inbox]
    var updated: Int64
}

struct Inbox: Codable, Identifiable, Hashable {
    var id: String
    var name: String
    var email: String
    var unread: Int
    var messages: [Item]
    var error: String
}

struct Item: Codable, Identifiable, Hashable {
    var uid: UInt32
    var from: String
    var email: String
    var subject: String
    var date: Int64
    var seen: Bool
    var flagged: Bool
    var id: UInt32 { uid }
}

struct WatchError: LocalizedError {
    var errorDescription: String?
}

@MainActor
final class Store: NSObject, ObservableObject, WCSessionDelegate {
    @Published private(set) var snapshot: Snapshot?
    @Published private(set) var refreshing = false

    private static let saved = "snapshot"

    override init() {
        super.init()
        if Demo.screen != nil {
            snapshot = Demo.snapshot
            return
        }
        if let data = UserDefaults.standard.data(forKey: Self.saved) {
            snapshot = try? JSONDecoder().decode(Snapshot.self, from: data)
        }
        if WCSession.isSupported() {
            WCSession.default.delegate = self
            WCSession.default.activate()
        }
    }

    func inbox(_ id: String) -> Inbox? { snapshot?.accounts.first { $0.id == id } }

    private func take(_ json: String) {
        guard let data = json.data(using: .utf8), let s = try? JSONDecoder().decode(Snapshot.self, from: data) else { return }
        snapshot = s
        UserDefaults.standard.set(data, forKey: Self.saved)
    }

    // MARK: asking the iPhone

    /// Sends a request and waits for the iPhone's answer. Needs the iPhone in reach.
    private func ask(_ request: [String: Any]) async throws -> [String: Any] {
        guard WCSession.default.isReachable else { throw WatchError(errorDescription: String(localized: "Your iPhone is out of reach.")) }
        let json = String(data: try JSONSerialization.data(withJSONObject: request), encoding: .utf8)!
        let reply: [String: Any] = try await withCheckedThrowingContinuation { done in
            WCSession.default.sendMessage(["request": json], replyHandler: { done.resume(returning: $0) }, errorHandler: { done.resume(throwing: $0) })
        }
        guard let text = reply["response"] as? String, let data = text.data(using: .utf8),
              let answer = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw WatchError(errorDescription: String(localized: "No answer from your iPhone."))
        }
        if let error = answer["error"] as? String { throw WatchError(errorDescription: error) }
        return answer
    }

    /// Something to do on the iPhone: now if it is in reach, else as soon as it is.
    private func tell(_ request: [String: Any]) {
        if Demo.screen != nil { return }
        Task {
            if (try? await ask(request)) != nil { return }
            if let data = try? JSONSerialization.data(withJSONObject: request), let json = String(data: data, encoding: .utf8) {
                WCSession.default.transferUserInfo(["request": json])
            }
        }
    }

    func refresh() async {
        if Demo.screen != nil { return }
        refreshing = true
        defer { refreshing = false }
        if let answer = try? await ask(["op": "refresh"]),
           let s = answer["snapshot"], let data = try? JSONSerialization.data(withJSONObject: s), let json = String(data: data, encoding: .utf8) {
            take(json)
        }
    }

    func body(account: String, uid: UInt32) async throws -> String {
        if Demo.screen != nil { return Demo.body }
        let answer = try await ask(["op": "open", "account": account, "uid": uid])
        change(account, uid) { $0.seen = true }
        return answer["body"] as? String ?? ""
    }

    func setSeen(account: String, uid: UInt32, _ on: Bool) {
        change(account, uid) { $0.seen = on }
        tell(["op": "seen", "account": account, "uid": uid, "on": on])
    }

    func delete(account: String, uid: UInt32) {
        guard var s = snapshot, let a = s.accounts.firstIndex(where: { $0.id == account }) else { return }
        if s.accounts[a].messages.first(where: { $0.uid == uid })?.seen == false { s.accounts[a].unread -= 1 }
        s.accounts[a].messages.removeAll { $0.uid == uid }
        snapshot = s
        tell(["op": "delete", "account": account, "uid": uid])
    }

    func reply(account: String, uid: UInt32, text: String) {
        tell(["op": "reply", "account": account, "uid": uid, "text": text])
    }

    /// Updates a message on the watch straight away, keeping the unread count right.
    private func change(_ account: String, _ uid: UInt32, _ edit: (inout Item) -> Void) {
        guard var s = snapshot, let a = s.accounts.firstIndex(where: { $0.id == account }),
              let m = s.accounts[a].messages.firstIndex(where: { $0.uid == uid }) else { return }
        let before = s.accounts[a].messages[m].seen
        edit(&s.accounts[a].messages[m])
        let after = s.accounts[a].messages[m].seen
        if before != after { s.accounts[a].unread = max(0, s.accounts[a].unread + (after ? -1 : 1)) }
        snapshot = s
    }

    // MARK: WCSessionDelegate

    nonisolated func session(_ session: WCSession, activationDidCompleteWith state: WCSessionActivationState, error: Error?) {
        let json = session.receivedApplicationContext["snapshot"] as? String
        Task { @MainActor in
            if let json { take(json) }
            await refresh()
        }
    }

    nonisolated func session(_ session: WCSession, didReceiveApplicationContext context: [String: Any]) {
        guard let json = context["snapshot"] as? String else { return }
        Task { @MainActor in take(json) }
    }
}
