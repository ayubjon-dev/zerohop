import 'dart:math' as math;

import 'package:flutter/widgets.dart';
import 'package:qr/qr.dart' as qr;

/// A QR code, always dark on light: scanners expect that, whatever the theme.
class QrView extends StatelessWidget {
  const QrView({
    super.key,
    required this.data,
    required this.size,
    required this.label,
  });

  final String data;

  /// The largest the code may be, quiet zone included.
  final double size;

  /// What a screen reader says instead of the code.
  final String label;

  /// Modules of empty space the standard asks for around the code.
  static const _quiet = 4;

  static String? _lastData;
  static qr.QrImage? _lastImage;

  static qr.QrImage _image(String data) {
    if (data != _lastData || _lastImage == null) {
      _lastData = data;
      _lastImage = qr.QrImage(
        qr.QrCode(
          payload: qr.QrPayload.fromString(data),
          errorCorrectLevel: qr.QrErrorCorrectLevel.medium,
        ),
      );
    }
    return _lastImage!;
  }

  @override
  Widget build(BuildContext context) {
    final image = _image(data);
    final modules = image.moduleCount + 2 * _quiet;
    final ratio = MediaQuery.devicePixelRatioOf(context);
    // Whole device pixels per module keep the edges sharp for the camera.
    final cell = math.max(1, (size * ratio / modules).floor()) / ratio;
    final side = cell * modules;
    return Semantics(
      label: label,
      image: true,
      excludeSemantics: true,
      child: Container(
        width: side,
        height: side,
        padding: EdgeInsets.all(cell * _quiet),
        decoration: BoxDecoration(
          color: const Color(0xFFFFFFFF),
          borderRadius: BorderRadius.circular(cell * 3),
        ),
        child: CustomPaint(painter: _QrPainter(image, cell)),
      ),
    );
  }
}

class _QrPainter extends CustomPainter {
  _QrPainter(this.image, this.cell);

  final qr.QrImage image;
  final double cell;

  @override
  void paint(Canvas canvas, Size size) {
    final count = image.moduleCount;
    final path = Path();
    for (var row = 0; row < count; row++) {
      for (var col = 0; col < count; col++) {
        if (image.isDark(row, col)) {
          path.addRect(Rect.fromLTWH(col * cell, row * cell, cell, cell));
        }
      }
    }
    canvas.drawPath(
      path,
      Paint()
        ..color = const Color(0xFF000000)
        ..isAntiAlias = false,
    );
  }

  @override
  bool shouldRepaint(_QrPainter old) => old.image != image || old.cell != cell;
}
