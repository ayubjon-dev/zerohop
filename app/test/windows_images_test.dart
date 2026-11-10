import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter_test/flutter_test.dart';
import 'package:zerohop/src/windows_images.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  group('dibFromRgba', () {
    // Top row: red, green. Bottom row: blue, half-transparent white.
    final rgba = Uint8List.fromList([
      255, 0, 0, 255, 0, 255, 0, 255, //
      0, 0, 255, 255, 255, 255, 255, 128,
    ]);

    test('writes the header Windows expects', () {
      final dib = dibFromRgba(rgba, 2, 2);
      final view = ByteData.sublistView(dib);
      expect(
        [
          view.getUint32(0, Endian.little),
          view.getInt32(4, Endian.little),
          view.getInt32(8, Endian.little),
          view.getUint16(12, Endian.little),
          view.getUint16(14, Endian.little),
          view.getUint32(16, Endian.little),
          view.getUint32(20, Endian.little),
          dib.length,
        ],
        [40, 2, 2, 1, 32, 0, 16, 56],
      );
    });

    test('stores the rows bottom up, blue first', () {
      final dib = dibFromRgba(rgba, 2, 2);
      expect(dib.sublist(40), [
        255, 0, 0, 255, 255, 255, 255, 128, //
        0, 0, 255, 255, 0, 255, 0, 255,
      ]);
    });
  });

  group('bmpFromDib', () {
    Uint8List header({
      int size = 40,
      int width = 1,
      int height = 1,
      int planes = 1,
      int bits = 32,
      int compression = 0,
      int colors = 0,
      int extra = 4,
    }) {
      final dib = Uint8List(size + extra);
      ByteData.sublistView(dib)
        ..setUint32(0, size, Endian.little)
        ..setInt32(4, width, Endian.little)
        ..setInt32(8, height, Endian.little)
        ..setUint16(12, planes, Endian.little)
        ..setUint16(14, bits, Endian.little)
        ..setUint32(16, compression, Endian.little)
        ..setUint32(32, colors, Endian.little);
      return dib;
    }

    int pixelOffset(Uint8List bmp) =>
        ByteData.sublistView(bmp).getUint32(10, Endian.little);

    test('points past the header for a plain bitmap', () {
      final bmp = bmpFromDib(header())!;
      expect(
        [bmp[0], bmp[1], bmp.length, pixelOffset(bmp)],
        [0x42, 0x4D, 58, 54],
      );
    });

    test('skips the colour masks that follow the smallest header', () {
      final bmp = bmpFromDib(header(compression: 3, extra: 16))!;
      expect(pixelOffset(bmp), 14 + 40 + 12);
    });

    test('finds the masks inside a larger header', () {
      final bmp = bmpFromDib(header(size: 124, compression: 3))!;
      expect(pixelOffset(bmp), 14 + 124);
    });

    test('skips the colour table of a paletted bitmap', () {
      final bmp = bmpFromDib(header(bits: 8, extra: 256 * 4 + 4))!;
      expect(pixelOffset(bmp), 14 + 40 + 256 * 4);
    });

    test('accepts rows stored top down', () {
      expect(bmpFromDib(header(height: -1)), isNotNull);
    });

    test('refuses bytes too short for a header', () {
      expect(bmpFromDib(Uint8List(39)), isNull);
    });

    test('refuses an unknown header size', () {
      expect(bmpFromDib(header(size: 12, extra: 40)), isNull);
    });

    test('refuses a header larger than the bytes', () {
      expect(bmpFromDib(header(size: 124, extra: 0).sublist(0, 100)), isNull);
    });

    test('refuses a picture with no width', () {
      expect(bmpFromDib(header(width: 0)), isNull);
    });

    test('refuses a picture with no height', () {
      expect(bmpFromDib(header(height: 0)), isNull);
    });

    test('refuses a bitmap with more than one plane', () {
      expect(bmpFromDib(header(planes: 2)), isNull);
    });

    test('refuses an unknown number of bits', () {
      expect(bmpFromDib(header(bits: 7)), isNull);
    });

    test('refuses JPEG and PNG inside a bitmap', () {
      expect(bmpFromDib(header(compression: 5)), isNull);
    });

    test('refuses a colour table larger than allowed', () {
      expect(bmpFromDib(header(colors: 257, extra: 2000)), isNull);
    });

    test('refuses a colour table that runs past the end', () {
      expect(bmpFromDib(header(bits: 8, extra: 100)), isNull);
    });

    test('refuses pixels that run past the end', () {
      expect(bmpFromDib(header(width: 2, height: 2, extra: 8)), isNull);
    });
  });

  group('converting pictures', () {
    // Two rows of three opaque pixels.
    final rgba = Uint8List.fromList([
      255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, //
      10, 20, 30, 255, 200, 100, 50, 255, 255, 255, 255, 255,
    ]);

    test('a picture survives the trip through a Windows bitmap', () async {
      final png = await _png(rgba, 3, 2);
      final dib = await dibFromImage(png);
      final back = await pngFromDib(dib!);
      expect(await _pixels(back!), rgba);
    });

    test('a copied PNG comes out as the same picture', () async {
      final png = await _png(rgba, 3, 2);
      final withTail = Uint8List.fromList([...png, 1, 2, 3, 4, 5, 6, 7]);
      expect(await _pixels((await pngFromImage(withTail))!), rgba);
    });

    test('bytes that are not a picture give nothing', () async {
      expect(await pngFromImage(Uint8List.fromList([1, 2, 3, 4])), isNull);
    });

    test('a bitmap that does not add up gives nothing', () async {
      expect(await pngFromDib(Uint8List(64)), isNull);
    });

    test('a picture with too many pixels is not unpacked', () async {
      await expectLater(
        pngFromImage(_pngHeaderOnly(10000, 5000)),
        throwsA(isA<PictureTooLarge>()),
      );
    });
  });
}

Future<Uint8List> _png(Uint8List rgba, int width, int height) async {
  final buffer = await ui.ImmutableBuffer.fromUint8List(rgba);
  final descriptor = ui.ImageDescriptor.raw(
    buffer,
    width: width,
    height: height,
    pixelFormat: ui.PixelFormat.rgba8888,
  );
  final codec = await descriptor.instantiateCodec();
  final image = (await codec.getNextFrame()).image;
  final png = await image.toByteData(format: ui.ImageByteFormat.png);
  image.dispose();
  codec.dispose();
  descriptor.dispose();
  buffer.dispose();
  return png!.buffer.asUint8List();
}

Future<Uint8List> _pixels(Uint8List encoded) async {
  final codec = await ui.instantiateImageCodec(encoded);
  final image = (await codec.getNextFrame()).image;
  final data = await image.toByteData(
    format: ui.ImageByteFormat.rawStraightRgba,
  );
  image.dispose();
  codec.dispose();
  return data!.buffer.asUint8List();
}

/// A PNG that claims the given size, with only a token of image data.
Uint8List _pngHeaderOnly(int width, int height) {
  final ihdr = ByteData(13)
    ..setUint32(0, width)
    ..setUint32(4, height)
    ..setUint8(8, 8)
    ..setUint8(9, 6);
  return Uint8List.fromList([
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, //
    ..._chunk('IHDR', ihdr.buffer.asUint8List()),
    ..._chunk('IDAT', [0x78, 0x9C, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01]),
    ..._chunk('IEND', const []),
  ]);
}

List<int> _chunk(String type, List<int> data) {
  final body = [...type.codeUnits, ...data];
  final length = ByteData(4)..setUint32(0, data.length);
  final crc = ByteData(4)..setUint32(0, _crc32(body));
  return [...length.buffer.asUint8List(), ...body, ...crc.buffer.asUint8List()];
}

int _crc32(List<int> bytes) {
  var crc = 0xFFFFFFFF;
  for (final byte in bytes) {
    crc ^= byte;
    for (var bit = 0; bit < 8; bit++) {
      crc = (crc & 1) != 0 ? (crc >> 1) ^ 0xEDB88320 : crc >> 1;
    }
  }
  return crc ^ 0xFFFFFFFF;
}
