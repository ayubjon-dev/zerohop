import AppKit
import os

/// What went wrong while starting, never what the core carries.
private let log = Logger(subsystem: "dev.zerohop.app", category: "service")

/// Zerohop with its window closed: the core, finding devices and the shared
/// clipboard, all without Flutter. The interface runs only while its window
/// is open, and hears the core through here.
final class Service {
  static let shared = Service()

  enum Started: String {
    case running
    /// The keychain did not hand over the secret that opens the device key.
    case keyUnavailable = "key_unavailable"
    case failed
  }

  let clipboard: SharedClipboard
  /// Pasting files copied on other devices, which needs macOS 13.
  private var pastedFiles: AnyObject?
  let bonjour = Bonjour()

  private var started: Started?
  private var starting = false
  private var waiting: [(Started) -> Void] = []

  /// The open window's interface, which hears everything the core says.
  private weak var link: InterfaceLink?

  private var clipboardOn: Bool?
  private var pairing = false

  private init() {
    var report: (String, [String: Any]) -> Void = { _, _ in }
    clipboard = SharedClipboard { report($0, $1) }
    if #available(macOS 13.0, *) {
      pastedFiles = PastedFiles(clipboard: clipboard) { report($0, $1) }
    }
    report = { [weak self] method, arguments in self?.reported(method, arguments) }
  }

  @available(macOS 13.0, *)
  private var pasted: PastedFiles? { pastedFiles as? PastedFiles }

  /// What macOS lets Zerohop do with copied content right now.
  var access: [String: Any] {
    var files = "unavailable"
    if #available(macOS 13.0, *), let pasted { files = pasted.state }
    return ["access": clipboard.access, "files": files]
  }

  // MARK: Starting

  /// Starts the core once, and calls back when it is running or has failed.
  /// A failure is tried again on the next call.
  func start(_ done: @escaping (Started) -> Void) {
    if started == .running { return done(.running) }
    waiting.append(done)
    guard !starting else { return }
    starting = true
    // Reading the keychain can make macOS ask the owner, so never on the
    // main thread.
    DispatchQueue.global(qos: .userInitiated).async {
      let result = Self.launch()
      DispatchQueue.main.async {
        self.starting = false
        self.started = result
        if result == .running { self.bonjour.browse() }
        let waiting = self.waiting
        self.waiting = []
        waiting.forEach { $0(result) }
      }
    }
  }

  private static func launch() -> Started {
    guard let places = Places.current() else { return .failed }
    var secret: Data?
    // The keychain can be briefly unavailable while the system is starting.
    for attempt in 0..<3 {
      secret = KeyVault.secret()
      if secret != nil { break }
      if attempt < 2 { Thread.sleep(forTimeInterval: 1) }
    }
    guard var secret else { return .keyUnavailable }
    let status = Core.start(stateDir: places.state, folder: places.folder, secret: secret)
    secret.resetBytes(in: 0..<secret.count)
    guard status == 0 else {
      log.error("the core did not start: \(status, privacy: .public)")
      return .failed
    }
    let settings = Core.call("load_settings")?["ok"] as? [String: Any]
    if let bookmark = (settings?["ui"] as? [String: Any])?["folder_bookmark"] as? String {
      _ = Bookmarks.resolve(bookmark)
    }
    // A device key that will not open is the interface's to explain.
    _ = Core.call("bootstrap", ["allow_loopback": false])
    return .running
  }

  // MARK: The interface

  /// The interface of a new window joins, and hears everything from now on.
  func attach(_ link: InterfaceLink, _ done: @escaping (Started) -> Void) {
    start { [weak self, weak link] result in
      if result == .running, let link { self?.link = link }
      done(result)
    }
  }

  /// The window closed. Pairing needs the screen, so it ends too.
  func detach() {
    link = nil
    if pairing { Core.send("pair_cancel") }
  }

  /// What the clipboard and pasted files report, for the interface.
  private func reported(_ method: String, _ arguments: [String: Any]) {
    if method == "clipboardCopied" { return copied(arguments) }
    link?.tell(method, arguments)
  }

  // MARK: What the core says

  /// One message from the core, on the main thread.
  fileprivate func heard(_ message: Data) {
    guard let parsed = (try? JSONSerialization.jsonObject(with: message)) as? [String: Any] else {
      return
    }
    if let reply = parsed["reply"] {
      // Answers to this side's own requests carry the number 0.
      if (reply as? Int) != 0 { link?.deliver(message) }
      return
    }
    handle(parsed)
    link?.deliver(message)
  }

  private func handle(_ event: [String: Any]) {
    switch event["event"] as? String {
    case "snapshot":
      apply(event)
    case "offer_arrived":
      BackgroundMode.shared.showWindow()
    case "clipboard_apply":
      let data = (event["data"] as? String).flatMap { Data(base64Encoded: $0) }
      clipboard.apply(
        format: event["format"] as? String, text: event["text"] as? String, data: data,
        ttlMs: event["ttl_ms"] as? Int ?? 120_000)
    case "clipboard_files":
      offerFiles(event)
    default:
      break
    }
  }

  private func apply(_ snapshot: [String: Any]) {
    pairing = snapshot["pairing"] is [String: Any]
    bonjour.advertise(snapshot["advert"] as? [String: Any])
    let on = (snapshot["settings"] as? [String: Any])?["clipboard"] as? Bool ?? false
    guard on != clipboardOn else { return }
    clipboardOn = on
    BackgroundMode.shared.set(on)
    clipboard.watch(on)
    if on, #available(macOS 13.0, *) { pasted?.check() }
    link?.tell("clipboardAccess", ["state": clipboard.access])
  }

  /// Files copied on another device: only their list has come, and pasting
  /// here fetches them.
  private func offerFiles(_ event: [String: Any]) {
    guard let id = event["id"] as? String else { return }
    let from = event["from"] as? String ?? ""
    guard #available(macOS 13.0, *), let pasted else {
      link?.tell("clipboardFilesOffered", ["from": from, "state": "unavailable"])
      return
    }
    pasted.offer(clip: id, ttlMs: event["ttl_ms"] as? Int ?? 120_000) { [weak self] state in
      self?.link?.tell("clipboardFilesOffered", ["from": from, "state": state])
    }
  }

  /// Something copied on this Mac, for the other devices.
  private func copied(_ clip: [String: Any]) {
    if clip["format"] as? String == "files" {
      return Core.send("clip_files", ["paths": clip["paths"] as? [String] ?? []])
    }
    var params: [String: Any] = ["format": clip["format"] as? String ?? ""]
    if let text = clip["text"] as? String { params["text"] = text }
    if let data = clip["data"] as? Data { params["data"] = data.base64EncodedString() }
    Core.send("clip_changed", params)
  }
}

/// Each message from the core arrives on one of its own threads.
private func received(_ message: UnsafeMutablePointer<CChar>?) {
  guard let message else { return }
  let data = Data(bytes: message, count: strlen(message))
  Core.free(message)
  DispatchQueue.main.async { Service.shared.heard(data) }
}

/// Where this Mac keeps its key and settings, and where files go until the
/// owner picks another folder. These are where the interface kept them
/// before Zerohop ran without it, so nothing moves.
struct Places {
  let state: String
  let folder: String

  static func current() -> Places? {
    let user = FileManager.SearchPathDomainMask.userDomainMask
    guard
      let support = NSSearchPathForDirectoriesInDomains(.applicationSupportDirectory, user, true)
        .first,
      let downloads = NSSearchPathForDirectoriesInDomains(.downloadsDirectory, user, true).first,
      let id = Bundle.main.bundleIdentifier
    else { return nil }
    var state = (support as NSString).appendingPathComponent(id)
    var folder = (downloads as NSString).appendingPathComponent("Zerohop")
    // A second copy for testing keeps a state and a folder of its own.
    if let profile = ProcessInfo.processInfo.environment["ZEROHOP_PROFILE"] {
      state = (state as NSString).appendingPathComponent("profile-\(profile)")
      folder += "-\(profile)"
    }
    return Places(state: state, folder: folder)
  }
}

/// Access to a folder the owner picked, kept across restarts.
enum Bookmarks {
  static func make(_ path: String) -> String? {
    let url = URL(fileURLWithPath: path, isDirectory: true)
    let data = try? url.bookmarkData(
      options: .withSecurityScope, includingResourceValuesForKeys: nil, relativeTo: nil)
    return data?.base64EncodedString()
  }

  /// Regains access for as long as Zerohop runs, and returns the path.
  static func resolve(_ bookmark: String) -> String? {
    guard let data = Data(base64Encoded: bookmark) else { return nil }
    var stale = false
    guard
      let url = try? URL(
        resolvingBookmarkData: data, options: .withSecurityScope, relativeTo: nil,
        bookmarkDataIsStale: &stale),
      url.startAccessingSecurityScopedResource()
    else { return nil }
    return url.path
  }
}

/// The Rust core, through the functions it exports.
enum Core {
  typealias Callback = @convention(c) (UnsafeMutablePointer<CChar>?) -> Void
  private typealias Start = @convention(c) (
    UnsafePointer<CChar>?, UnsafePointer<CChar>?, UnsafePointer<UInt8>?, Int, Callback
  ) -> Int32
  private typealias Request = @convention(c) (UnsafePointer<CChar>?) -> Void
  private typealias Call = @convention(c) (UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
  private typealias Free = @convention(c) (UnsafeMutablePointer<CChar>?) -> Void

  private struct Functions {
    let start: Start
    let request: Request
    let call: Call
    let free: Free
  }

  private static let functions: Functions? = {
    guard let frameworks = Bundle.main.privateFrameworksURL else { return nil }
    let path = frameworks.appendingPathComponent("zerohop_ffi.framework/zerohop_ffi").path
    guard let library = dlopen(path, RTLD_NOW),
      let start = dlsym(library, "zh_start"), let request = dlsym(library, "zh_request"),
      let call = dlsym(library, "zh_call"), let free = dlsym(library, "zh_free")
    else { return nil }
    return Functions(
      start: unsafeBitCast(start, to: Start.self),
      request: unsafeBitCast(request, to: Request.self),
      call: unsafeBitCast(call, to: Call.self),
      free: unsafeBitCast(free, to: Free.self))
  }()

  /// Starts the core with the secret that seals the device key on disk.
  /// Zero means it is running.
  static func start(stateDir: String, folder: String, secret: Data) -> Int32 {
    guard let functions else { return -1 }
    return secret.withUnsafeBytes { bytes in
      functions.start(
        stateDir, folder, bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count, received)
    }
  }

  static func free(_ message: UnsafeMutablePointer<CChar>) {
    functions?.free(message)
  }

  /// Hands a request over without waiting. Its answer is dropped.
  static func send(_ method: String, _ params: [String: Any] = [:]) {
    guard let functions, let text = encode(id: 0, method, params) else { return }
    text.withCString { functions.request($0) }
  }

  /// Waits for the answer, so never on the main thread.
  static func call(_ method: String, _ params: [String: Any] = [:]) -> [String: Any]? {
    dispatchPrecondition(condition: .notOnQueue(.main))
    guard let functions, let text = encode(id: 1, method, params),
      let answer = text.withCString({ functions.call($0) })
    else { return nil }
    defer { functions.free(answer) }
    let data = Data(bytes: answer, count: strlen(answer))
    return (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
  }

  private static func encode(id: Int, _ method: String, _ params: [String: Any]) -> String? {
    let request: [String: Any] = ["id": id, "method": method, "params": params]
    guard let body = try? JSONSerialization.data(withJSONObject: request) else { return nil }
    return String(data: body, encoding: .utf8)
  }
}
