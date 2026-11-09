import 'package:flutter/material.dart';

/// The colors of one appearance and palette.
@immutable
class ZColors extends ThemeExtension<ZColors> {
  const ZColors({
    required this.bg,
    required this.surface,
    required this.surface2,
    required this.ink,
    required this.muted,
    required this.line,
    required this.accent,
    required this.on,
    required this.soft,
    required this.a,
    required this.b,
    required this.ia,
    required this.ib,
    required this.danger,
    required this.dangerSoft,
  });

  final Color bg, surface, surface2, ink, muted, line, accent, on, soft;
  final Color a, b, ia, ib, danger, dangerSoft;

  LinearGradient get gradient => LinearGradient(
    begin: Alignment.topLeft,
    end: Alignment.bottomRight,
    colors: [a, b],
  );

  @override
  ZColors copyWith() => this;

  @override
  ZColors lerp(ThemeExtension<ZColors>? other, double t) =>
      t < 0.5 || other is! ZColors ? this : other;
}

class Palette {
  const Palette({
    required this.light,
    required this.dark,
    required this.a,
    required this.b,
    required this.ia,
    required this.ib,
  });

  final List<Color> light, dark;
  final Color a, b, ia, ib;
}

const palettes = <String, Palette>{
  'cobalt': Palette(
    light: [Color(0xFF2F5BFF), Color(0xFFFFFFFF), Color(0xFFE6ECFF)],
    dark: [Color(0xFF86A5FF), Color(0xFF0B1230), Color(0xFF1E2A52)],
    a: Color(0xFF2A5FF0),
    b: Color(0xFF1A2FC9),
    ia: Color(0xFF4A8DFF),
    ib: Color(0xFF1F37E6),
  ),
  'sunset': Palette(
    light: [Color(0xFFCC2542), Color(0xFFFFFFFF), Color(0xFFFDE6E8)],
    dark: [Color(0xFFFF8FA0), Color(0xFF2B060D), Color(0xFF45202A)],
    a: Color(0xFFD93A2B),
    b: Color(0xFFB8124F),
    ia: Color(0xFFFF9A3D),
    ib: Color(0xFFF0245C),
  ),
  'mint': Palette(
    light: [Color(0xFF007F70), Color(0xFFFFFFFF), Color(0xFFD8F3EC)],
    dark: [Color(0xFF4FE0BC), Color(0xFF03261F), Color(0xFF123C35)],
    a: Color(0xFF0B7F73),
    b: Color(0xFF0A5A99),
    ia: Color(0xFF22D3A6),
    ib: Color(0xFF0A6FB5),
  ),
};

ZColors colorsFor(String paletteId, Brightness brightness) {
  final palette = palettes[paletteId] ?? palettes['cobalt']!;
  final dark = brightness == Brightness.dark;
  final tones = dark ? palette.dark : palette.light;
  return ZColors(
    bg: dark ? const Color(0xFF0E1118) : const Color(0xFFF3F4F8),
    surface: dark ? const Color(0xFF181C26) : const Color(0xFFFFFFFF),
    surface2: dark ? const Color(0xFF242936) : const Color(0xFFEBEDF3),
    ink: dark ? const Color(0xFFF2F4F8) : const Color(0xFF11131A),
    muted: dark ? const Color(0xFFA4ACBC) : const Color(0xFF5C6373),
    line: dark ? const Color(0xFF343B4D) : const Color(0xFFD5D9E3),
    danger: dark ? const Color(0xFFFF9A90) : const Color(0xFFC8241B),
    dangerSoft: dark ? const Color(0xFF3A1D1B) : const Color(0xFFFDECEA),
    accent: tones[0],
    on: tones[1],
    soft: tones[2],
    a: palette.a,
    b: palette.b,
    ia: palette.ia,
    ib: palette.ib,
  );
}

final _themes = <(String, Brightness), ThemeData>{};

/// Themes are costly to build and the app rebuilds often during a transfer.
ThemeData themeFor(String paletteId, Brightness brightness) {
  return _themes.putIfAbsent((
    paletteId,
    brightness,
  ), () => _buildTheme(paletteId, brightness));
}

ThemeData _buildTheme(String paletteId, Brightness brightness) {
  final colors = colorsFor(paletteId, brightness);
  return ThemeData(
    brightness: brightness,
    fontFamily: 'Onest',
    scaffoldBackgroundColor: colors.bg,
    colorScheme: ColorScheme.fromSeed(
      seedColor: colors.accent,
      brightness: brightness,
      primary: colors.accent,
      onPrimary: colors.on,
      surface: colors.surface,
      onSurface: colors.ink,
      error: colors.danger,
    ),
    extensions: [colors],
    splashFactory: NoSplash.splashFactory,
    highlightColor: Colors.transparent,
    hoverColor: colors.ink.withValues(alpha: 0.04),
    focusColor: colors.accent.withValues(alpha: 0.18),
    textSelectionTheme: TextSelectionThemeData(
      cursorColor: colors.accent,
      selectionColor: colors.accent.withValues(alpha: 0.28),
    ),
  );
}

extension ZTheme on BuildContext {
  ZColors get z => Theme.of(this).extension<ZColors>()!;
}

/// Text in Onest at a precise weight. The font is variable, so the weight has
/// to be set on its axis as well.
TextStyle zText(
  double size, {
  int weight = 400,
  Color? color,
  double? height,
  double letterSpacing = 0,
  bool tabular = false,
}) {
  return TextStyle(
    fontFamily: 'Onest',
    fontSize: size,
    fontWeight: FontWeight.values[(weight ~/ 100) - 1],
    fontVariations: [FontVariation('wght', weight.toDouble())],
    color: color,
    height: height,
    letterSpacing: letterSpacing * size,
    fontFeatures: tabular ? const [FontFeature.tabularFigures()] : null,
  );
}
