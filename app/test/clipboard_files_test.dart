// Copied files go through the clipboard as a list: the files themselves move
// only when the owner pastes them on the other computer.

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
  final calls = <(String, Map<String, dynamic>?)>[];

  @override
  Future<dynamic> call(String method, [Map<String, dynamic>? params]) async {
    calls.add((method, params));
    return <String, dynamic>{'state': 'sent'};
  }
}

Snapshot _snapshot() => Snapshot({
  'event': 'snapshot',
  'name': 'Mac',
  'me': {
    'id': 'ff',
    'short_id': 'F0F0F0F0',
    'name': 'Mac',
    'platform': 'mac',
    'removed': false,
  },
  'devices': [
    {
      'id': 'aa',
      'short_id': 'A1B2C3D4',
      'name': 'Windows PC',
      'platform': 'windows',
      'nearby': true,
      'mode': null,
      'awaiting_confirmation': false,
      'clipboard': {'send': true, 'receive': true, 'files': false},
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

({AppState state, _RecordingCore core, List<MethodCall> sent}) _app(
  ClipboardSupport support, {
  Map<String, Object?> read = const {'state': 'empty'},
  bool pastesFiles = false,
  String? placed,
}) {
  final sent = <MethodCall>[];
  TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
      .setMockMethodCallHandler(const MethodChannel('dev.zerohop/platform'), (
        call,
      ) async {
        sent.add(call);
        return switch (call.method) {
          'clipboardRead' => read,
          'clipboardFiles' => placed,
          _ => null,
        };
      });
  final core = _RecordingCore();
  final state = AppState(
    core: core,
    discovery: Discovery(core),
    platform: PlatformBridge(clipboard: support, pastesFiles: pastesFiles),
  )..snapshot = _snapshot();
  return (state: state, core: core, sent: sent);
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

  group('the rule for files', () {
    test('is on unless the owner turned it off', () {
      expect(ClipRules({}).files, isTrue);
    });

    test('follows what the core says', () {
      expect(ClipRules({'files': false}).files, isFalse);
    });
  });

  group('copying files', () {
    test('hands the core their paths, not their contents', () async {
      final app = _app(
        ClipboardSupport.automatic,
        read: {
          'state': 'ok',
          'format': 'files',
          'paths': ['/Users/me/a.txt', '/Users/me/album'],
        },
      );
      await app.state.sendClipboard();
      expect(
        app.core.calls.map((c) => '${c.$1} ${c.$2}').join('; '),
        'clip_files {paths: [/Users/me/a.txt, /Users/me/album]}',
      );
    });
  });

  group('offering files for pasting', () {
    test(
      'a computer that can paste them puts their promise in place',
      () async {
        final app = _app(ClipboardSupport.automatic, pastesFiles: true);
        await app.state.platform.offerPastedFiles({
          'id': 'ab' * 16,
          'ttl_ms': 120000,
          'from': 'MacBook',
        });
        expect(
          app.sent.map((c) => '${c.method} ${c.arguments}').join('; '),
          'clipboardFiles {id: ${'ab' * 16}, ttl_ms: 120000}',
        );
      },
    );

    test('a device that cannot paste them is not asked', () async {
      final app = _app(ClipboardSupport.automatic);
      await app.state.platform.offerPastedFiles({'id': 'ab' * 16});
      expect(app.sent, isEmpty);
    });
  });

  group('on a Mac', () {
    final offer = {'id': 'ab' * 16, 'ttl_ms': 120000, 'from': 'Windows PC'};

    test('files ready to paste are announced', () async {
      final app = _app(
        ClipboardSupport.automatic,
        pastesFiles: true,
        placed: 'ready',
      );
      await app.state.offerFiles(offer);
      expect(app.state.toast!(t), t.clipFilesArrived('Windows PC'));
      expect(app.state.filesNeedSetup, isFalse);
    });

    test('until the file provider is on, the owner is told how', () async {
      final app = _app(
        ClipboardSupport.automatic,
        pastesFiles: true,
        placed: 'disabled',
      );
      await app.state.offerFiles(offer);
      expect(app.state.toast!(t), t.clipFilesSetUpFirst('Windows PC'));
      expect(app.state.filesNeedSetup, isTrue);
    });

    test('the system\'s word on the file provider reaches the app', () async {
      final app = _app(ClipboardSupport.automatic, pastesFiles: true);
      final heard = <String>[];
      app.state.platform.listenToClipboard(
        onCopied: (_) {},
        onAccess: (_) {},
        onSend: (_) async => 'sent',
        onShared: (_) {},
        onFilesState: heard.add,
      );
      await TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .handlePlatformMessage(
            'dev.zerohop/platform',
            const StandardMethodCodec().encodeMethodCall(
              const MethodCall('clipboardFilesState', {'state': 'ready'}),
            ),
            (_) {},
          );
      expect(heard, ['ready']);
    });

    testWidgets('settings offer the way to switch it on', (tester) async {
      tester.view.physicalSize = const Size(900, 1400);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final app = _app(ClipboardSupport.automatic, pastesFiles: true);
      app.state
        ..filesState = 'disabled'
        ..screen = Screen.settings;
      await tester.pumpWidget(ZerohopApp(state: app.state));
      await tester.pump(const Duration(milliseconds: 300));
      expect(find.text(t.clipFilesNeedSetup), findsOneWidget);
      await tester.tap(find.text(t.clipFilesSetUp));
      expect(app.sent.map((c) => c.method), contains('fileProviderSettings'));
    });
  });

  group('a device\'s clipboard settings', () {
    Future<void> show(WidgetTester tester, ClipboardSupport support) async {
      tester.view.physicalSize = const Size(900, 1400);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final app = _app(support);
      app.state.selected = 'aa';
      app.state.screen = Screen.deviceSettings;
      await tester.pumpWidget(ZerohopApp(state: app.state));
      await tester.pump(const Duration(milliseconds: 300));
    }

    testWidgets('a computer offers the switch for files', (tester) async {
      await show(tester, ClipboardSupport.automatic);
      expect(find.text(t.clipFiles), findsOneWidget);
    });

    testWidgets('a phone does not', (tester) async {
      await show(tester, ClipboardSupport.tapWhileOpen);
      expect(find.text(t.clipFiles), findsNothing);
    });
  });
}
