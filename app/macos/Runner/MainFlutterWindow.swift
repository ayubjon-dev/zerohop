import Cocoa
import FlutterMacOS
import ServiceManagement

class MainFlutterWindow: NSWindow {
  override func close() {
    super.close()
    BackgroundMode.shared.windowClosed()
  }

  override func awakeFromNib() {
    let flutterViewController = FlutterViewController()
    let windowFrame = self.frame
    self.contentViewController = flutterViewController
    self.setFrame(windowFrame, display: true)
    self.title = "Zerohop"
    self.minSize = NSSize(width: 380, height: 620)
    self.setContentSize(NSSize(width: 1120, height: 760))
    self.center()

    RegisterGeneratedPlugins(registry: flutterViewController)
    PlatformChannel.register(with: flutterViewController.engine.binaryMessenger)

    super.awakeFromNib()
  }
}

/// The parts of macOS that Flutter does not reach by itself.
enum PlatformChannel {
  private static var channel: FlutterMethodChannel?
  private static let clipboard = SharedClipboard { method, arguments in
    channel?.invokeMethod(method, arguments: arguments)
  }

  static func register(with messenger: FlutterBinaryMessenger) {
    let channel = FlutterMethodChannel(name: "dev.zerohop/platform", binaryMessenger: messenger)
    channel.setMethodCallHandler(handle)
    Self.channel = channel
  }

  private static func handle(_ call: FlutterMethodCall, result: @escaping FlutterResult) {
    let args = call.arguments as? [String: Any]
    switch call.method {
    case "bookmark":
      guard let path = args?["path"] as? String else { return result(nil) }
      let url = URL(fileURLWithPath: path, isDirectory: true)
      let data = try? url.bookmarkData(
        options: .withSecurityScope, includingResourceValuesForKeys: nil, relativeTo: nil)
      result(data?.base64EncodedString())
    case "resolve":
      guard let text = args?["bookmark"] as? String, let data = Data(base64Encoded: text) else {
        return result(nil)
      }
      var stale = false
      guard
        let url = try? URL(
          resolvingBookmarkData: data, options: .withSecurityScope, relativeTo: nil,
          bookmarkDataIsStale: &stale),
        url.startAccessingSecurityScopedResource()
      else { return result(nil) }
      result(url.path)
    case "show":
      guard let path = args?["path"] as? String else { return result(nil) }
      NSWorkspace.shared.open(URL(fileURLWithPath: path, isDirectory: true))
      result(nil)
    case "activate":
      BackgroundMode.shared.showWindow()
      result(nil)
    case "deviceSecret":
      // Reading it can make macOS ask the owner, so never on the main thread.
      DispatchQueue.global(qos: .userInitiated).async {
        let secret = KeyVault.secret()
        DispatchQueue.main.async { result(secret.map { FlutterStandardTypedData(bytes: $0) }) }
      }
    case "background":
      BackgroundMode.shared.set(args?["on"] as? Bool ?? false)
      result(nil)
    case "clipboardWatch":
      clipboard.watch(args?["on"] as? Bool ?? false)
      result(clipboard.access)
    case "clipboardRead":
      result(clipboard.read())
    case "clipboardApply":
      clipboard.apply(args ?? [:])
      result(nil)
    case "clipboardSettings":
      let pane = "x-apple.systempreferences:com.apple.preference.security?Privacy_Pasteboard"
      if let url = URL(string: pane) { NSWorkspace.shared.open(url) }
      result(nil)
    default:
      result(FlutterMethodNotImplemented)
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

  func windowClosed() {
    if isOn { NSApp.setActivationPolicy(.accessory) }
  }

  @objc func showWindow() {
    NSApp.setActivationPolicy(.regular)
    NSApp.windows.first { $0 is MainFlutterWindow }?.makeKeyAndOrderFront(nil)
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

  func read() -> [String: Any] {
    guard let types = board.types, !types.isEmpty else { return ["state": "empty"] }
    if types.contains(where: { Self.notForOthers.contains($0.rawValue) }) {
      return ["state": "secret"]
    }
    // A copied file also offers its name as text, which is not what was meant.
    let fileOnly: [NSPasteboard.ReadingOptionKey: Any] = [.urlReadingFileURLsOnly: true]
    if types.contains(.fileURL) || board.canReadObject(forClasses: [NSURL.self], options: fileOnly) {
      return ["state": "files"]
    }
    if types.contains(.string) {
      guard let text = board.string(forType: .string) else { return ["state": "denied"] }
      if text.isEmpty { return ["state": "empty"] }
      return ["state": "ok", "format": "text", "text": text]
    }
    guard types.contains(.png) || types.contains(.tiff) else { return ["state": "empty"] }
    guard let png = readPng() else { return ["state": "denied"] }
    if png.count > Self.maxImage { return ["state": "too_large"] }
    return ["state": "ok", "format": "png", "data": FlutterStandardTypedData(bytes: png)]
  }

  private func readPng() -> Data? {
    if let png = board.data(forType: .png) { return png }
    guard let tiff = board.data(forType: .tiff) else { return nil }
    return NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:])
  }

  func apply(_ arguments: [String: Any]) {
    let format = arguments["format"] as? String
    let item = NSPasteboardItem()
    if format == "text", let text = arguments["text"] as? String {
      item.setString(text, forType: .string)
    } else if let image = (arguments["data"] as? FlutterStandardTypedData)?.data {
      let png =
        format == "png"
        ? image : NSBitmapImageRep(data: image)?.representation(using: .png, properties: [:])
      guard let png else { return }
      item.setData(png, forType: .png)
    } else {
      return
    }
    // Clipboard managers leave it alone, and Universal Clipboard does not carry it on.
    item.setData(Data(), forType: Self.transient)
    board.prepareForNewContents(with: .currentHostOnly)
    guard board.writeObjects([item]) else { return }
    let stamp = board.changeCount
    ours = stamp
    seen = stamp
    expiry?.invalidate()
    let ttl = Double(arguments["ttl_ms"] as? Int ?? 120_000) / 1000
    let timer = Timer(timeInterval: ttl, repeats: false) { [weak self] _ in
      self?.clearIfStillOurs()
    }
    RunLoop.main.add(timer, forMode: .common)
    expiry = timer
  }

  /// Anything copied since belongs to the owner and is left alone.
  private func clearIfStillOurs() {
    guard let ours, board.changeCount == ours else { return }
    board.clearContents()
    seen = board.changeCount
    self.ours = nil
  }
}
