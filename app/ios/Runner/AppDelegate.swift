import AVFoundation
import Flutter
import PhotosUI
import UIKit
import UniformTypeIdentifiers

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
    PhotoPicker.removeLeftovers()
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
    case "pickPhotos":
      PhotoPicker.shared.pick(result)
    case "scanQr":
      QrScanner.scan(words: args ?? [:], result: result)
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

/// Lets the owner pick photos and videos without giving the app the rest of
/// the library, so iOS asks no question. Each one is copied byte for byte,
/// as iOS hands it over, into a folder of its own in the temporary folder.
/// Dart removes the copy once it is sent or put aside. iOS hands over the
/// file as stored, except that it writes in a date or time zone the library
/// keeps apart from the file, which means a re-encoded copy.
final class PhotoPicker: NSObject, PHPickerViewControllerDelegate,
  UIAdaptivePresentationControllerDelegate
{
  static let shared = PhotoPicker()

  /// Dart looks for this folder too when it removes a copy.
  private static let folder = FileManager.default.temporaryDirectory
    .appendingPathComponent("Photos", isDirectory: true)

  private var pending: FlutterResult?

  /// Nothing picked survives a restart, so copies from an earlier run only
  /// take up room.
  static func removeLeftovers() {
    try? FileManager.default.removeItem(at: folder)
  }

  func pick(_ result: @escaping FlutterResult) {
    guard pending == nil, let top = Self.topController() else {
      return result(FlutterError(code: "unavailable", message: nil, details: nil))
    }
    var config = PHPickerConfiguration()
    config.filter = .any(of: [.images, .videos])
    config.selectionLimit = 0
    config.selection = .ordered
    // As stored, never a copy converted for compatibility (protocol 10.5).
    config.preferredAssetRepresentationMode = .current
    let picker = PHPickerViewController(configuration: config)
    picker.delegate = self
    picker.presentationController?.delegate = self
    pending = result
    top.present(picker, animated: true)
  }

  func picker(_ picker: PHPickerViewController, didFinishPicking results: [PHPickerResult]) {
    picker.dismiss(animated: true)
    guard let result = pending else { return }
    pending = nil
    Self.copy(results.map(\.itemProvider)) { paths, failed in
      result(["paths": paths, "failed": failed])
    }
  }

  /// Swiping the picker away cancels it too.
  func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
    guard let result = pending else { return }
    pending = nil
    result(["paths": [String](), "failed": 0])
  }

  /// Keeps the order the owner picked in. iOS downloads an original held
  /// only in iCloud before it hands it over; one it cannot get is counted
  /// as failed, never replaced.
  private static func copy(
    _ providers: [NSItemProvider], done: @escaping ([String], Int) -> Void
  ) {
    var paths = [String?](repeating: nil, count: providers.count)
    var failed = 0
    let lock = NSLock()
    let group = DispatchGroup()
    for (index, provider) in providers.enumerated() {
      guard let type = original(provider) else {
        lock.lock()
        failed += 1
        lock.unlock()
        continue
      }
      let name = provider.suggestedName
      group.enter()
      _ = provider.loadFileRepresentation(forTypeIdentifier: type.identifier) { lent, _ in
        // The file is lent only until this returns.
        let path = lent.flatMap { keep($0, name: name, type: type) }
        lock.lock()
        if let path { paths[index] = path } else { failed += 1 }
        lock.unlock()
        group.leave()
      }
    }
    group.notify(queue: .main) { done(paths.compactMap { $0 }, failed) }
  }

  /// The photo or video itself, offered first among its forms. A Live
  /// Photo's motion is a separate part and stays behind.
  private static func original(_ provider: NSItemProvider) -> UTType? {
    provider.registeredTypeIdentifiers.lazy.compactMap { UTType($0) }.first {
      ($0.conforms(to: .image) || $0.conforms(to: .movie)) && !$0.conforms(to: .livePhoto)
    }
  }

  /// A folder of its own lets two photos with the same name travel together.
  private static func keep(_ lent: URL, name: String?, type: UTType) -> String? {
    let ext = lent.pathExtension.isEmpty ? type.preferredFilenameExtension ?? "" : lent.pathExtension
    var base = (name ?? "").replacingOccurrences(of: "/", with: "_")
      .replacingOccurrences(of: "\0", with: "")
      .trimmingCharacters(in: .whitespacesAndNewlines)
    if base.isEmpty || base == "." || base == ".." {
      base = lent.deletingPathExtension().lastPathComponent
    }
    let file =
      ext.isEmpty || (base as NSString).pathExtension.lowercased() == ext.lowercased()
      ? base : "\(base).\(ext)"
    let own = folder.appendingPathComponent(UUID().uuidString, isDirectory: true)
    let copy = own.appendingPathComponent(file, isDirectory: false)
    do {
      try FileManager.default.createDirectory(at: own, withIntermediateDirectories: true)
      try FileManager.default.copyItem(at: lent, to: copy)
      return copy.path
    } catch {
      try? FileManager.default.removeItem(at: own)
      return nil
    }
  }

  static func topController() -> UIViewController? {
    let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
    let windows = scenes.flatMap(\.windows)
    var top = (windows.first { $0.isKeyWindow } ?? windows.first)?.rootViewController
    while let presented = top?.presentedViewController { top = presented }
    return top
  }
}

/// Reads the pairing QR code shown by another device, with the system's own
/// camera and code reader. Only a Zerohop pairing code is handed back; any
/// other code is ignored, with a short note, and the camera keeps looking.
final class QrScanner: UIViewController, AVCaptureMetadataOutputObjectsDelegate {
  private static let prefix = "zerohop://pair/"

  private let session = AVCaptureSession()
  private let queue = DispatchQueue(label: "dev.zerohop.camera")
  private let words: [String: Any]
  private var result: FlutterResult?
  private let preview = AVCaptureVideoPreviewLayer()
  private let shade = CAShapeLayer()
  private let hint = UILabel()
  private var hintTimer: Timer?

  /// The side of the square the owner aims at, as a share of the width.
  private static let frameShare: CGFloat = 0.7

  static func scan(words: [String: Any], result: @escaping FlutterResult) {
    // No reason to ask for a camera that is not there.
    guard AVCaptureDevice.default(for: .video) != nil else {
      return result(FlutterError(code: "no_camera", message: nil, details: nil))
    }
    switch AVCaptureDevice.authorizationStatus(for: .video) {
    case .authorized:
      present(words: words, result: result)
    case .notDetermined:
      AVCaptureDevice.requestAccess(for: .video) { granted in
        DispatchQueue.main.async {
          if granted {
            present(words: words, result: result)
          } else {
            result(FlutterError(code: "camera_denied", message: nil, details: nil))
          }
        }
      }
    default:
      result(FlutterError(code: "camera_denied", message: nil, details: nil))
    }
  }

  private static func present(words: [String: Any], result: @escaping FlutterResult) {
    guard let top = PhotoPicker.topController() else {
      return result(FlutterError(code: "unavailable", message: nil, details: nil))
    }
    let scanner = QrScanner(words: words, result: result)
    scanner.modalPresentationStyle = .fullScreen
    top.present(scanner, animated: true)
  }

  private init(words: [String: Any], result: @escaping FlutterResult) {
    self.words = words
    self.result = result
    super.init(nibName: nil, bundle: nil)
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("not used") }

  override var preferredStatusBarStyle: UIStatusBarStyle { .lightContent }

  override func viewDidLoad() {
    super.viewDidLoad()
    view.backgroundColor = .black
    let camera =
      AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .back)
      ?? AVCaptureDevice.default(for: .video)
    guard let camera, let input = try? AVCaptureDeviceInput(device: camera),
      session.canAddInput(input)
    else {
      DispatchQueue.main.async { self.finish(FlutterError(code: "no_camera", message: nil, details: nil)) }
      return
    }
    session.addInput(input)
    let output = AVCaptureMetadataOutput()
    if session.canAddOutput(output) {
      session.addOutput(output)
      output.setMetadataObjectsDelegate(self, queue: .main)
      output.metadataObjectTypes = [.qr]
    }
    preview.session = session
    preview.videoGravity = .resizeAspectFill
    view.layer.addSublayer(preview)
    shade.fillRule = .evenOdd
    shade.fillColor = UIColor(white: 0, alpha: 0.55).cgColor
    view.layer.addSublayer(shade)
    addLabels()
  }

  private func addLabels() {
    let title = UILabel()
    title.text = words["title"] as? String
    title.textColor = .white
    title.font = .preferredFont(forTextStyle: .headline)
    title.adjustsFontForContentSizeCategory = true
    title.numberOfLines = 0
    title.textAlignment = .center
    hint.textColor = .white
    hint.font = .preferredFont(forTextStyle: .subheadline)
    hint.adjustsFontForContentSizeCategory = true
    hint.textAlignment = .center
    hint.numberOfLines = 0
    hint.alpha = 0
    var config = UIButton.Configuration.filled()
    config.title = words["cancel"] as? String
    config.baseBackgroundColor = UIColor(white: 1, alpha: 0.18)
    config.baseForegroundColor = .white
    config.cornerStyle = .capsule
    config.contentInsets = NSDirectionalEdgeInsets(top: 14, leading: 28, bottom: 14, trailing: 28)
    let cancel = UIButton(configuration: config)
    cancel.addAction(UIAction { [weak self] _ in self?.finish(nil) }, for: .touchUpInside)
    for item in [title, hint, cancel] as [UIView] {
      item.translatesAutoresizingMaskIntoConstraints = false
      view.addSubview(item)
    }
    let guide = view.safeAreaLayoutGuide
    NSLayoutConstraint.activate([
      title.topAnchor.constraint(equalTo: guide.topAnchor, constant: 32),
      title.leadingAnchor.constraint(equalTo: guide.leadingAnchor, constant: 24),
      title.trailingAnchor.constraint(equalTo: guide.trailingAnchor, constant: -24),
      cancel.bottomAnchor.constraint(equalTo: guide.bottomAnchor, constant: -24),
      cancel.centerXAnchor.constraint(equalTo: guide.centerXAnchor),
      hint.bottomAnchor.constraint(equalTo: cancel.topAnchor, constant: -20),
      hint.leadingAnchor.constraint(equalTo: guide.leadingAnchor, constant: 24),
      hint.trailingAnchor.constraint(equalTo: guide.trailingAnchor, constant: -24),
    ])
  }

  override func viewDidLayoutSubviews() {
    super.viewDidLayoutSubviews()
    preview.frame = view.bounds
    shade.frame = view.bounds
    let side = min(view.bounds.width, view.bounds.height) * Self.frameShare
    let square = CGRect(
      x: (view.bounds.width - side) / 2, y: (view.bounds.height - side) / 2,
      width: side, height: side)
    let path = UIBezierPath(rect: view.bounds)
    path.append(UIBezierPath(roundedRect: square, cornerRadius: 24))
    shade.path = path.cgPath
  }

  override func viewWillAppear(_ animated: Bool) {
    super.viewWillAppear(animated)
    queue.async { [session] in session.startRunning() }
  }

  override func viewWillDisappear(_ animated: Bool) {
    super.viewWillDisappear(animated)
    queue.async { [session] in session.stopRunning() }
  }

  func metadataOutput(
    _ output: AVCaptureMetadataOutput, didOutput objects: [AVMetadataObject],
    from connection: AVCaptureConnection
  ) {
    for case let code as AVMetadataMachineReadableCodeObject in objects {
      guard let text = code.stringValue else { continue }
      if text.hasPrefix(Self.prefix) {
        UINotificationFeedbackGenerator().notificationOccurred(.success)
        finish(text)
        return
      }
      showNotZerohop()
    }
  }

  private func showNotZerohop() {
    hint.text = words["notZerohop"] as? String
    hint.alpha = 1
    hintTimer?.invalidate()
    hintTimer = Timer.scheduledTimer(withTimeInterval: 2, repeats: false) { [weak self] _ in
      UIView.animate(withDuration: 0.3) { self?.hint.alpha = 0 }
    }
  }

  /// Hands back the code, nil for a cancel, or an error, exactly once.
  private func finish(_ value: Any?) {
    guard let result else { return }
    self.result = nil
    hintTimer?.invalidate()
    queue.async { [session] in session.stopRunning() }
    dismiss(animated: true) { result(value) }
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
