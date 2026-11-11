// The shared clipboard on iPhone and iPad: it is sent with a tap in the app,
// after iOS asks, and nothing in the background or outside the app.

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:zerohop/l10n/app_localizations_en.dart';
import 'package:zerohop/main.dart';
import 'package:zerohop/src/app_state.dart';
import 'package:zerohop/src/bridge/core.dart';
import 'package:zerohop/src/discovery.dart';
import 'package:zerohop/src/model.dart';
import 'package:zerohop/src/platform.dart';

class _RecordingCore extends Core {
  final methods = <String>[];

  @override
  Future<dynamic> call(String method, [Map<String, dynamic>? params]) async {
    methods.add(method);
    return null;
  }
}

Snapshot _snapshot() => Snapshot({
  'event': 'snapshot',
  'name': 'iPhone',
  'me': {
    'id': 'ff',
    'short_id': 'F0F0F0F0',
    'name': 'iPhone',
    'platform': 'ios',
    'removed': false,
  },
  'devices': [
    {
      'id': 'aa',
      'short_id': 'A1B2C3D4',
      'name': 'MacBook',
      'platform': 'mac',
      'nearby': true,
      'mode': null,
      'awaiting_confirmation': false,
      'clipboard': {'send': true, 'receive': true},
    },
  ],
  'settings': {
    'default_mode': 'smart',
    'folder': '/tmp',
    'folder_is_default': true,
    'visible': true,
    'confirm_new': false,
    'clipboard': true,
    'ui': {'theme': 'light', 'language': 'en'},
  },
  'problem': null,
  'pairing': null,
  'offers': <dynamic>[],
  'transfers': <dynamic>[],
  'unfinished': <dynamic>[],
  'held': <dynamic>[],
  'history': <dynamic>[],
  'advert': null,
});

({AppState state, _RecordingCore core}) _app(
  ClipboardSupport support, {
  Map<String, Object?> read = const {'state': 'empty'},
}) {
  TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
      .setMockMethodCallHandler(const MethodChannel('dev.zerohop/platform'), (
        call,
      ) async {
        return call.method == 'clipboardRead' ? read : null;
      });
  final core = _RecordingCore();
  final state = AppState(
    core: core,
    discovery: Discovery(core),
    platform: PlatformBridge(clipboard: support),
  )..snapshot = _snapshot();
  return (state: state, core: core);
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final t = L10nEn();

  setUp(() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
          const MethodChannel('desktop_drop'),
          (_) async => null,
        );
  });

  tearDown(
    () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
          const MethodChannel('dev.zerohop/platform'),
          null,
        ),
  );

  group('sending with a tap', () {
    test('iPhone sends the clipboard with a button', () {
      const bridge = PlatformBridge(clipboard: ClipboardSupport.tapWhileOpen);
      expect(bridge.sendsClipboardOnTap, isTrue);
    });

    test('a refused paste says how to allow it on iPhone', () async {
      final app = _app(
        ClipboardSupport.tapWhileOpen,
        read: {'state': 'denied'},
      );
      await app.state.sendClipboard();
      expect(app.state.toast?.call(t), t.clipDeniedAsk);
    });

    test('a refused paste on Android keeps its own message', () async {
      final app = _app(ClipboardSupport.oneTap, read: {'state': 'denied'});
      await app.state.sendClipboard();
      expect(app.state.toast?.call(t), t.clipDenied);
    });

    test('a refused paste sends nothing', () async {
      final app = _app(
        ClipboardSupport.tapWhileOpen,
        read: {'state': 'denied'},
      );
      await app.state.sendClipboard();
      expect(app.core.methods, isEmpty);
    });

    test('what came from another device is not sent back', () async {
      final app = _app(ClipboardSupport.tapWhileOpen, read: {'state': 'same'});
      await app.state.sendClipboard();
      expect(
        (app.state.toast?.call(t), app.core.methods.length),
        (t.clipSame, 0),
      );
    });
  });

  group('what the screens offer', () {
    Future<void> show(
      WidgetTester tester,
      ClipboardSupport support,
      Screen screen,
    ) async {
      tester.view.physicalSize = const Size(390, 1400);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final app = _app(support);
      app.state.screen = screen;
      await tester.pumpWidget(ZerohopApp(state: app.state));
      await tester.pump(const Duration(milliseconds: 300));
    }

    testWidgets('iPhone settings explain when the clipboard arrives', (
      tester,
    ) async {
      await show(tester, ClipboardSupport.tapWhileOpen, Screen.settings);
      expect(find.text(t.clipboardSubWhileOpen), findsOneWidget);
    });

    testWidgets('iPhone settings offer no quick settings button', (
      tester,
    ) async {
      await show(tester, ClipboardSupport.tapWhileOpen, Screen.settings);
      expect(find.text(t.clipTileTitle), findsNothing);
    });

    testWidgets('iPhone settings offer no text menu entry', (tester) async {
      await show(tester, ClipboardSupport.tapWhileOpen, Screen.settings);
      expect(find.text(t.clipMenuTitle), findsNothing);
    });

    testWidgets('Android settings still offer the quick settings button', (
      tester,
    ) async {
      await show(tester, ClipboardSupport.oneTap, Screen.settings);
      expect(find.text(t.clipTileTitle), findsOneWidget);
    });

    testWidgets('the iPhone home screen has the Send button', (tester) async {
      await show(tester, ClipboardSupport.tapWhileOpen, Screen.home);
      expect(
        (
          find.text(t.clipboardSend).evaluate().length,
          find.text(t.clipboardWhileOpen).evaluate().length,
        ),
        (1, 1),
      );
    });
  });
}
