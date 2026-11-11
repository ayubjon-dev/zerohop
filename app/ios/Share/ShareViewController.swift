import UIKit
import UniformTypeIdentifiers

/// Zerohop in the system's Share menu. It copies what the owner shared into
/// the space the app reaches and asks them to open Zerohop, where they choose
/// the device and tap Send. Nothing leaves the phone from here, and iOS does
/// not let an extension like this one open its app.
final class ShareViewController: UIViewController {
  private static let minFreeBytes: Int64 = 200 * 1024 * 1024

  private enum Outcome {
    case ready(failed: Int)
    case nothing
    case noRoom
    case failed
    case unavailable
  }

  private let spinner = UIActivityIndicatorView(style: .large)
  private let message = UILabel()
  private let button = UIButton(type: .system)
  private let lock = NSLock()
  private var stopped = false
  private var loading: Progress?
  private var share: URL?
  private var done = false

  override func viewDidLoad() {
    super.viewDidLoad()
    view.backgroundColor = .systemBackground
    let title = UILabel()
    title.text = "Zerohop"
    title.font = .preferredFont(forTextStyle: .headline)
    title.adjustsFontForContentSizeCategory = true
    message.text = Self.words("preparing")
    message.font = .preferredFont(forTextStyle: .body)
    message.adjustsFontForContentSizeCategory = true
    message.numberOfLines = 0
    message.textAlignment = .center
    button.setTitle(Self.words("cancel"), for: .normal)
    button.titleLabel?.font = .preferredFont(forTextStyle: .headline)
    button.titleLabel?.adjustsFontForContentSizeCategory = true
    button.addTarget(self, action: #selector(tapped), for: .touchUpInside)
    spinner.startAnimating()
    let stack = UIStackView(arrangedSubviews: [title, spinner, message, button])
    stack.axis = .vertical
    stack.alignment = .center
    stack.spacing = 20
    stack.translatesAutoresizingMaskIntoConstraints = false
    view.addSubview(stack)
    let margins = view.layoutMarginsGuide
    NSLayoutConstraint.activate([
      stack.centerYAnchor.constraint(equalTo: margins.centerYAnchor),
      stack.leadingAnchor.constraint(equalTo: margins.leadingAnchor, constant: 8),
      stack.trailingAnchor.constraint(equalTo: margins.trailingAnchor, constant: -8),
      button.heightAnchor.constraint(greaterThanOrEqualToConstant: 44),
    ])
    let providers = (extensionContext?.inputItems as? [NSExtensionItem] ?? [])
      .flatMap { $0.attachments ?? [] }
    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
      guard let self else { return }
      let outcome = self.copyAll(Array(providers.prefix(SharedInbox.maxFiles)))
      DispatchQueue.main.async { self.finish(outcome) }
    }
  }

  @objc private func tapped() {
    if done {
      extensionContext?.completeRequest(returningItems: nil)
      return
    }
    lock.lock()
    stopped = true
    loading?.cancel()
    let half = share
    lock.unlock()
    if let half { try? FileManager.default.removeItem(at: half) }
    extensionContext?.cancelRequest(
      withError: NSError(domain: NSCocoaErrorDomain, code: NSUserCancelledError))
  }

  private var isStopped: Bool {
    lock.lock()
    defer { lock.unlock() }
    return stopped
  }

  /// One file at a time, in the order the owner chose them, so a large
  /// video does not have to share the free space with the rest.
  private func copyAll(_ providers: [NSItemProvider]) -> Outcome {
    guard let root = SharedInbox.root else { return .unavailable }
    SharedInbox.sweep()
    let folder = root.appendingPathComponent(UUID().uuidString, isDirectory: true)
    do {
      try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
    } catch {
      return .failed
    }
    lock.lock()
    share = folder
    lock.unlock()
    var copied = 0
    var failed = 0
    for (index, provider) in providers.enumerated() {
      if isStopped { return .failed }
      switch copyOne(provider, into: folder.appendingPathComponent(String(format: "%03d", index))) {
      case .copied: copied += 1
      case .skipped: failed += 1
      case .noRoom:
        try? FileManager.default.removeItem(at: folder)
        return .noRoom
      }
    }
    if isStopped { return .failed }
    guard copied > 0 else {
      try? FileManager.default.removeItem(at: folder)
      return .nothing
    }
    do {
      try Data().write(to: folder.appendingPathComponent(SharedInbox.readyMark))
    } catch {
      try? FileManager.default.removeItem(at: folder)
      return .failed
    }
    CFNotificationCenterPostNotification(
      CFNotificationCenterGetDarwinNotifyCenter(),
      CFNotificationName(SharedInbox.readyNotice as CFString), nil, nil, true)
    return .ready(failed: failed)
  }

  private enum Copy { case copied, skipped, noRoom }

  private func copyOne(_ provider: NSItemProvider, into own: URL) -> Copy {
    guard let type = Self.fileType(provider) else { return .skipped }
    let name = provider.suggestedName
    let wait = DispatchSemaphore(value: 0)
    var outcome = Copy.skipped
    let progress = provider.loadFileRepresentation(forTypeIdentifier: type.identifier) {
      lent, _ in
      // The file is lent only until this returns.
      defer { wait.signal() }
      guard let lent, !self.isStopped else { return }
      outcome = Self.keep(lent, name: name, type: type, in: own)
    }
    lock.lock()
    loading = progress
    lock.unlock()
    wait.wait()
    lock.lock()
    loading = nil
    lock.unlock()
    return outcome
  }

  /// The file itself, offered first among its forms. A Live Photo's motion
  /// is a separate part and stays behind, as in the app's own photo picker.
  /// Folders and bare links are not files and are skipped.
  private static func fileType(_ provider: NSItemProvider) -> UTType? {
    let types = provider.registeredTypeIdentifiers.compactMap { UTType($0) }
    let media = types.first {
      ($0.conforms(to: .image) || $0.conforms(to: .movie)) && !$0.conforms(to: .livePhoto)
    }
    return media ?? types.first { $0.conforms(to: .data) && !$0.conforms(to: .url) }
  }

  private static func keep(_ lent: URL, name: String?, type: UTType, in own: URL) -> Copy {
    let manager = FileManager.default
    let size = (try? lent.resourceValues(forKeys: [.fileSizeKey]))?.fileSize ?? 0
    let free =
      (try? own.deletingLastPathComponent().resourceValues(
        forKeys: [.volumeAvailableCapacityForImportantUsageKey]))?
      .volumeAvailableCapacityForImportantUsage ?? 0
    if free - Int64(size) < minFreeBytes { return .noRoom }
    let ext = lent.pathExtension.isEmpty ? type.preferredFilenameExtension ?? "" : lent.pathExtension
    var base = SharedInbox.safeName(name, fallback: "")
    if base.isEmpty { base = SharedInbox.safeName(lent.deletingPathExtension().lastPathComponent, fallback: "shared") }
    let file =
      ext.isEmpty || (base as NSString).pathExtension.lowercased() == ext.lowercased()
      ? base : SharedInbox.safeName("\(base).\(ext)", fallback: "shared")
    do {
      try manager.createDirectory(at: own, withIntermediateDirectories: true)
      try manager.copyItem(at: lent, to: own.appendingPathComponent(file, isDirectory: false))
      return .copied
    } catch {
      try? manager.removeItem(at: own)
      return .skipped
    }
  }

  private func finish(_ outcome: Outcome) {
    guard !isStopped else { return }
    done = true
    spinner.stopAnimating()
    spinner.isHidden = true
    switch outcome {
    case .ready(let failed):
      message.text =
        failed == 0
        ? Self.words("ready")
        : Self.words("ready") + "\n\n" + String(format: Self.words("partly"), failed)
    case .nothing: message.text = Self.words("nothing")
    case .noRoom: message.text = Self.words("no_room")
    case .failed: message.text = Self.words("failed")
    case .unavailable: message.text = Self.words("unavailable")
    }
    button.setTitle(Self.words("done"), for: .normal)
  }

  private static func words(_ key: String) -> String {
    NSLocalizedString(key, comment: "")
  }
}
