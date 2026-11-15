import SwiftUI

private func when(_ date: Int64) -> String {
    let d = Date(timeIntervalSince1970: TimeInterval(date))
    return Calendar.current.isDateInToday(d) ? d.formatted(date: .omitted, time: .shortened) : d.formatted(.dateTime.day().month(.abbreviated))
}

struct AccountsView: View {
    @EnvironmentObject private var store: Store

    var body: some View {
        Group {
            if let s = store.snapshot, !s.accounts.isEmpty {
                List {
                    ForEach(s.accounts) { a in
                        NavigationLink(value: a.id) {
                            HStack {
                                VStack(alignment: .leading) {
                                    Text(a.name).font(.headline).lineLimit(1)
                                    Text(a.email).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                                }
                                Spacer()
                                if a.unread > 0 {
                                    Text("\(a.unread)").font(.caption.bold()).padding(.horizontal, 6).padding(.vertical, 2)
                                        .background(Capsule().fill(Color("Accent")))
                                }
                            }
                        }
                    }
                    Section {
                        Button {
                            Task { await store.refresh() }
                        } label: {
                            Label(store.refreshing ? "Checking…" : "Check for new mail", systemImage: "arrow.clockwise")
                        }
                        .disabled(store.refreshing)
                    } footer: {
                        Text("Updated \(when(s.updated))")
                    }
                }
                .navigationDestination(for: String.self) { InboxView(account: $0) }
            } else {
                ScrollView {
                    VStack(spacing: 8) {
                        Image(systemName: "iphone").font(.title)
                        Text("Open ToggleMail on your iPhone and add an account. Your inboxes show up here.")
                            .multilineTextAlignment(.center).font(.footnote)
                        Button("Try again") { Task { await store.refresh() } }
                    }
                }
            }
        }
        .navigationTitle("ToggleMail")
    }
}

struct InboxView: View {
    @EnvironmentObject private var store: Store
    let account: String

    var body: some View {
        let inbox = store.inbox(account)
        List {
            if let e = inbox?.error, !e.isEmpty {
                Text(e).font(.footnote).foregroundStyle(.red)
            }
            if inbox?.messages.isEmpty ?? true, inbox?.error.isEmpty ?? true {
                Text("No mail in your inbox.").foregroundStyle(.secondary)
            }
            ForEach(inbox?.messages ?? []) { m in
                NavigationLink(value: m) {
                    VStack(alignment: .leading, spacing: 2) {
                        HStack {
                            if !m.seen { Circle().fill(Color("Accent")).frame(width: 6, height: 6) }
                            Text(m.from).font(.headline).lineLimit(1)
                            Spacer()
                            Text(when(m.date)).font(.caption2).foregroundStyle(.secondary)
                        }
                        Text(m.subject.isEmpty ? String(localized: "(no subject)") : m.subject)
                            .font(.footnote).foregroundStyle(m.seen ? .secondary : .primary).lineLimit(2)
                    }
                }
                .swipeActions {
                    Button(role: .destructive) { store.delete(account: account, uid: m.uid) } label: { Label("Delete", systemImage: "trash") }
                    Button { store.setSeen(account: account, uid: m.uid, !m.seen) } label: {
                        Label(m.seen ? "Unread" : "Read", systemImage: m.seen ? "envelope.badge" : "envelope.open")
                    }
                }
            }
        }
        .navigationTitle(inbox?.name ?? "Inbox")
        .navigationDestination(for: Item.self) { MessageView(account: account, item: $0) }
    }
}

struct MessageView: View {
    @EnvironmentObject private var store: Store
    @Environment(\.dismiss) private var dismiss
    let account: String
    let item: Item
    @State private var body_: String?
    @State private var error: String?
    @State private var confirmDelete = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 6) {
                Text(item.from).font(.headline)
                Text(item.email).font(.caption2).foregroundStyle(.secondary)
                Text(item.subject.isEmpty ? String(localized: "(no subject)") : item.subject).font(.subheadline.bold())
                Text(when(item.date)).font(.caption2).foregroundStyle(.secondary)
                Divider()
                if let body_ {
                    Text(body_).font(.footnote)
                } else if let error {
                    Text(error).font(.footnote).foregroundStyle(.secondary)
                } else {
                    ProgressView()
                }
                NavigationLink {
                    ReplyView(account: account, item: item)
                } label: {
                    Label("Reply", systemImage: "arrowshape.turn.up.left")
                }
                .padding(.top, 8)
                Button {
                    store.setSeen(account: account, uid: item.uid, false)
                    dismiss()
                } label: {
                    Label("Mark as unread", systemImage: "envelope.badge")
                }
                Button(role: .destructive) { confirmDelete = true } label: { Label("Delete", systemImage: "trash") }
            }
        }
        .confirmationDialog("Delete this email?", isPresented: $confirmDelete) {
            Button("Delete", role: .destructive) {
                store.delete(account: account, uid: item.uid)
                dismiss()
            }
        }
        .task {
            do { body_ = try await store.body(account: account, uid: item.uid) } catch { self.error = error.localizedDescription }
        }
    }
}

struct ReplyView: View {
    @EnvironmentObject private var store: Store
    @Environment(\.dismiss) private var dismiss
    let account: String
    let item: Item
    @State private var text = ""
    @State private var sent = false

    private let quick = [
        String(localized: "Thank you!"),
        String(localized: "Got it, thanks."),
        String(localized: "I'll get back to you soon."),
        String(localized: "Yes"),
        String(localized: "No"),
        String(localized: "Call me when you can."),
    ]

    var body: some View {
        if sent {
            VStack(spacing: 8) {
                Image(systemName: "checkmark.circle.fill").font(.largeTitle).foregroundStyle(.green)
                Text("Your iPhone sends it now, or as soon as it is in reach.").multilineTextAlignment(.center).font(.footnote)
                Button("Done") { dismiss() }
            }
        } else {
            List {
                Section {
                    TextField("Write a reply", text: $text)
                    if !text.trimmingCharacters(in: .whitespaces).isEmpty {
                        Button { send(text) } label: { Label("Send", systemImage: "paperplane.fill") }
                    }
                }
                Section("Quick replies") {
                    ForEach(quick, id: \.self) { q in Button(q) { send(q) } }
                }
            }
            .navigationTitle("Reply")
        }
    }

    private func send(_ reply: String) {
        store.reply(account: account, uid: item.uid, text: reply)
        sent = true
    }
}
