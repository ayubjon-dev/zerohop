import Flutter
import UIKit

@main
@objc class AppDelegate: FlutterAppDelegate, FlutterImplicitEngineDelegate {
  override func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
  ) -> Bool {
    return super.application(application, didFinishLaunchingWithOptions: launchOptions)
  }

  func didInitializeImplicitFlutterEngine(_ engineBridge: FlutterImplicitEngineBridge) {
    GeneratedPluginRegistrant.register(with: engineBridge.pluginRegistry)
    PlatformChannel.register(with: engineBridge.applicationRegistrar.messenger())
  }
}

/// The parts of iOS that Flutter does not reach by itself.
enum PlatformChannel {
  static func register(with messenger: FlutterBinaryMessenger) {
    let channel = FlutterMethodChannel(name: "dev.zerohop/platform", binaryMessenger: messenger)
    channel.setMethodCallHandler(handle)
  }

  private static func handle(_ call: FlutterMethodCall, result: @escaping FlutterResult) {
    let args = call.arguments as? [String: Any]
    switch call.method {
    case "deviceSecret":
      DispatchQueue.global(qos: .userInitiated).async {
        let secret = KeyVault.secret()
        DispatchQueue.main.async { result(secret.map { FlutterStandardTypedData(bytes: $0) }) }
      }
    case "deviceName":
      // Since iOS 16 this is only "iPhone" or "iPad", which the owner can change.
      result(UIDevice.current.name)
    case "show":
      guard let path = args?["path"] as? String else { return result(nil) }
      FilesApp.show(path)
      result(nil)
    case "keepAwake":
      Awake.shared.set(args?["on"] as? Bool ?? false)
      result(nil)
    default:
      result(FlutterMethodNotImplemented)
    }
  }
}

/// Keeps the secret that seals the device key on disk in the keychain. It
/// stays on this device: it is left out of backups and never moves to a new
/// phone, so a copy of the app's files cannot open the key anywhere else.
enum KeyVault {
  private static let service = "dev.zerohop.device"
  private static let account = "sealing-secret"
  private static let size = 32
  /// Kept with the app's own data, which iOS removes along with the app.
  private static let installed = "keychainClaimed"

  private static var item: [String: Any] {
    [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: service,
      kSecAttrAccount as String: account,
    ]
  }

  /// Nil means something may pass, such as the keychain not being ready yet.
  /// Nothing is replaced then: a new secret would leave the device key
  /// impossible to open.
  static func secret() -> Data? {
    // Keychain items outlive the app. A secret left by an earlier install
    // belongs to keys that are gone, so a fresh install starts without it.
    let defaults = UserDefaults.standard
    if !defaults.bool(forKey: installed) {
      let removed = SecItemDelete(item as CFDictionary)
      guard removed == errSecSuccess || removed == errSecItemNotFound else { return nil }
      defaults.set(true, forKey: installed)
    }
    var query = item
    query[kSecReturnData as String] = true
    query[kSecMatchLimit as String] = kSecMatchLimitOne
    var found: CFTypeRef?
    let status = SecItemCopyMatching(query as CFDictionary, &found)
    if status == errSecSuccess { return (found as? Data).flatMap { $0.count == size ? $0 : nil } }
    guard status == errSecItemNotFound else { return nil }
    var secret = Data(count: size)
    let made = secret.withUnsafeMutableBytes { bytes in
      SecRandomCopyBytes(kSecRandomDefault, size, bytes.baseAddress!)
    }
    guard made == errSecSuccess else { return nil }
    // Readable once the phone has been unlocked after starting, so that the
    // app can still start if iOS launches it ahead of time while locked.
    var new = item
    new[kSecAttrLabel as String] = "Zerohop device key"
    new[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
    new[kSecValueData as String] = secret
    let added = SecItemAdd(new as CFDictionary, nil)
    return added == errSecSuccess ? secret : nil
  }
}

/// Received files sit in the app's documents, which the Files app lists
/// under the app's name.
enum FilesApp {
  /// Opens the Files app at a folder. Received files are never opened.
  static func show(_ path: String) {
    let file = URL(fileURLWithPath: path, isDirectory: true).absoluteString
    guard file.hasPrefix("file://"),
      let url = URL(string: "shareddocuments://" + file.dropFirst("file://".count))
    else { return }
    UIApplication.shared.open(url)
  }
}

/// Keeps the screen on while files are moving. If the owner leaves the app
/// meanwhile, iOS allows a short time to finish before it stops the app.
final class Awake {
  static let shared = Awake()

  private var task: UIBackgroundTaskIdentifier = .invalid

  func set(_ on: Bool) {
    UIApplication.shared.isIdleTimerDisabled = on
    if on, task == .invalid {
      task = UIApplication.shared.beginBackgroundTask(withName: "Zerohop transfer") {
        [weak self] in self?.endTask()
      }
    } else if !on {
      endTask()
    }
  }

  private func endTask() {
    guard task != .invalid else { return }
    UIApplication.shared.endBackgroundTask(task)
    task = .invalid
  }
}
