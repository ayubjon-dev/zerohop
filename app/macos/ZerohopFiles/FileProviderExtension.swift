import FileProvider
import UniformTypeIdentifiers
import os

private let log = Logger(subsystem: "dev.zerohop.app.files", category: "paste")

/// Shows files copied on another device as placeholders in a hidden place,
/// and gives a placeholder its contents only when something reads it, which
/// in practice is the owner pasting it. Nothing here is written by the
/// owner: the place only ever holds the latest copy, and only until it ends.
final class FileProviderExtension: NSObject, NSFileProviderReplicatedExtension,
  NSFileProviderServicing
{
  required init(domain: NSFileProviderDomain) {
    super.init()
    Shelf.shared.manager = NSFileProviderManager(for: domain)
  }

  func invalidate() {}

  func item(
    for identifier: NSFileProviderItemIdentifier, request: NSFileProviderRequest,
    completionHandler: @escaping (NSFileProviderItem?, Error?) -> Void
  ) -> Progress {
    if identifier == .rootContainer {
      completionHandler(Root.item, nil)
    } else if let placeholder = Shelf.shared.placeholder(identifier) {
      completionHandler(placeholder, nil)
    } else {
      completionHandler(nil, NSError.fileProviderErrorForNonExistentItem(withIdentifier: identifier))
    }
    return Progress()
  }

  func fetchContents(
    for itemIdentifier: NSFileProviderItemIdentifier,
    version requestedVersion: NSFileProviderItemVersion?, request: NSFileProviderRequest,
    completionHandler: @escaping (URL?, NSFileProviderItem?, Error?) -> Void
  ) -> Progress {
    let progress = Progress(totalUnitCount: 1)
    guard let placeholder = Shelf.shared.placeholder(itemIdentifier), !placeholder.item.dir else {
      completionHandler(
        nil, nil, NSError.fileProviderErrorForNonExistentItem(withIdentifier: itemIdentifier))
      return progress
    }
    Shelf.shared.open(placeholder) { handle, problem in
      guard let handle else {
        log.notice("paste could not fetch: \(problem ?? "no app", privacy: .public)")
        let code: NSFileProviderError.Code =
          problem == "clip_gone" ? .noSuchItem : .serverUnreachable
        completionHandler(nil, nil, NSFileProviderError(code))
        return
      }
      do {
        let url = try Self.take(handle, size: placeholder.item.size)
        progress.completedUnitCount = 1
        completionHandler(url, placeholder, nil)
      } catch {
        log.error("paste could not keep a file: \(error.localizedDescription, privacy: .public)")
        completionHandler(nil, nil, error)
      }
    }
    return progress
  }

  /// A file of our own with what the app's file holds. On the same disk the
  /// copy shares the app's blocks, so even a large file costs nothing.
  private static func take(_ handle: FileHandle, size: Int64) throws -> URL {
    defer { try? handle.close() }
    let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    if fclonefileat(handle.fileDescriptor, AT_FDCWD, url.path, 0) != 0 {
      FileManager.default.createFile(atPath: url.path, contents: nil)
      let out = try FileHandle(forWritingTo: url)
      defer { try? out.close() }
      while let chunk = try handle.read(upToCount: 1 << 20), !chunk.isEmpty {
        try out.write(contentsOf: chunk)
      }
    }
    let written = try FileManager.default.attributesOfItem(atPath: url.path)[.size] as? Int64
    guard written == size else {
      try? FileManager.default.removeItem(at: url)
      throw NSFileProviderError(.cannotSynchronize)
    }
    return url
  }

  // Nothing here is the owner's to change: an edit keeps what the other
  // device copied, and nothing new can be put here.

  func createItem(
    basedOn itemTemplate: NSFileProviderItem, fields: NSFileProviderItemFields, contents url: URL?,
    options: NSFileProviderCreateItemOptions = [], request: NSFileProviderRequest,
    completionHandler: @escaping (NSFileProviderItem?, NSFileProviderItemFields, Bool, Error?) ->
      Void
  ) -> Progress {
    completionHandler(
      nil, [], false, NSError(domain: NSCocoaErrorDomain, code: NSFeatureUnsupportedError))
    return Progress()
  }

  func modifyItem(
    _ item: NSFileProviderItem, baseVersion version: NSFileProviderItemVersion,
    changedFields: NSFileProviderItemFields, contents newContents: URL?,
    options: NSFileProviderModifyItemOptions = [], request: NSFileProviderRequest,
    completionHandler: @escaping (NSFileProviderItem?, NSFileProviderItemFields, Bool, Error?) ->
      Void
  ) -> Progress {
    if let placeholder = Shelf.shared.placeholder(item.itemIdentifier) {
      completionHandler(placeholder, [], false, nil)
    } else {
      completionHandler(
        nil, [], false,
        NSError.fileProviderErrorForNonExistentItem(withIdentifier: item.itemIdentifier))
    }
    return Progress()
  }

  func deleteItem(
    identifier: NSFileProviderItemIdentifier, baseVersion version: NSFileProviderItemVersion,
    options: NSFileProviderDeleteItemOptions = [], request: NSFileProviderRequest,
    completionHandler: @escaping (Error?) -> Void
  ) -> Progress {
    completionHandler(nil)
    return Progress()
  }

  func enumerator(
    for containerItemIdentifier: NSFileProviderItemIdentifier, request: NSFileProviderRequest
  ) throws -> NSFileProviderEnumerator {
    Listing(container: containerItemIdentifier)
  }

  func supportedServiceSources(
    for itemIdentifier: NSFileProviderItemIdentifier,
    completionHandler: @escaping ([NSFileProviderServiceSource]?, Error?) -> Void
  ) -> Progress {
    completionHandler([Link.shared], nil)
    return Progress()
  }
}

/// The top of the hidden place.
final class Root: NSObject, NSFileProviderItem {
  static let item = Root()
  let itemIdentifier = NSFileProviderItemIdentifier.rootContainer
  let parentItemIdentifier = NSFileProviderItemIdentifier.rootContainer
  let filename = "Zerohop"
  let contentType = UTType.folder
  let capabilities: NSFileProviderItemCapabilities = [.allowsReading, .allowsContentEnumerating]
  let itemVersion = NSFileProviderItemVersion(contentVersion: Data([1]), metadataVersion: Data([1]))
}

/// A file or folder copied on another device, before any of it has come.
final class Placeholder: NSObject, NSFileProviderItem {
  let item: PasteItem
  let itemIdentifier: NSFileProviderItemIdentifier
  let parentItemIdentifier: NSFileProviderItemIdentifier
  let itemVersion: NSFileProviderItemVersion

  init(_ item: PasteItem, clip: String, parent: NSFileProviderItemIdentifier) {
    self.item = item
    itemIdentifier = FilesLink.itemID(clip: clip, index: item.index)
    parentItemIdentifier = parent
    // One copy's files never change, so the copy itself is the version.
    let version = Data(clip.utf8)
    itemVersion = NSFileProviderItemVersion(contentVersion: version, metadataVersion: version)
  }

  var filename: String { item.path.last ?? "" }

  var contentType: UTType {
    if item.dir { return .folder }
    let ext = (filename as NSString).pathExtension
    return ext.isEmpty ? .data : UTType(filenameExtension: ext) ?? .data
  }

  // Without writing and deleting, the system marks the file locked, and a
  // pasted copy would keep that mark.
  var capabilities: NSFileProviderItemCapabilities {
    item.dir
      ? [.allowsReading, .allowsContentEnumerating, .allowsWriting, .allowsDeleting]
      : [.allowsReading, .allowsWriting, .allowsDeleting]
  }

  var fileSystemFlags: NSFileProviderFileSystemFlags {
    item.dir ? [.userReadable, .userWritable, .userExecutable] : [.userReadable, .userWritable]
  }

  var documentSize: NSNumber? { item.dir ? nil : NSNumber(value: item.size) }

  /// The download mark every received file gets, so that macOS checks a
  /// pasted program before it first runs, as it would a downloaded one.
  var extendedAttributes: [String: Data] {
    item.dir ? [:] : ["com.apple.quarantine": Self.downloadMark]
  }

  private static let downloadMark: Data = {
    let now = String(UInt64(Date().timeIntervalSince1970), radix: 16)
    return Data("0083;\(now);Zerohop;".utf8)
  }()

  var contentModificationDate: Date? { item.modified.map(Self.date) }
  var creationDate: Date? { item.created.map(Self.date) }

  private static func date(_ ms: Int64) -> Date {
    Date(timeIntervalSince1970: TimeInterval(ms) / 1000)
  }
}

/// What the extension shows, and the app it asks for contents. Lives only in
/// memory: when macOS restarts the extension, the app sends the list again.
final class Shelf: NSObject, FilesShelf {
  static let shared = Shelf()

  var manager: NSFileProviderManager?

  private let lock = NSLock()
  /// New on each start, so that the system lists everything again rather
  /// than ask for changes this process never saw.
  private let generation = UUID()
  private var version: UInt64 = 0
  private var list = PasteList.empty
  private var byID: [NSFileProviderItemIdentifier: Placeholder] = [:]
  private var children: [NSFileProviderItemIdentifier: [Placeholder]] = [:]
  private var lastChange: (from: UInt64, gone: [NSFileProviderItemIdentifier], came: [Placeholder])?
  private var app: NSXPCConnection?
  private var waiting: [(NSXPCConnection?) -> Void] = []

  private static let appWait: TimeInterval = 10

  // MARK: What the app asks

  func show(_ data: Data, reply: @escaping (Bool) -> Void) {
    guard let next = try? JSONDecoder().decode(PasteList.self, from: data), next.isSound else {
      reply(false)
      return
    }
    lock.withLock { install(next) }
    manager?.signalEnumerator(for: .workingSet) { _ in }
    reply(true)
  }

  private func install(_ next: PasteList) {
    guard next != list else { return }
    var byID: [NSFileProviderItemIdentifier: Placeholder] = [:]
    var children: [NSFileProviderItemIdentifier: [Placeholder]] = [:]
    var folderIDs: [String: NSFileProviderItemIdentifier] = [:]
    for item in next.items {
      let parent =
        item.path.count == 1
        ? NSFileProviderItemIdentifier.rootContainer
        : folderIDs[item.path.dropLast().joined(separator: "/")] ?? .rootContainer
      let placeholder = Placeholder(item, clip: next.clip, parent: parent)
      byID[placeholder.itemIdentifier] = placeholder
      children[parent, default: []].append(placeholder)
      if item.dir { folderIDs[item.path.joined(separator: "/")] = placeholder.itemIdentifier }
    }
    let gone = self.byID.keys.filter { byID[$0] == nil }
    lastChange = (version, gone, Array(byID.values))
    version += 1
    list = next
    self.byID = byID
    self.children = children
  }

  // MARK: What the system asks

  func placeholder(_ id: NSFileProviderItemIdentifier) -> Placeholder? {
    lock.withLock { byID[id] }
  }

  func items(in container: NSFileProviderItemIdentifier) -> [NSFileProviderItem] {
    lock.withLock {
      switch container {
      case .workingSet:
        return list.items.compactMap { byID[FilesLink.itemID(clip: list.clip, index: $0.index)] }
      case .trashContainer: return []
      default: return children[container] ?? []
      }
    }
  }

  var anchor: NSFileProviderSyncAnchor {
    lock.withLock { Self.anchor(generation, version) }
  }

  /// What changed since the system last looked, or nil when that was before
  /// this process started or more than one list ago.
  func changes(since anchor: NSFileProviderSyncAnchor) -> (
    gone: [NSFileProviderItemIdentifier], came: [Placeholder], now: NSFileProviderSyncAnchor
  )? {
    lock.withLock {
      let now = Self.anchor(generation, version)
      if anchor.rawValue == now.rawValue { return ([], [], now) }
      guard let lastChange, anchor.rawValue == Self.anchor(generation, lastChange.from).rawValue
      else { return nil }
      return (lastChange.gone, lastChange.came, now)
    }
  }

  private static func anchor(_ generation: UUID, _ version: UInt64) -> NSFileProviderSyncAnchor {
    var data = withUnsafeBytes(of: generation.uuid) { Data($0) }
    withUnsafeBytes(of: version.littleEndian) { data.append(contentsOf: $0) }
    return NSFileProviderSyncAnchor(data)
  }

  // MARK: The app

  func connected(_ connection: NSXPCConnection) {
    let ready: [(NSXPCConnection?) -> Void] = lock.withLock {
      app = connection
      defer { waiting = [] }
      return waiting
    }
    ready.forEach { $0(connection) }
  }

  func lost(_ connection: NSXPCConnection) {
    lock.withLock {
      if app === connection { app = nil }
    }
  }

  /// Asks the app for one file. The app may be reconnecting after macOS
  /// restarted this extension, so it gets a moment to come back.
  func open(_ placeholder: Placeholder, reply: @escaping (FileHandle?, String?) -> Void) {
    let clip = lock.withLock { list.clip }
    var answered = false
    let answer: (FileHandle?, String?) -> Void = { [lock] handle, problem in
      let first = lock.withLock { () -> Bool in
        defer { answered = true }
        return !answered
      }
      if first { reply(handle, problem) } else { try? handle?.close() }
    }
    withApp { app in
      guard let app else { return answer(nil, "unreachable") }
      let proxy = app.remoteObjectProxyWithErrorHandler { _ in answer(nil, "unreachable") }
      guard let source = proxy as? FilesSource else { return answer(nil, "unreachable") }
      source.open(clip, index: placeholder.item.index, reply: answer)
    }
  }

  private func withApp(_ then: @escaping (NSXPCConnection?) -> Void) {
    let app: NSXPCConnection? = lock.withLock {
      if self.app == nil { waiting.append(then) }
      return self.app
    }
    if let app { return then(app) }
    DispatchQueue.global().asyncAfter(deadline: .now() + Self.appWait) { [weak self] in
      guard let self else { return }
      let late: [(NSXPCConnection?) -> Void] = self.lock.withLock {
        defer { self.waiting = [] }
        return self.waiting
      }
      late.forEach { $0(nil) }
    }
  }
}

/// Lists what the shelf holds, a page at a time.
final class Listing: NSObject, NSFileProviderEnumerator {
  private let container: NSFileProviderItemIdentifier
  private static let pageSize = 500

  init(container: NSFileProviderItemIdentifier) {
    self.container = container
  }

  func invalidate() {}

  func enumerateItems(
    for observer: NSFileProviderEnumerationObserver, startingAt page: NSFileProviderPage
  ) {
    let items = Shelf.shared.items(in: container)
    let start = Self.offset(page)
    let end = min(items.count, start + Self.pageSize)
    if start < end { observer.didEnumerate(Array(items[start..<end])) }
    observer.finishEnumerating(upTo: end < items.count ? Self.page(end) : nil)
  }

  func enumerateChanges(
    for observer: NSFileProviderChangeObserver, from syncAnchor: NSFileProviderSyncAnchor
  ) {
    guard let changes = Shelf.shared.changes(since: syncAnchor) else {
      observer.finishEnumeratingWithError(NSFileProviderError(.syncAnchorExpired))
      return
    }
    if !changes.gone.isEmpty { observer.didDeleteItems(withIdentifiers: changes.gone) }
    if !changes.came.isEmpty { observer.didUpdate(changes.came) }
    observer.finishEnumeratingChanges(upTo: changes.now, moreComing: false)
  }

  func currentSyncAnchor(completionHandler: @escaping (NSFileProviderSyncAnchor?) -> Void) {
    completionHandler(Shelf.shared.anchor)
  }

  // The system's own first pages are not offsets; anything that is not one
  // starts at the beginning.
  private static func offset(_ page: NSFileProviderPage) -> Int {
    guard page.rawValue.count == 8 else { return 0 }
    let value = page.rawValue.withUnsafeBytes { $0.loadUnaligned(as: UInt64.self) }
    return Int(clamping: UInt64(littleEndian: value))
  }

  private static func page(_ offset: Int) -> NSFileProviderPage {
    withUnsafeBytes(of: UInt64(offset).littleEndian) { NSFileProviderPage(Data($0)) }
  }
}

/// The channel the app opens to this extension. macOS lets only processes
/// that manage the place reach it, which is Zerohop itself.
final class Link: NSObject, NSFileProviderServiceSource, NSXPCListenerDelegate {
  static let shared = Link()

  let serviceName = FilesLink.service
  let isRestricted = true
  private let listener = NSXPCListener.anonymous()

  override init() {
    super.init()
    listener.delegate = self
    listener.resume()
  }

  func makeListenerEndpoint() throws -> NSXPCListenerEndpoint {
    listener.endpoint
  }

  func listener(_ listener: NSXPCListener, shouldAcceptNewConnection connection: NSXPCConnection)
    -> Bool
  {
    connection.exportedInterface = NSXPCInterface(with: FilesShelf.self)
    connection.exportedObject = Shelf.shared
    connection.remoteObjectInterface = NSXPCInterface(with: FilesSource.self)
    connection.invalidationHandler = { [weak connection] in
      if let connection { Shelf.shared.lost(connection) }
    }
    connection.interruptionHandler = connection.invalidationHandler
    connection.resume()
    Shelf.shared.connected(connection)
    return true
  }
}
