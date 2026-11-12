import 'package:flutter/material.dart';

import 'tokens.g.dart';

/// This package's name, for its bundled fonts.
const kitPackage = 'agentrix_kit';

/// Headings: Manrope (the concepts' `--font-display`).
TextStyle displayStyle(double size, {FontWeight weight = FontWeight.w800, Color? color, double height = 1.2}) =>
    TextStyle(fontFamily: 'Manrope', package: kitPackage, fontSize: size, fontWeight: weight, color: color, height: height, letterSpacing: -0.2);

/// Text: DM Sans (`--font-text`).
TextStyle textStyle(double size, {FontWeight weight = FontWeight.w400, Color? color, double height = 1.45}) =>
    TextStyle(fontFamily: 'DMSans', package: kitPackage, fontSize: size, fontWeight: weight, color: color, height: height);

/// Codes and fingerprints: JetBrains Mono (`--font-mono`).
TextStyle monoStyle(double size, {Color? color}) =>
    TextStyle(fontFamily: 'JetBrainsMono', package: kitPackage, fontSize: size, fontWeight: FontWeight.w500, color: color, letterSpacing: 0.4);

/// The colours in use (day or Night Time) and, in a customer app, the site's brand.
class AgxPalette extends ThemeExtension<AgxPalette> {
  const AgxPalette({required this.c, this.brand});

  /// Agentrix's colours.
  final AgxColors c;

  /// The site's own brand, in a customer app.
  final SiteBrand? brand;

  @override
  AgxPalette copyWith({AgxColors? c, SiteBrand? brand}) => AgxPalette(c: c ?? this.c, brand: brand ?? this.brand);

  @override
  AgxPalette lerp(AgxPalette? other, double t) => t < 0.5 ? this : (other ?? this);
}

/// The palette from the nearest theme.
AgxColors agx(BuildContext context) => Theme.of(context).extension<AgxPalette>()!.c;

/// The site brand from the nearest theme, if any.
SiteBrand? siteBrand(BuildContext context) => Theme.of(context).extension<AgxPalette>()!.brand;

/// The Agentrix theme: light by day, dark for Night Time (the concepts' one dark theme).
ThemeData agxTheme({bool night = false, SiteBrand? brand}) {
  final c = night ? AgxColors.dark : AgxColors.light;
  final accent = brand?.accent ?? c.indigo;
  final base = ThemeData(
    useMaterial3: true,
    brightness: night ? Brightness.dark : Brightness.light,
    colorScheme: ColorScheme.fromSeed(
      seedColor: accent,
      brightness: night ? Brightness.dark : Brightness.light,
      primary: accent,
      surface: brand?.bg ?? c.paper,
    ),
    scaffoldBackgroundColor: brand?.soft ?? c.surface,
    fontFamily: 'DMSans',
    extensions: [AgxPalette(c: c, brand: brand)],
  );
  final ink = brand?.ink ?? c.ink;
  return base.copyWith(
    textTheme: base.textTheme.apply(fontFamily: 'DMSans', package: kitPackage, bodyColor: ink, displayColor: c.indigo900),
    appBarTheme: AppBarTheme(
      backgroundColor: brand?.bg ?? c.paper,
      foregroundColor: brand?.ink ?? c.indigo900,
      elevation: 0,
      scrolledUnderElevation: 0,
      titleTextStyle: displayStyle(18, color: brand?.ink ?? c.indigo900),
    ),
    dividerColor: brand?.line ?? c.line,
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: brand?.bg ?? c.paper,
      contentPadding: const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
      border: OutlineInputBorder(borderRadius: BorderRadius.circular(12), borderSide: BorderSide(color: brand?.line ?? c.lineStrong)),
      enabledBorder: OutlineInputBorder(borderRadius: BorderRadius.circular(12), borderSide: BorderSide(color: brand?.line ?? c.lineStrong)),
      focusedBorder: OutlineInputBorder(borderRadius: BorderRadius.circular(12), borderSide: BorderSide(color: accent, width: 1.5)),
    ),
  );
}
