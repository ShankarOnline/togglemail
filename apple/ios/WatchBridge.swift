// The iPhone side of the Apple Watch app. It passes inbox snapshots from the Rust core (src-tauri/src/watch.rs)
// to the watch over WatchConnectivity, and hands the watch's requests to the core. Requests also wake the
// iPhone app in the background, so the watch works while ToggleMail is closed on the phone.
import Foundation
import WatchConnectivity

@_silgen_name("togglemail_watch_request")
private func rustWatchRequest(_ json: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?

@_silgen_name("togglemail_watch_free")
private func rustWatchFree(_ s: UnsafeMutablePointer<CChar>?)

final class WatchBridge: NSObject, WCSessionDelegate {
    static let shared = WatchBridge()

    /// Requests run one at a time, off the main thread (the core waits on the network).
    private let queue = DispatchQueue(label: "com.toggleminds.togglemail.watch")

    func start() {
        guard WCSession.isSupported() else { return }
        WCSession.default.delegate = self
        WCSession.default.activate()
    }

    var installed: Bool {
        guard WCSession.isSupported() else { return false }
        let s = WCSession.default
        return s.activationState == .activated && s.isPaired && s.isWatchAppInstalled
    }

    /// The latest snapshot; the watch gets it as soon as it can, even when it is not running.
    func push(_ json: String) {
        guard installed else { return }
        try? WCSession.default.updateApplicationContext(["snapshot": json])
    }

    private func answer(_ json: String) -> String {
        json.withCString { p in
            guard let out = rustWatchRequest(p) else { return #"{"ok":false,"error":"ToggleMail is still starting. Try again."}"# }
            defer { rustWatchFree(out) }
            return String(cString: out)
        }
    }

    func session(_ session: WCSession, activationDidCompleteWith state: WCSessionActivationState, error: Error?) {}
    func sessionDidBecomeInactive(_ session: WCSession) {}
    func sessionDidDeactivate(_ session: WCSession) { session.activate() }

    /// The watch is waiting for an answer (open a message, refresh).
    func session(_ session: WCSession, didReceiveMessage message: [String: Any], replyHandler: @escaping ([String: Any]) -> Void) {
        guard let request = message["request"] as? String else { return replyHandler([:]) }
        queue.async { replyHandler(["response": self.answer(request)]) }
    }

    /// Queued while the phone was out of reach (mark read, delete, reply).
    func session(_ session: WCSession, didReceiveUserInfo userInfo: [String: Any] = [:]) {
        guard let request = userInfo["request"] as? String else { return }
        queue.async { _ = self.answer(request) }
    }
}

// Called by the Rust core.
@_cdecl("togglemail_watch_start")
public func togglemailWatchStart() {
    DispatchQueue.main.async { WatchBridge.shared.start() }
}

@_cdecl("togglemail_watch_installed")
public func togglemailWatchInstalled() -> Bool {
    WatchBridge.shared.installed
}

@_cdecl("togglemail_watch_push")
public func togglemailWatchPush(_ json: UnsafePointer<CChar>) {
    WatchBridge.shared.push(String(cString: json))
}
