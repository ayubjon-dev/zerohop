import FileProvider
import Foundation

/// What Zerohop and its File Provider extension say to each other. Built into
/// both.
///
/// Files copied on another device show in Finder as placeholders that hold
/// nothing yet. The extension lists them; when a paste reads one, the
/// extension asks the app, which fetches the files from the other device
/// right then and hands each one over already open.
enum FilesLink {
  static let service = NSFileProviderServiceName("dev.zerohop.app.files.link")
  static let domain = NSFileProviderDomainIdentifier("clipboard")
  /// Most placeholders shown at once. A bigger copy is not offered here.
  static let maxItems = 20_000

  /// The placeholder for one item of one copy.
  static func itemID(clip: String, index: Int) -> NSFileProviderItemIdentifier {
    NSFileProviderItemIdentifier("\(clip).\(index)")
  }
}

/// What the app asks of the extension.
@objc protocol FilesShelf {
  /// Shows these files in place of whatever was shown. An empty list
  /// clears. Answers whether the list was taken.
  func show(_ list: Data, reply: @escaping (Bool) -> Void)
}

/// What the extension asks of the app.
@objc protocol FilesSource {
  /// One listed file, open for reading, once the paste has fetched it. On
  /// failure, the core's name for what went wrong.
  func open(_ clip: String, index: Int, reply: @escaping (FileHandle?, String?) -> Void)
}

/// The files of one copy, by the names the paste will give them.
struct PasteList: Codable, Equatable {
  var clip: String
  var items: [PasteItem]

  static let empty = PasteList(clip: "", items: [])

  /// The items at the top of the copy, which are what the clipboard holds.
  var topLevel: [PasteItem] { items.filter { $0.path.count == 1 } }

  /// Whether the list can be shown as it is: every name stays inside its
  /// folder, and every folder is listed before what is in it. The core has
  /// already made the names safe; this is checked again on each side so that
  /// neither depends on the other being right.
  var isSound: Bool {
    if clip.isEmpty { return items.isEmpty }
    guard clip.count == 32, clip.allSatisfy(\.isHexDigit), !items.isEmpty,
      items.count <= FilesLink.maxItems
    else { return false }
    var seen = Set<String>()
    var folders = Set<String>()
    for (position, item) in items.enumerated() {
      guard item.index == position, !item.path.isEmpty, item.path.allSatisfy(Self.isName),
        item.size >= 0, !(item.dir && item.size != 0)
      else { return false }
      let key = item.path.joined(separator: "/")
      guard seen.insert(key).inserted else { return false }
      if item.path.count > 1 {
        let parent = item.path.dropLast().joined(separator: "/")
        guard folders.contains(parent) else { return false }
      }
      if item.dir { folders.insert(key) }
    }
    return true
  }

  static func isName(_ name: String) -> Bool {
    !name.isEmpty && name != "." && name != ".." && !name.contains("/")
      && !name.contains("\0") && name.utf8.count <= 255
  }
}

/// One file or folder of a copy.
struct PasteItem: Codable, Equatable {
  var index: Int
  var path: [String]
  var dir: Bool
  var size: Int64
  var modified: Int64?
  var created: Int64?
}
