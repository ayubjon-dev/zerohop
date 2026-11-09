import 'package:flutter/material.dart';
import 'package:path_parsing/path_parsing.dart';

/// Line icons drawn from their outlines, on a 24-unit grid.
abstract final class ZIcons {
  static const laptop =
      'M20 16V7a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v9m16 0H4m16 0 1.28 2.55a1 1 0 0 1-.9 1.45H3.62a1 1 0 0 1-.9-1.45L4 16';
  static const desktop =
      'M4 3h16a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2zM8 21h8M12 17v4';
  static const phone =
      'M7.5 2h9A2.5 2.5 0 0 1 19 4.5v15a2.5 2.5 0 0 1-2.5 2.5h-9A2.5 2.5 0 0 1 5 19.5v-15A2.5 2.5 0 0 1 7.5 2zM12 18h.01';
  static const plus = 'M12 5v14M5 12h14';
  static const clip =
      'M9 2h6a1 1 0 0 1 1 1v2a1 1 0 0 1-1 1H9a1 1 0 0 1-1-1V3a1 1 0 0 1 1-1zM16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2';
  static const back = 'M15 18l-6-6 6-6';
  static const x = 'M18 6 6 18M6 6l12 12';
  static const check = 'M20 6 9 17l-5-5';
  static const dots = 'M5 12h.01M12 12h.01M19 12h.01';
  static const grid =
      'M4.5 3h4A1.5 1.5 0 0 1 10 4.5v4A1.5 1.5 0 0 1 8.5 10h-4A1.5 1.5 0 0 1 3 8.5v-4A1.5 1.5 0 0 1 4.5 3zM15.5 3h4A1.5 1.5 0 0 1 21 4.5v4a1.5 1.5 0 0 1-1.5 1.5h-4A1.5 1.5 0 0 1 14 8.5v-4A1.5 1.5 0 0 1 15.5 3zM4.5 14h4a1.5 1.5 0 0 1 1.5 1.5v4A1.5 1.5 0 0 1 8.5 21h-4A1.5 1.5 0 0 1 3 19.5v-4A1.5 1.5 0 0 1 4.5 14zM15.5 14h4a1.5 1.5 0 0 1 1.5 1.5v4a1.5 1.5 0 0 1-1.5 1.5h-4a1.5 1.5 0 0 1-1.5-1.5v-4a1.5 1.5 0 0 1 1.5-1.5z';
  static const clock = 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 7v5l3.5 2';
  static const sliders =
      'M21 6h-8M9 6H3M21 18h-4M13 18H3M15 12H3M21 12h-2M11 4a2 2 0 1 0 0 4 2 2 0 0 0 0-4zM17 10a2 2 0 1 0 0 4 2 2 0 0 0 0-4zM15 16a2 2 0 1 0 0 4 2 2 0 0 0 0-4z';
  static const upload =
      'M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M17 8l-5-5-5 5M12 3v12';
  static const image =
      'M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2zM9 7a2 2 0 1 0 0 4 2 2 0 0 0 0-4zM21 15l-3.09-3.09a2 2 0 0 0-2.82 0L6 21';
  static const video =
      'M4 6h10a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2zM16 13l5.22 3.48a.5.5 0 0 0 .78-.42V7.87a.5.5 0 0 0-.75-.43L16 10.5';
  static const doc =
      'M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7ZM14 2v4a2 2 0 0 0 2 2h4M16 13H8M16 17H8';
  static const folder =
      'M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z';
  static const up = 'M12 19V5M5 12l7-7 7 7';
  static const down = 'M12 5v14M19 12l-7 7-7-7';
  static const qr =
      'M4 3h5a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1zM15 3h5a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1h-5a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1zM4 14h5a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1v-5a1 1 0 0 1 1-1zM14 14h3v3h-3zM21 21h-4M21 14v3';
  static const hash = 'M4 9h16M4 15h16M10 3 8 21M16 3l-2 18';
  static const warn =
      'M21.73 18l-8-14a2 2 0 0 0-3.46 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3zM12 9v4M12 17h.01';

  static String forPlatform(String platform) {
    switch (platform) {
      case 'ios':
      case 'android':
        return phone;
      case 'windows':
      case 'linux':
        return desktop;
      default:
        return laptop;
    }
  }

  static String forFile(String name, {bool isDirectory = false}) {
    if (isDirectory) return folder;
    final dot = name.lastIndexOf('.');
    final extension = dot < 0 ? '' : name.substring(dot + 1).toLowerCase();
    const images = {'jpg', 'jpeg', 'png', 'gif', 'heic', 'webp', 'bmp', 'tiff'};
    const videos = {'mp4', 'mov', 'mkv', 'avi', 'webm', 'hevc'};
    if (images.contains(extension)) return image;
    if (videos.contains(extension)) return video;
    return doc;
  }
}

class _PathBuilder extends PathProxy {
  final path = Path();

  @override
  void close() => path.close();

  @override
  void cubicTo(
    double x1,
    double y1,
    double x2,
    double y2,
    double x3,
    double y3,
  ) => path.cubicTo(x1, y1, x2, y2, x3, y3);

  @override
  void lineTo(double x, double y) => path.lineTo(x, y);

  @override
  void moveTo(double x, double y) => path.moveTo(x, y);
}

final _cache = <String, Path>{};

Path _parse(String data) {
  return _cache.putIfAbsent(data, () {
    final builder = _PathBuilder();
    writeSvgPathDataToPath(data, builder);
    return builder.path;
  });
}

class ZIcon extends StatelessWidget {
  const ZIcon(
    this.data, {
    super.key,
    this.size = 22,
    this.color,
    this.stroke = 1.9,
  });

  final String data;
  final double size;
  final Color? color;
  final double stroke;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: size,
      height: size,
      child: CustomPaint(
        painter: _IconPainter(
          _parse(data),
          color ?? DefaultTextStyle.of(context).style.color ?? Colors.black,
          stroke,
        ),
      ),
    );
  }
}

class _IconPainter extends CustomPainter {
  _IconPainter(this.path, this.color, this.stroke);

  final Path path;
  final Color color;
  final double stroke;

  @override
  void paint(Canvas canvas, Size size) {
    final scale = size.width / 24;
    canvas.scale(scale);
    canvas.drawPath(
      path,
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = stroke
        ..strokeCap = StrokeCap.round
        ..strokeJoin = StrokeJoin.round
        ..color = color,
    );
  }

  @override
  bool shouldRepaint(_IconPainter old) =>
      old.path != path || old.color != color || old.stroke != stroke;
}

/// The folded Z of the app icon, in white.
class ZGlyph extends StatelessWidget {
  const ZGlyph({super.key, required this.size});

  final double size;

  @override
  Widget build(BuildContext context) {
    return CustomPaint(size: Size.square(size), painter: _GlyphPainter());
  }
}

class _GlyphPainter extends CustomPainter {
  @override
  void paint(Canvas canvas, Size size) {
    canvas.scale(size.width / 1024);
    final paint = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = 124
      ..strokeCap = StrokeCap.round
      ..color = Colors.white;
    canvas.drawLine(
      const Offset(716, 330),
      const Offset(308, 694),
      paint..color = Colors.white.withValues(alpha: 0.72),
    );
    paint.color = Colors.white;
    canvas.drawLine(const Offset(308, 330), const Offset(716, 330), paint);
    canvas.drawLine(const Offset(308, 694), const Offset(716, 694), paint);
  }

  @override
  bool shouldRepaint(_GlyphPainter old) => false;
}
