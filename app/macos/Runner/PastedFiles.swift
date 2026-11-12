import AppKit
import FileProvider
import os

/// What went wrong with placing or fetching files, never their names.
private let log = Logger(subsystem: "dev.zerohop.app", category: "paste")

/// Files copied on another device, offered for pasting on this Mac.
///
/// Only their list has come. Zerohop's File Provider extension shows each one
/// as a placeholder in a hidden place, and the clipboard holds the
/// placeholders. When the owner pastes, Finder reads them, the extension asks
/// here, and the core fetches the files from the other device right then.
///
/// macOS adds the extension switched off, and only the owner can switch it
/// on, in System Settings. Until then copies of files stay unpasteable here,
/// and the interface says how to change that.
@available(macOS 13.0, *)
final class PastedFiles: NSObject, FilesSource {
  /// What the interface hears: "ready", "disabled" (waiting for the owner to
  /// switch the extension on), or "unavailable".
  private(set) var state = "unknown"

  private let clipboard: SharedClipboard
  private let report: (String, [String: Any]) -> Void
  /// The hidden place. System Settings lists only places that are not
  /// hidden, so it stays visible until the owner has switched it on, and
  /// hides once that is done.
  private func domain(hidden: Bool) -> NSFileProviderDomain {
    let domain = NSFileProviderDomain(identifier: FilesLink.domain, displayName: "Zerohop")
    domain.isHidden = hidden
    return domain
  }
  private var link: NSXPCConnection?
  /// What the extension should be showing, until when.
  private var shown = PasteList.empty
  private var until = Date.distantPast
  /// A copy that came while the extension was off, placed once it is on.
  private var waiting: (list: PasteList, until: Date)?
  private var expiry: Timer?
  private var access: [URL] = []

  /// Pastes of one copy fetch it once: the first fetches, the rest reuse it.
  private let fetching = DispatchQueue(label: "dev.zerohop.paste")
  private var fetched: (clip: String, folder: URL)?
  private var failedClip: String?

  private static let settingsPane = "x-apple.systempreferences:com.apple.LoginItems-Settings.extension"
  private static let placeTries = 25
  private static let placeRetry: TimeInterval = 0.2

  init(clipboard: SharedClipboard, report: @escaping (String, [String: Any]) -> Void) {
    self.clipboard = clipboard
    self.report = report
    super.init()
    // The owner switches the extension on in System Settings.
    for name in [NSNotification.Name.fileProviderDomainDidChange, NSApplication.didBecomeActiveNotification] {
      NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) {
        [weak self] _ in
        guard let self, self.state == "disabled" else { return }
        self.check()
      }
    }
  }

  static func openSettings() {
    if let url = URL(string: settingsPane) { NSWorkspace.shared.open(url) }
  }

  /// Makes sure the hidden place exists and says whether it can be used.
  func check() {
    ready { [weak self] manager in
      guard let self, let manager else { return }
      guard let waiting = self.waiting else { return self.sweep(manager) }
      self.waiting = nil
      let left = waiting.until.timeIntervalSinceNow
      if left > 1 { self.place(waiting.list, ttl: left) { _ in } }
    }
  }

  /// Offers the files of one copy for pasting. Answers "ready", "disabled",
  /// "unavailable", "gone" or "failed".
  func offer(clip: String, ttlMs: Int, done: @escaping (String) -> Void) {
    let ttl = TimeInterval(max(ttlMs, 0)) / 1000
    DispatchQueue.global(qos: .userInitiated).async {
      let answer = Core.call("clip_files_list", ["id": clip])
      let raw = (answer?["ok"] as? [String: Any])?["items"] as? [[String: Any]]
      let items = raw?.compactMap(Self.item) ?? []
      let list = PasteList(clip: clip, items: items)
      DispatchQueue.main.async {
        guard let raw else { return done("gone") }
        guard items.count == raw.count, list.isSound else { return done("failed") }
        self.place(list, ttl: ttl, done: done)
      }
    }
  }

  private static func item(_ json: [String: Any]) -> PasteItem? {
    guard let index = json["index"] as? Int, let path = json["path"] as? [String],
      let dir = json["dir"] as? Bool, let size = (json["size"] as? NSNumber)?.int64Value
    else { return nil }
    return PasteItem(
      index: index, path: path, dir: dir, size: size,
      modified: (json["mtime_ms"] as? NSNumber)?.int64Value,
      created: (json["created_ms"] as? NSNumber)?.int64Value)
  }

  private func place(_ list: PasteList, ttl: TimeInterval, done: @escaping (String) -> Void) {
    ready { [weak self] manager in
      guard let self else { return }
      guard let manager else {
        if self.state == "disabled" { self.waiting = (list, Date().addingTimeInterval(ttl)) }
        return done(self.state)
      }
      self.waiting = nil
      self.show(list, manager) { taken in
        guard taken else {
          log.error("the extension did not take the list")
          return done("failed")
        }
        self.shown = list
        self.until = Date().addingTimeInterval(ttl)
        self.urls(for: list, manager, tries: Self.placeTries) { urls in
          // A newer copy may have come meanwhile.
          guard self.shown == list else { return done("gone") }
          guard let urls else {
            log.error("placeholders did not appear")
            return done("failed")
          }
          self.access.forEach { $0.stopAccessingSecurityScopedResource() }
          // Without this, a sandboxed app's file URLs never reach the clipboard.
          self.access = urls.filter { $0.startAccessingSecurityScopedResource() }
          guard self.clipboard.applyFiles(urls, ttl: ttl) else {
            log.error("the clipboard did not take the placeholders")
            return done("failed")
          }
          self.expiry?.invalidate()
          let timer = Timer(timeInterval: ttl, repeats: false) { [weak self] _ in
            self?.withdraw()
          }
          RunLoop.main.add(timer, forMode: .common)
          self.expiry = timer
          done("ready")
          self.sweep(manager)
        }
      }
    }
  }

  /// Once a copy's time is up its placeholders go, and so does the clipboard
  /// if it still holds them.
  private func withdraw() {
    expiry?.invalidate()
    expiry = nil
    clipboard.clearIfStillOurs()
    access.forEach { $0.stopAccessingSecurityScopedResource() }
    access = []
    shown = .empty
    if let manager = NSFileProviderManager(for: domain(hidden: true)) {
      show(.empty, manager) { [weak self] _ in self?.sweep(manager) }
    }
  }

  /// Removes placeholders that are not of the copy now offered. The
  /// extension takes back its own as each copy ends, but one that macOS
  /// restarted cannot know what an earlier one showed, so what is left over
  /// goes through the file system; the extension agrees to every deletion.
  private func sweep(_ manager: NSFileProviderManager) {
    let list = shown
    let keep = Set(list.topLevel.compactMap(\.path.first))
    manager.getUserVisibleURL(for: .rootContainer) { [weak self] root, _ in
      guard let root else { return }
      DispatchQueue.global(qos: .utility).async {
        guard let self else { return }
        let scoped = root.startAccessingSecurityScopedResource()
        defer { if scoped { root.stopAccessingSecurityScopedResource() } }
        let entries =
          (try? FileManager.default.contentsOfDirectory(
            at: root, includingPropertiesForKeys: nil, options: [.skipsHiddenFiles])) ?? []
        var removed = 0
        for entry in entries where !keep.contains(entry.lastPathComponent) {
          // A newer copy may have come meanwhile; its placeholders stay.
          guard DispatchQueue.main.sync(execute: { self.shown == list }) else { break }
          if (try? FileManager.default.removeItem(at: entry)) != nil { removed += 1 }
        }
        if removed > 0 { log.notice("removed \(removed) leftover placeholders") }
      }
    }
  }

  // MARK: The hidden place

  private func ready(_ then: @escaping (NSFileProviderManager?) -> Void) {
    // The place is there whether or not the owner has switched it on; only
    // the system's list says which.
    NSFileProviderManager.getDomainsWithCompletionHandler { [weak self] domains, _ in
      DispatchQueue.main.async {
        guard let self else { return }
        let ours = domains.first { $0.identifier == FilesLink.domain }
        let enabled = ours?.userEnabled ?? false
        if let ours, ours.isHidden == enabled {
          return self.settle(ours, enabled: enabled, then)
        }
        NSFileProviderManager.add(self.domain(hidden: enabled)) { error in
          DispatchQueue.main.async {
            guard error == nil else {
              self.set("unavailable")
              return then(nil)
            }
            self.settle(self.domain(hidden: enabled), enabled: enabled, then)
          }
        }
      }
    }
  }

  private func settle(
    _ domain: NSFileProviderDomain, enabled: Bool,
    _ then: @escaping (NSFileProviderManager?) -> Void
  ) {
    guard enabled else {
      set("disabled")
      return then(nil)
    }
    guard let manager = NSFileProviderManager(for: domain) else {
      set("unavailable")
      return then(nil)
    }
    set("ready")
    then(manager)
  }

  private func set(_ next: String) {
    guard next != state else { return }
    log.notice("pasting files: \(next, privacy: .public)")
    state = next
    report("clipboardFilesState", ["state": next])
  }

  /// Where each top-level placeholder is, once the system has made them.
  private func urls(
    for list: PasteList, _ manager: NSFileProviderManager, tries: Int,
    then: @escaping ([URL]?) -> Void
  ) {
    let ids = list.topLevel.map { FilesLink.itemID(clip: list.clip, index: $0.index) }
    var found = [URL?](repeating: nil, count: ids.count)
    let group = DispatchGroup()
    for (i, id) in ids.enumerated() {
      group.enter()
      manager.getUserVisibleURL(for: id) { url, _ in
        DispatchQueue.main.async {
          found[i] = url
          group.leave()
        }
      }
    }
    group.notify(queue: .main) {
      let urls = found.compactMap { $0 }
      if urls.count == ids.count { return then(urls) }
      guard tries > 1, self.shown == list else { return then(nil) }
      DispatchQueue.main.asyncAfter(deadline: .now() + Self.placeRetry) {
        self.urls(for: list, manager, tries: tries - 1, then: then)
      }
    }
  }

  // MARK: The extension

  private func show(
    _ list: PasteList, _ manager: NSFileProviderManager, then: @escaping (Bool) -> Void
  ) {
    guard let data = try? JSONEncoder().encode(list) else { return then(false) }
    connect(manager) { link in
      guard let link else { return then(false) }
      let proxy = link.remoteObjectProxyWithErrorHandler { _ in
        DispatchQueue.main.async { then(false) }
      }
      guard let shelf = proxy as? FilesShelf else { return then(false) }
      shelf.show(data) { taken in DispatchQueue.main.async { then(taken) } }
    }
  }

  private func connect(_ manager: NSFileProviderManager, then: @escaping (NSXPCConnection?) -> Void)
  {
    if let link { return then(link) }
    manager.getService(named: FilesLink.service, for: .rootContainer) { service, _ in
      guard let service else { return DispatchQueue.main.async { then(nil) } }
      service.getFileProviderConnection { connection, _ in
        DispatchQueue.main.async {
          guard let connection else { return then(nil) }
          connection.remoteObjectInterface = NSXPCInterface(with: FilesShelf.self)
          connection.exportedInterface = NSXPCInterface(with: FilesSource.self)
          connection.exportedObject = self
          let lost: () -> Void = { [weak self, weak connection] in
            DispatchQueue.main.async {
              guard let self, let connection else { return }
              self.lost(connection)
            }
          }
          connection.invalidationHandler = lost
          connection.interruptionHandler = lost
          connection.resume()
          self.link = connection
          then(connection)
        }
      }
    }
  }

  /// macOS stops the extension when it likes, and a new one has nothing to
  /// show. While a copy is still offered, it gets the list again.
  private func lost(_ connection: NSXPCConnection) {
    guard link === connection else { return }
    link = nil
    connection.invalidate()
    guard shown != .empty, until > Date(),
      let manager = NSFileProviderManager(for: domain(hidden: true))
    else { return }
    let list = shown
    DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
      guard self.shown == list else { return }
      self.show(list, manager) { _ in }
    }
  }

  func open(_ clip: String, index: Int, reply: @escaping (FileHandle?, String?) -> Void) {
    fetching.async {
      let list = DispatchQueue.main.sync { self.shown }
      guard list.clip == clip, list.items.indices.contains(index), !list.items[index].dir else {
        return reply(nil, "clip_gone")
      }
      let folder: URL
      if let fetched = self.fetched, fetched.clip == clip {
        folder = fetched.folder
      } else {
        let answer = Core.call("paste_files", ["id": clip])
        guard let path = (answer?["ok"] as? [String: Any])?["folder"] as? String else {
          let kind = (answer?["error"] as? [String: Any])?["kind"] as? String ?? "unreachable"
          log.error("paste could not fetch: \(kind, privacy: .public)")
          if self.failedClip != clip {
            self.failedClip = clip
            DispatchQueue.main.async { self.report("clipboardPasteFailed", [:]) }
          }
          return reply(nil, kind)
        }
        folder = URL(fileURLWithPath: path, isDirectory: true).standardizedFileURL
        self.fetched = (clip, folder)
      }
      let file = list.items[index].path.reduce(folder) { $0.appendingPathComponent($1) }
        .standardizedFileURL
      guard file.path.hasPrefix(folder.path + "/"),
        let handle = try? FileHandle(forReadingFrom: file)
      else { return reply(nil, "missing") }
      reply(handle, nil)
    }
  }
}
