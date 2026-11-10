// Pictures on the Windows clipboard come as the system's own bitmaps, which
// other systems do not read, and many Windows programs paste nothing else.
// These turn them into PNG and back. It is done here rather than in the native
// code because the bytes come from other programs and other devices.

import 'dart:typed_data';
import 'dart:ui' as ui;

/// The most a copied picture may take, as on the Mac.
const maxClipBytes = 24 * 1024 * 1024;

/// Larger pictures are not unpacked, so that one copy cannot use up memory.
const maxClipPixels = 40 * 1000 * 1000;

/// A picture with more pixels than [maxClipPixels].
class PictureTooLarge implements Exception {
  const PictureTooLarge();
}

const _fileHeader = 14;
const _infoHeader = 40;
const _plain = 0;
const _bitFields = 3;
const _alphaBitFields = 6;

/// A PNG made from a clipboard bitmap, or null when the bytes are not a
/// picture. Throws [PictureTooLarge].
Future<Uint8List?> pngFromDib(Uint8List dib) async {
  final bmp = bmpFromDib(dib);
  if (bmp == null) return null;
  return pngFromImage(bmp);
}

/// Any picture the system can read, as a fresh PNG. Nothing that followed the
/// image in the clipboard's memory goes along. Throws [PictureTooLarge].
Future<Uint8List?> pngFromImage(Uint8List encoded) async {
  final image = await _decode(encoded);
  if (image == null) return null;
  try {
    final png = await image.toByteData(format: ui.ImageByteFormat.png);
    if (png == null) return null;
    return png.buffer.asUint8List(png.offsetInBytes, png.lengthInBytes);
  } finally {
    image.dispose();
  }
}

/// A bitmap that Windows programs can paste, from any picture the system can
/// read. Throws [PictureTooLarge].
Future<Uint8List?> dibFromImage(Uint8List encoded) async {
  final image = await _decode(encoded);
  if (image == null) return null;
  try {
    final rgba = await image.toByteData(
      format: ui.ImageByteFormat.rawStraightRgba,
    );
    if (rgba == null) return null;
    return dibFromRgba(
      rgba.buffer.asUint8List(rgba.offsetInBytes, rgba.lengthInBytes),
      image.width,
      image.height,
    );
  } finally {
    image.dispose();
  }
}

/// A 32-bit bitmap with its rows from the bottom up, the layout every Windows
/// program reads.
Uint8List dibFromRgba(Uint8List rgba, int width, int height) {
  final stride = width * 4;
  final out = Uint8List(_infoHeader + stride * height);
  ByteData.sublistView(out)
    ..setUint32(0, _infoHeader, Endian.little)
    ..setInt32(4, width, Endian.little)
    ..setInt32(8, height, Endian.little)
    ..setUint16(12, 1, Endian.little)
    ..setUint16(14, 32, Endian.little)
    ..setUint32(16, _plain, Endian.little)
    ..setUint32(20, stride * height, Endian.little);
  for (var y = 0; y < height; y++) {
    final from = y * stride;
    final to = _infoHeader + (height - 1 - y) * stride;
    for (var x = 0; x < stride; x += 4) {
      out[to + x] = rgba[from + x + 2];
      out[to + x + 1] = rgba[from + x + 1];
      out[to + x + 2] = rgba[from + x];
      out[to + x + 3] = rgba[from + x + 3];
    }
  }
  return out;
}

/// Puts a file header in front of a clipboard bitmap so that the system's
/// decoder can read it. Null when the header does not add up.
Uint8List? bmpFromDib(Uint8List dib) {
  if (dib.length < _infoHeader) return null;
  final view = ByteData.sublistView(dib);
  final headerSize = view.getUint32(0, Endian.little);
  final width = view.getInt32(4, Endian.little);
  final height = view.getInt32(8, Endian.little);
  final planes = view.getUint16(12, Endian.little);
  final bits = view.getUint16(14, Endian.little);
  final compression = view.getUint32(16, Endian.little);
  var colors = view.getUint32(32, Endian.little);
  if (!const {40, 52, 56, 108, 124}.contains(headerSize) ||
      headerSize > dib.length ||
      planes != 1 ||
      !const {1, 4, 8, 16, 24, 32}.contains(bits) ||
      !const {0, 1, 2, 3, 6}.contains(compression) ||
      width <= 0 ||
      height == 0 ||
      colors > 256) {
    return null;
  }
  if (bits <= 8 && colors == 0) colors = 1 << bits;
  // The smallest header keeps its colour masks after it; larger ones inside.
  final masks = headerSize != _infoHeader
      ? 0
      : compression == _bitFields
      ? 12
      : compression == _alphaBitFields
      ? 16
      : 0;
  final offset = headerSize + masks + colors * 4;
  if (offset > dib.length) return null;
  if (compression == _plain || compression == _bitFields) {
    final stride = ((width * bits + 31) ~/ 32) * 4;
    if (offset + stride * height.abs() > dib.length) return null;
  }
  final out = Uint8List(_fileHeader + dib.length);
  out[0] = 0x42;
  out[1] = 0x4D;
  ByteData.sublistView(out, 0, _fileHeader)
    ..setUint32(2, out.length, Endian.little)
    ..setUint32(10, _fileHeader + offset, Endian.little);
  out.setRange(_fileHeader, out.length, dib);
  return out;
}

Future<ui.Image?> _decode(Uint8List encoded) async {
  ui.ImmutableBuffer? buffer;
  ui.ImageDescriptor? descriptor;
  ui.Codec? codec;
  try {
    buffer = await ui.ImmutableBuffer.fromUint8List(encoded);
    descriptor = await ui.ImageDescriptor.encoded(buffer);
    final width = descriptor.width;
    final height = descriptor.height;
    if (width <= 0 || height <= 0) return null;
    if (width * height > maxClipPixels) throw const PictureTooLarge();
    codec = await descriptor.instantiateCodec();
    final frame = await codec.getNextFrame();
    return frame.image;
  } on PictureTooLarge {
    rethrow;
  } on Exception {
    return null;
  } finally {
    codec?.dispose();
    descriptor?.dispose();
    buffer?.dispose();
  }
}
