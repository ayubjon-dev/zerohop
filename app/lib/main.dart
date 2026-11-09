import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_localizations/flutter_localizations.dart';

import 'l10n/app_localizations.dart';
import 'src/app_state.dart';
import 'src/bridge/core.dart';
import 'src/discovery.dart';
import 'src/platform.dart';
import 'src/theme.dart';
import 'src/ui/shell.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  LicenseRegistry.addLicense(() async* {
    final text = await rootBundle.loadString('assets/fonts/OFL.txt');
    yield LicenseEntryWithLineBreaks(['Onest'], text);
  });
  final core = Core();
  final state = AppState(
    core: core,
    discovery: Discovery(core),
    platform: const PlatformBridge(),
  );
  runApp(ZerohopApp(state: state));
  state.init();
}

class ZerohopApp extends StatelessWidget {
  const ZerohopApp({super.key, required this.state});

  final AppState state;

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: state,
      builder: (context, _) {
        final ui = state.snapshot?.ui ?? const <String, dynamic>{};
        final palette = ui['palette'] as String? ?? 'cobalt';
        final theme = ui['theme'] as String? ?? 'system';
        final language = ui['language'] as String?;
        return MaterialApp(
          title: 'Zerohop',
          debugShowCheckedModeBanner: false,
          localizationsDelegates: const [
            L10n.delegate,
            GlobalMaterialLocalizations.delegate,
            GlobalWidgetsLocalizations.delegate,
            GlobalCupertinoLocalizations.delegate,
          ],
          supportedLocales: L10n.supportedLocales,
          locale: language == null ? null : Locale(language),
          theme: themeFor(palette, Brightness.light),
          darkTheme: themeFor(palette, Brightness.dark),
          themeMode: switch (theme) {
            'light' => ThemeMode.light,
            'dark' => ThemeMode.dark,
            _ => ThemeMode.system,
          },
          home: AppShell(state: state),
        );
      },
    );
  }
}
