// Draws the app icon at every size each platform asks for, so that all of them
// come from one definition. Run from the app folder: swift tool/make_icons.swift

import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

let srgb = CGColorSpace(name: CGColorSpace.sRGB)!

func color(_ r: Int, _ g: Int, _ b: Int, _ a: CGFloat = 1) -> CGColor {
  CGColor(srgbRed: CGFloat(r) / 255, green: CGFloat(g) / 255, blue: CGFloat(b) / 255, alpha: a)
}

/// The artwork on a 1024-unit square with the origin at the top left.
func drawArtwork(_ c: CGContext) {
  let background = CGGradient(
    colorsSpace: srgb, colors: [color(0x4A, 0x8D, 0xFF), color(0x1F, 0x37, 0xE6)] as CFArray,
    locations: [0, 1])!
  c.drawLinearGradient(
    background, start: .zero, end: CGPoint(x: 1024, y: 1024),
    options: [.drawsBeforeStartLocation, .drawsAfterEndLocation])

  c.setLineCap(.round)
  c.setLineWidth(124)

  // The fold, softer at the top so that it reads as going behind the bars.
  c.saveGState()
  c.move(to: CGPoint(x: 716, y: 330))
  c.addLine(to: CGPoint(x: 308, y: 694))
  c.replacePathWithStrokedPath()
  c.clip()
  let fold = CGGradient(
    colorsSpace: srgb, colors: [color(255, 255, 255, 0.55), color(255, 255, 255, 0.92)] as CFArray,
    locations: [0, 1])!
  c.drawLinearGradient(
    fold, start: CGPoint(x: 778, y: 268), end: CGPoint(x: 246, y: 756),
    options: [.drawsBeforeStartLocation, .drawsAfterEndLocation])
  c.restoreGState()

  c.setStrokeColor(color(255, 255, 255))
  c.move(to: CGPoint(x: 308, y: 330))
  c.addLine(to: CGPoint(x: 716, y: 330))
  c.move(to: CGPoint(x: 308, y: 694))
  c.addLine(to: CGPoint(x: 716, y: 694))
  c.strokePath()
}

enum Shape {
  case square  // iOS: the system rounds it, and it must be opaque.
  case rounded  // Android and Windows.
  case macOS  // Sits inside the canvas, as the macOS icon grid expects.
}

func render(_ pixels: Int, _ shape: Shape) -> CGImage {
  let info = shape == .square
    ? CGImageAlphaInfo.noneSkipLast.rawValue : CGImageAlphaInfo.premultipliedLast.rawValue
  let c = CGContext(
    data: nil, width: pixels, height: pixels, bitsPerComponent: 8, bytesPerRow: 0, space: srgb,
    bitmapInfo: info)!
  c.interpolationQuality = .high
  c.scaleBy(x: CGFloat(pixels) / 1024, y: CGFloat(pixels) / 1024)
  c.translateBy(x: 0, y: 1024)
  c.scaleBy(x: 1, y: -1)

  switch shape {
  case .square:
    drawArtwork(c)
  case .rounded:
    let radius: CGFloat = 1024 * 0.224
    c.addPath(
      CGPath(
        roundedRect: CGRect(x: 0, y: 0, width: 1024, height: 1024), cornerWidth: radius,
        cornerHeight: radius, transform: nil))
    c.clip()
    drawArtwork(c)
  case .macOS:
    let inset: CGFloat = 100
    let side: CGFloat = 824
    let radius = side * 0.2237
    let body = CGPath(
      roundedRect: CGRect(x: inset, y: inset, width: side, height: side), cornerWidth: radius,
      cornerHeight: radius, transform: nil)
    c.saveGState()
    c.setShadow(offset: CGSize(width: 0, height: 10), blur: 28, color: color(0, 0, 0, 0.3))
    c.addPath(body)
    c.setFillColor(color(0x1F, 0x37, 0xE6))
    c.fillPath()
    c.restoreGState()
    c.addPath(body)
    c.clip()
    c.translateBy(x: inset, y: inset)
    c.scaleBy(x: side / 1024, y: side / 1024)
    drawArtwork(c)
  }
  return c.makeImage()!
}

func pngData(_ image: CGImage) -> Data {
  let out = NSMutableData()
  let destination = CGImageDestinationCreateWithData(out, UTType.png.identifier as CFString, 1, nil)!
  CGImageDestinationAddImage(destination, image, nil)
  CGImageDestinationFinalize(destination)
  return out as Data
}

func pixelWidth(_ url: URL) -> Int? {
  guard let source = CGImageSourceCreateWithURL(url as CFURL, nil),
    let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any]
  else { return nil }
  return properties[kCGImagePropertyPixelWidth] as? Int
}

/// Redraws every PNG in a folder tree at the size it already has.
func redraw(_ path: String, _ shape: Shape, matching: (String) -> Bool = { _ in true }) {
  let root = URL(fileURLWithPath: path)
  guard let files = FileManager.default.enumerator(at: root, includingPropertiesForKeys: nil) else {
    return
  }
  var count = 0
  for case let url as URL in files where url.pathExtension == "png" && matching(url.lastPathComponent) {
    guard let width = pixelWidth(url) else { continue }
    try! pngData(render(width, shape)).write(to: url)
    count += 1
  }
  print("\(path): \(count) icons")
}

func windowsIcon(_ path: String) {
  let images = [16, 24, 32, 48, 64, 128, 256].map { ($0, pngData(render($0, .rounded))) }
  func u16(_ v: Int) -> Data { withUnsafeBytes(of: UInt16(v).littleEndian) { Data($0) } }
  func u32(_ v: Int) -> Data { withUnsafeBytes(of: UInt32(v).littleEndian) { Data($0) } }
  var out = u16(0) + u16(1) + u16(images.count)
  var offset = 6 + 16 * images.count
  for (size, data) in images {
    out.append(UInt8(size == 256 ? 0 : size))
    out.append(UInt8(size == 256 ? 0 : size))
    out.append(contentsOf: [0, 0])
    out += u16(1) + u16(32) + u32(data.count) + u32(offset)
    offset += data.count
  }
  for (_, data) in images { out += data }
  try! out.write(to: URL(fileURLWithPath: path))
  print("\(path): \(images.count) sizes")
}

redraw("macos/Runner/Assets.xcassets/AppIcon.appiconset", .macOS)
redraw("ios/Runner/Assets.xcassets/AppIcon.appiconset", .square)
redraw("android/app/src/main/res", .rounded) { $0.starts(with: "ic_launcher") }
windowsIcon("windows/runner/resources/app_icon.ico")
try! pngData(render(512, .macOS)).write(to: URL(fileURLWithPath: CommandLine.arguments[1]))
