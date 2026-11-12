import Cocoa
import FlutterMacOS
import LocalAuthentication
import ServiceManagement

class MainFlutterWindow: NSWindow {
  override func close() {
    super.close()
    BackgroundMode.shared.windowClosed(self)
  }

  /// Made in code rather than in the nib, because a window from the nib can
  /// never be let go. A new one opens where the last one was.
  static func make(frame: NSRect?) -> MainFlutterWindow {
    let window = MainFlutterWindow(
      contentRect: NSRect(x: 0, y: 0, width: 1120, height: 760),
      styleMask: [.titled, .closable, .miniaturizable, .resizable],
      backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    window.showInterface()
    window.setUp(frame: frame)
    return window
  }

  private func setUp(frame: NSRect?) {
    title = "Zerohop"
    minSize = NSSize(width: 380, height: 620)
    if let frame {
      setFrame(frame, display: false)
    } else {
      setContentSize(NSSize(width: 1120, height: 760))
      center()
    }
  }

  /// Puts the interface into the window, on an engine of its own.
  func showInterface() {
    guard contentViewController == nil else { return }
    let frame = self.frame
    let engine = Interface.shared.engine()
    contentViewController = FlutterViewController(engine: engine, nibName: nil, bundle: nil)
    setFrame(frame, display: false)
    // After the view exists, so that Dart starts with one.
    Interface.shared.run()
  }

  /// Lets the interface go, with all of Flutter. The core keeps running, so
  /// files and the clipboard still arrive.
  func dropInterface() {
    contentViewController = nil
    // AppKit lets go of a view, and of the surfaces Flutter drew into, only
    // once the view has left its window. A window that goes away with the
    // view still in it can leave the view behind.
    contentView = NSView()
    Interface.shared.shutDown()
  }
}

/// Flutter, only while the window is open. Its engine, the surfaces it
/// draws into and Dart's memory all go when the window closes.
final class Interface {
  static let shared = Interface()

  private var current: FlutterEngine?
  private var link: InterfaceLink?

  /// The engine for the window being opened.
  func engine() -> FlutterEngine {
    if let current { return current }
    let engine = FlutterEngine(name: "zerohop", project: nil, allowHeadlessExecution: true)
    current = engine
    return engine
  }

  func run() {
    guard let engine = current, link == nil else { return }
    engine.run(withEntrypoint: nil)
    RegisterGeneratedPlugins(registry: engine)
    link = InterfaceLink(messenger: engine.binaryMessenger)
  }

  func shutDown() {
    guard let engine = current else { return }
    link?.close()
    link = nil
    Service.shared.detach()
    engine.shutDownEngine()
    current = nil
    // What Flutter freed still counts as Zerohop's memory until it is handed
    // back to the system, which otherwise waits for the Mac to run short.
    DispatchQueue.main.asyncAfter(deadline: .now() + Self.settleSeconds) {
      _ = malloc_zone_pressure_relief(nil, 0)
    }
  }

  /// Flutter's own threads finish letting go shortly after it shuts down.
  private static let settleSeconds = 3.0
}

/// What the interface asks of macOS, and what it hears from the core while
/// its window is open.
final class InterfaceLink {
  private let platform: FlutterMethodChannel
  private let core: FlutterBasicMessageChannel

  init(messenger: FlutterBinaryMessenger) {
    platform = FlutterMethodChannel(name: "dev.zerohop/platform", binaryMessenger: messenger)
    core = FlutterBasicMessageChannel(
      name: "dev.zerohop/core", binaryMessenger: messenger,
      codec: FlutterBinaryCodec.sharedInstance())
    platform.setMethodCallHandler { [weak self] call, result in
      self?.handle(call, result: result)
    }
  }

  func close() {
    platform.setMethodCallHandler(nil)
  }

  /// A message from the core, as the core wrote it.
  func deliver(_ message: Data) {
    core.sendMessage(message)
  }

  /// Something the clipboard or pasted files report.
  func tell(_ method: String, _ arguments: [String: Any]) {
    platform.invokeMethod(method, arguments: arguments)
  }

  private func handle(_ call: FlutterMethodCall, result: @escaping FlutterResult) {
    let args = call.arguments as? [String: Any]
    switch call.method {
    case "attach":
      Service.shared.attach(self) { started in
        var answer = Service.shared.access
        answer["state"] = started.rawValue
        result(answer)
      }
    case "bookmark":
      guard let path = args?["path"] as? String else { return result(nil) }
      result(Bookmarks.make(path))
    case "resolve":
      guard let bookmark = args?["bookmark"] as? String else { return result(nil) }
      result(Bookmarks.resolve(bookmark))
    case "show":
      guard let path = args?["path"] as? String else { return result(nil) }
      NSWorkspace.shared.open(URL(fileURLWithPath: path, isDirectory: true))
      result(nil)
    case "fileProviderSettings":
      if #available(macOS 13.0, *) { PastedFiles.openSettings() }
      result(nil)
    case "confirmOwner":
      OwnerCheck.run(reason: args?["reason"] as? String ?? "", result: result)
    case "clipboardSettings":
      let pane = "x-apple.systempreferences:com.apple.preference.security?Privacy_Pasteboard"
      if let url = URL(string: pane) { NSWorkspace.shared.open(url) }
      result(nil)
    default:
      result(FlutterMethodNotImplemented)
    }
  }
}

/// The system's own check that the owner is present, before Zerohop signs a
/// revocation: Touch ID or the login password. A device with no password has
/// nothing to check with, which Dart treats as allowed.
enum OwnerCheck {
  static func run(reason: String, result: @escaping FlutterResult) {
    let context = LAContext()
    var problem: NSError?
    guard context.canEvaluatePolicy(.deviceOwnerAuthentication, error: &problem) else {
      let unset =
        problem?.domain == LAErrorDomain && problem?.code == LAError.Code.passcodeNotSet.rawValue
      result(unset ? "unavailable" : "refused")
      return
    }
    context.evaluatePolicy(.deviceOwnerAuthentication, localizedReason: reason) { passed, _ in
      // The context has to live until the answer arrives.
      _ = context
      DispatchQueue.main.async { result(passed ? "passed" : "refused") }
    }
  }
}

/// Keeps the secret that seals the device key on disk in the login keychain,
/// where only this app can read it without the owner being asked.
enum KeyVault {
  private static let service = "dev.zerohop.device"
  private static let account = "sealing-secret"
  private static let size = 32

  /// Nil means something may pass, such as the owner declining. Nothing is
  /// replaced then: a new secret would leave the device key impossible to open.
  static func secret() -> Data? {
    var found: CFTypeRef?
    let status = SecItemCopyMatching(
      [
        kSecClass as String: kSecClassGenericPassword,
        kSecAttrService as String: service,
        kSecAttrAccount as String: account,
        kSecReturnData as String: true,
        kSecMatchLimit as String: kSecMatchLimitOne,
      ] as CFDictionary, &found)
    if status == errSecSuccess { return (found as? Data).flatMap { $0.count == size ? $0 : nil } }
    guard status == errSecItemNotFound else { return nil }
    var secret = Data(count: size)
    let made = secret.withUnsafeMutableBytes { bytes in
      SecRandomCopyBytes(kSecRandomDefault, size, bytes.baseAddress!)
    }
    guard made == errSecSuccess else { return nil }
    let added = SecItemAdd(
      [
        kSecClass as String: kSecClassGenericPassword,
        kSecAttrService as String: service,
        kSecAttrAccount as String: account,
        kSecAttrLabel as String: "Zerohop device key",
        kSecValueData as String: secret,
      ] as CFDictionary, nil)
    return added == errSecSuccess ? secret : nil
  }
}

/// Keeps the app working with its window closed, for as long as the owner
/// has the shared clipboard turned on.
final class BackgroundMode: NSObject {
  static let shared = BackgroundMode()

  private(set) var isOn = false
  private var statusItem: NSStatusItem?

  func set(_ on: Bool) {
    guard on != isOn else { return }
    isOn = on
    if on {
      addStatusItem()
    } else {
      if let statusItem { NSStatusBar.system.removeStatusItem(statusItem) }
      statusItem = nil
      NSApp.setActivationPolicy(.regular)
    }
    setStartsAtLogin(on)
  }

  /// Where the window was when it was let go.
  private var lastFrame: NSRect?

  /// A window made in code lives only as long as something holds it.
  private var window: MainFlutterWindow?

  /// Started by the system at login, nothing is shown and Flutter does not
  /// start: the core runs on its own until the owner opens Zerohop.
  func start(visible: Bool) {
    Service.shared.start { _ in }
    guard visible else {
      NSApp.setActivationPolicy(.accessory)
      return
    }
    let window = MainFlutterWindow.make(frame: nil)
    self.window = window
    (NSApp.delegate as? FlutterAppDelegate)?.mainFlutterWindow = window
    window.makeKeyAndOrderFront(nil)
  }

  /// A closed window keeps the last picture it showed, and the surfaces
  /// behind it, until it draws again. So the window goes too, with all of
  /// Flutter, and a new one is made when the owner opens Zerohop.
  func windowClosed(_ window: MainFlutterWindow) {
    guard isOn else { return }
    NSApp.setActivationPolicy(.accessory)
    lastFrame = window.frame
    window.dropInterface()
    if let delegate = NSApp.delegate as? FlutterAppDelegate, delegate.mainFlutterWindow === window {
      delegate.mainFlutterWindow = nil
    }
    if self.window === window { self.window = nil }
  }

  @objc func showWindow() {
    NSApp.setActivationPolicy(.regular)
    let window =
      NSApp.windows.first(where: { $0 is MainFlutterWindow && $0.contentViewController != nil })
      as? MainFlutterWindow ?? MainFlutterWindow.make(frame: lastFrame)
    window.showInterface()
    self.window = window
    (NSApp.delegate as? FlutterAppDelegate)?.mainFlutterWindow = window
    window.makeKeyAndOrderFront(nil)
    NSApp.activate(ignoringOtherApps: true)
  }

  private func addStatusItem() {
    let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
    item.button?.image = NSImage(
      systemSymbolName: "doc.on.clipboard", accessibilityDescription: "Zerohop")
    let russian = Locale.preferredLanguages.first?.hasPrefix("ru") == true
    let menu = NSMenu()
    let open = NSMenuItem(
      title: russian ? "Открыть Zerohop" : "Open Zerohop",
      action: #selector(showWindow), keyEquivalent: "")
    open.target = self
    menu.addItem(open)
    menu.addItem(.separator())
    menu.addItem(
      NSMenuItem(
        title: russian ? "Выйти из Zerohop" : "Quit Zerohop",
        action: #selector(NSApplication.terminate(_:)), keyEquivalent: ""))
    item.menu = menu
    statusItem = item
  }

  private func setStartsAtLogin(_ on: Bool) {
    guard #available(macOS 13.0, *) else { return }
    do {
      if on {
        try SMAppService.mainApp.register()
      } else {
        try SMAppService.mainApp.unregister()
      }
    } catch {
      // Nothing depends on it: the app still works once it has been opened.
    }
  }
}

/// Copying on this Mac and pasting on another device, and the other way round.
/// Only the change counter is watched; contents are read once something changed.
final class SharedClipboard {
  private static let notForOthers: Set<String> = [
    "org.nspasteboard.ConcealedType",
    "org.nspasteboard.TransientType",
    "org.nspasteboard.AutoGeneratedType",
  ]
  private static let transient = NSPasteboard.PasteboardType("org.nspasteboard.TransientType")
  private static let maxImage = 24 * 1024 * 1024
  /// Files and folders picked at once in Finder, not counting what they hold.
  private static let maxFiles = 1000
  private static let tickSeconds = 0.5
  private static let ticksPerAccessCheck = 4

  private let board = NSPasteboard.general
  private let report: (String, [String: Any]) -> Void
  private var timer: Timer?
  private var activity: NSObjectProtocol?
  private var seen = 0
  private var ours: Int?
  private var expiry: Timer?
  private var ticks = 0
  private var lastAccess = ""

  init(report: @escaping (String, [String: Any]) -> Void) {
    self.report = report
    NotificationCenter.default.addObserver(
      forName: NSApplication.willTerminateNotification, object: nil, queue: .main
    ) { [weak self] _ in self?.clearIfStillOurs() }
  }

  /// What macOS lets this app do without asking.
  var access: String {
    if #available(macOS 15.4, *) {
      switch board.accessBehavior {
      case .alwaysAllow: return "allow"
      case .alwaysDeny: return "deny"
      case .ask: return "ask"
      default: return "default"
      }
    }
    return "allow"
  }

  func watch(_ on: Bool) {
    timer?.invalidate()
    timer = nil
    if let activity { ProcessInfo.processInfo.endActivity(activity) }
    activity = nil
    guard on else { return }
    // App Nap would hold back the checks and the removal of expired clips
    // for as long as the window is out of sight.
    activity = ProcessInfo.processInfo.beginActivity(
      options: .userInitiatedAllowingIdleSystemSleep, reason: "Shared clipboard")
    // What was copied before this was turned on stays on this Mac.
    seen = board.changeCount
    lastAccess = access
    let timer = Timer(timeInterval: Self.tickSeconds, repeats: true) { [weak self] _ in
      self?.tick()
    }
    timer.tolerance = Self.tickSeconds / 2
    RunLoop.main.add(timer, forMode: .common)
    self.timer = timer
  }

  private func tick() {
    ticks += 1
    if ticks % Self.ticksPerAccessCheck == 0, access != lastAccess {
      lastAccess = access
      report("clipboardAccess", ["state": lastAccess])
    }
    let count = board.changeCount
    guard count != seen else { return }
    seen = count
    guard count != ours else { return }
    // Asking on every copy would be unbearable, so after the first time the
    // owner has to allow it in System Settings. Until then copies stay here.
    guard access == "allow" || access == "default" else { return }
    let clip = read()
    lastAccess = access
    if clip["state"] as? String == "ok" { report("clipboardCopied", clip) }
  }

  private func read() -> [String: Any] {
    guard let types = board.types, !types.isEmpty else { return ["state": "empty"] }
    if types.contains(where: { Self.notForOthers.contains($0.rawValue) }) {
      return ["state": "secret"]
    }
    // A copied file also offers its name as text, which is not what was meant.
    // Only the list goes now: the files move when the owner pastes them.
    let fileOnly: [NSPasteboard.ReadingOptionKey: Any] = [.urlReadingFileURLsOnly: true]
    if types.contains(.fileURL) || board.canReadObject(forClasses: [NSURL.self], options: fileOnly) {
      let urls = board.readObjects(forClasses: [NSURL.self], options: fileOnly) as? [URL] ?? []
      let paths = urls.filter(\.isFileURL).map(\.path)
      if paths.isEmpty { return ["state": "empty"] }
      if paths.count > Self.maxFiles { return ["state": "too_large"] }
      return ["state": "ok", "format": "files", "paths": paths]
    }
    if types.contains(.string) {
      guard let text = board.string(forType: .string) else { return ["state": "denied"] }
      if text.isEmpty { return ["state": "empty"] }
      return ["state": "ok", "format": "text", "text": text]
    }
    guard types.contains(.png) || types.contains(.tiff) else { return ["state": "empty"] }
    guard let png = readPng() else { return ["state": "denied"] }
    if png.count > Self.maxImage { return ["state": "too_large"] }
    return ["state": "ok", "format": "png", "data": png]
  }

  private func readPng() -> Data? {
    if let png = board.data(forType: .png) { return png }
    guard let tiff = board.data(forType: .tiff) else { return nil }
    return NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:])
  }

  /// Puts what another device copied on the clipboard, for its lifetime.
  func apply(format: String?, text: String?, data: Data?, ttlMs: Int) {
    let item = NSPasteboardItem()
    if format == "text", let text {
      item.setString(text, forType: .string)
    } else if let data {
      let png =
        format == "png"
        ? data : NSBitmapImageRep(data: data)?.representation(using: .png, properties: [:])
      guard let png else { return }
      item.setData(png, forType: .png)
    } else {
      return
    }
    write([item], ttl: Double(ttlMs) / 1000)
  }

  /// Puts files copied on another device on the clipboard: placeholders
  /// whose contents come only when something reads them.
  func applyFiles(_ urls: [URL], ttl: TimeInterval) -> Bool {
    let items = urls.map { url -> NSPasteboardItem in
      let item = NSPasteboardItem()
      item.setString(url.absoluteString, forType: .fileURL)
      return item
    }
    return write(items, ttl: ttl)
  }

  @discardableResult
  private func write(_ items: [NSPasteboardItem], ttl: TimeInterval) -> Bool {
    // Clipboard managers leave it alone, the watch here does not send it
    // back, and Universal Clipboard does not carry it on.
    items.first?.setData(Data(), forType: Self.transient)
    board.prepareForNewContents(with: .currentHostOnly)
    guard board.writeObjects(items) else { return false }
    let stamp = board.changeCount
    ours = stamp
    seen = stamp
    expiry?.invalidate()
    let timer = Timer(timeInterval: ttl, repeats: false) { [weak self] _ in
      self?.clearIfStillOurs()
    }
    RunLoop.main.add(timer, forMode: .common)
    expiry = timer
    return true
  }

  /// Anything copied since belongs to the owner and is left alone.
  func clearIfStillOurs() {
    guard let ours, board.changeCount == ours else { return }
    board.clearContents()
    seen = board.changeCount
    self.ours = nil
  }
}
