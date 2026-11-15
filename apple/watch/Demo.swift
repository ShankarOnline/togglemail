// Sample inboxes for App Store screenshots in the watch simulator, which has no iPhone to send real ones.
// Only used when the app is started with `-demo inbox` (or `accounts`, `message`, `reply`), e.g.
//   xcrun simctl launch <watch> com.toggleminds.togglemail.watchkitapp -demo message
import Foundation

enum Demo {
    /// The screen to open, when started for screenshots.
    static let screen: String? = {
        let args = ProcessInfo.processInfo.arguments
        guard let i = args.firstIndex(of: "-demo"), i + 1 < args.count else { return nil }
        return args[i + 1]
    }()

    static let account = "asha@toggleminds.com"

    static var snapshot: Snapshot {
        let now = Int64(Date().timeIntervalSince1970)
        func item(_ uid: UInt32, _ from: String, _ email: String, _ subject: String, _ minutesAgo: Int64, seen: Bool, flagged: Bool = false) -> Item {
            Item(uid: uid, from: from, email: email, subject: subject, date: now - minutesAgo * 60, seen: seen, flagged: flagged)
        }
        return Snapshot(accounts: [
            Inbox(id: account, name: "Asha Rao", email: account, unread: 2, messages: [
                item(100, "Ravi Kumar", "ravi@example.com", "Weekend trip: train tickets booked", 12, seen: false, flagged: true),
                item(99, "Meena Iyer", "meena@example.com", "Photos from the school annual day", 48, seen: false),
                item(98, "Toggle Minds", "hello@toggleminds.com", "Welcome to ToggleMail", 95, seen: true),
                item(97, "Arun Prakash", "arun@example.com", "Re: Cricket on Sunday?", 300, seen: true),
                item(96, "Library Updates", "library@example.com", "Three new books on your wishlist are in", 600, seen: true),
            ], error: ""),
            Inbox(id: "asha.rao@example.com", name: "Asha (Gmail)", email: "asha.rao@example.com", unread: 3, messages: [], error: ""),
            Inbox(id: "9876543210@phonemail.net", name: "PhoneMail", email: "9876543210@phonemail.net", unread: 0, messages: [], error: ""),
        ], updated: now)
    }

    static let body = """
    Hi Asha,

    All four tickets are booked for Saturday morning. Coach B2, seats 33–36.

    Departs 06:10 · Arrives 11:45

    I have attached the e-tickets and the hotel booking.

    Ravi
    """
}
