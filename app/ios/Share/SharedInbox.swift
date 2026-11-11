import Foundation
import UniformTypeIdentifiers

/// Files shared to Zerohop from another app wait here, in the space the app
/// and its Share extension both reach, until Zerohop is opened. Nothing here
/// is sent on its own: the app shows the files, and only the owner's tap on
/// Send moves them. Built into both the app and the extension.
///
/// Each share is a folder named by a random UUID, holding one folder per
/// file, so two files with the same name can travel together. The extension
/// writes the ready mark last; the app takes only folders that carry it.
enum SharedInbox {
  static let group = "group.dev.zerohop.app"

  /// Posted by the extension once a share is ready.
  static let readyNotice = "dev.zerohop.app.shared"

  static let maxFiles = 200
  static let readyMark = ".ready"
  private static let maxNameBytes = 200
  private static let staleAfter: TimeInterval = 24 * 60 * 60

  /// Nil when iOS does not give this build the shared space, for example
  /// when it was signed without the app group.
  static var root: URL? {
    FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group)?
      .appendingPathComponent("Inbox", isDirectory: true)
  }

  /// A name that stays inside its folder and fits on the file system. The
  /// name comes from the other app and is treated as hostile.
  static func safeName(_ raw: String?, fallback: String) -> String {
    var name = String((raw ?? "").unicodeScalars.filter {
      !CharacterSet.controlCharacters.contains($0)
    }.map(Character.init))
    if let slash = name.lastIndex(where: { $0 == "/" || $0 == "\\" }) {
      name = String(name[name.index(after: slash)...])
    }
    name = name.trimmingCharacters(in: .whitespacesAndNewlines)
    while name.utf8.count > maxNameBytes { name.removeFirst() }
    return name.isEmpty || name == "." || name == ".." ? fallback : name
  }

  /// Shares never collected within a day, or left half written because the
  /// extension was stopped, only take up room.
  static func sweep() {
    guard let root else { return }
    let oldest = Date().addingTimeInterval(-staleAfter)
    for share in shares(in: root) {
      let made = (try? share.resourceValues(forKeys: [.creationDateKey]))?.creationDate
      if let made, made < oldest { try? FileManager.default.removeItem(at: share) }
    }
  }

  /// Moves every ready share into [folder] and returns the paths of its
  /// files, oldest share first. Runs in the app, never in the extension.
  static func collect(into folder: URL) -> [String] {
    guard let root else { return [] }
    sweep()
    let manager = FileManager.default
    let ready = shares(in: root).filter {
      manager.fileExists(atPath: $0.appendingPathComponent(readyMark).path)
    }.sorted { creation($0) < creation($1) }
    var paths = [String]()
    for share in ready {
      let items = (try? manager.contentsOfDirectory(
        at: share, includingPropertiesForKeys: nil)) ?? []
      for item in items.sorted(by: { $0.lastPathComponent < $1.lastPathComponent })
      where item.lastPathComponent != readyMark {
        if let path = take(item, into: folder) { paths.append(path) }
      }
      try? manager.removeItem(at: share)
    }
    return paths
  }

  /// One file's folder, moved whole. Anything but a single regular file is
  /// left behind and removed with its share.
  private static func take(_ item: URL, into folder: URL) -> String? {
    let manager = FileManager.default
    let files = (try? manager.contentsOfDirectory(
      at: item, includingPropertiesForKeys: [.isRegularFileKey, .isSymbolicLinkKey])) ?? []
    guard files.count == 1, let file = files.first,
      let kind = try? file.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey]),
      kind.isRegularFile == true, kind.isSymbolicLink != true
    else { return nil }
    let own = folder.appendingPathComponent(UUID().uuidString, isDirectory: true)
    do {
      try manager.createDirectory(at: folder, withIntermediateDirectories: true)
      try manager.moveItem(at: item, to: own)
      return own.appendingPathComponent(file.lastPathComponent).path
    } catch {
      try? manager.removeItem(at: own)
      return nil
    }
  }

  private static func shares(in root: URL) -> [URL] {
    let found = (try? FileManager.default.contentsOfDirectory(
      at: root, includingPropertiesForKeys: [.isDirectoryKey, .creationDateKey])) ?? []
    return found.filter { UUID(uuidString: $0.lastPathComponent) != nil }
  }

  private static func creation(_ url: URL) -> Date {
    (try? url.resourceValues(forKeys: [.creationDateKey]))?.creationDate ?? .distantPast
  }
}
