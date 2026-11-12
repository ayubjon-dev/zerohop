// On a Mac the system's own code runs the core, finds devices and handles the
// clipboard, so that nothing of Flutter stays in memory while the window is
// closed. The interface joins that core and only shows and asks.

import 'dart:convert';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:zerohop/l10n/app_localizations_en.dart';
import 'package:zerohop/src/app_state.dart';
import 'package:zerohop/src/bridge/core.dart';
import 'package:zerohop/src/discovery.dart';
import 'package:zerohop/src/platform.dart';

class _RecordingCore extends Core {
  final calls = <String>[];

  @override
  void start({
    required String stateDir,
    required String defaultFolder,
    Uint8List? keySecret,
  }) {
    calls.add('start');
  }

  @override
  Future<dynamic> call(String method, [Map<String, dynamic>? params]) async {
    calls.add(method);
    return null;
  }
}

Map<String, dynamic> _snapshot() => {
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
      'clipboard': {'send': true, 'receive': true, 'files': true},
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
  'advert': {'instance': 'a1b2c3', 'tag': 'ffee', 'port': 50123},
};

({
  AppState state,
  _RecordingCore core,
  List<MethodCall> asked,
  List<MethodCall> bonjour,
})
_app({String started = 'running'}) {
  final asked = <MethodCall>[];
  final bonjour = <MethodCall>[];
  TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
      .setMockMethodCallHandler(const MethodChannel('fr.skyost.bonsoir'), (
        call,
      ) async {
        bonjour.add(call);
        return true;
      });
  TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
      .setMockMethodCallHandler(const MethodChannel('dev.zerohop/platform'), (
        call,
      ) async {
        asked.add(call);
        return switch (call.method) {
          'attach' => {'state': started, 'access': 'ask', 'files': 'disabled'},
          _ => null,
        };
      });
  final core = _RecordingCore();
  final state = AppState(
    core: core,
    discovery: Discovery(core),
    platform: const PlatformBridge(
      clipboard: ClipboardSupport.automatic,
      pastesFiles: true,
      ownsCore: true,
    ),
  );
  return (state: state, core: core, asked: asked, bonjour: bonjour);
}

/// Sends what the system's own code would pass on from the core.
Future<void> _fromCore(Map<String, dynamic> message) async {
  final bytes = utf8.encode(jsonEncode(message));
  await TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
      .handlePlatformMessage(
        'dev.zerohop/core',
        ByteData.sublistView(bytes),
        (_) {},
      );
}

/// Sends a report from the system's own code, such as on pasted files.
Future<void> _fromSystem(String method, Map<String, dynamic> arguments) async {
  await TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
      .handlePlatformMessage(
        'dev.zerohop/platform',
        const StandardMethodCodec().encodeMethodCall(
          MethodCall(method, arguments),
        ),
        (_) {},
      );
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final t = L10nEn();

  tearDown(() {
    for (final name in ['dev.zerohop/platform', 'fr.skyost.bonsoir']) {
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(MethodChannel(name), null);
    }
  });

  test(
    'the interface joins the running core instead of starting one',
    () async {
      final app = _app();
      await app.state.init();
      expect(app.asked.map((c) => c.method), ['attach']);
      expect(app.core.calls, ['bootstrap']);
      expect(app.state.fatal, isNull);
    },
  );

  test('what macOS allows is known from the start', () async {
    final app = _app();
    await app.state.init();
    expect(app.state.clipboardAccess, 'ask');
    expect(app.state.filesState, 'disabled');
  });

  test('a keychain that keeps the key closed is explained', () async {
    final app = _app(started: 'key_unavailable');
    await app.state.init();
    expect(app.state.fatal, 'key_unavailable');
    expect(app.core.calls, isEmpty);
  });

  test('messages from the core reach the interface', () async {
    final app = _app();
    await app.state.init();
    await _fromCore(_snapshot());
    await pumpEventQueue();
    expect(app.state.snapshot?.devices.single.name, 'Windows PC');
  });

  test(
    'the interface neither looks for devices nor announces this one',
    () async {
      final app = _app();
      await app.state.init();
      await _fromCore(_snapshot());
      await pumpEventQueue();
      expect(app.bonjour, isEmpty);
    },
  );

  test(
    'a clip from another device is not put on the clipboard twice',
    () async {
      final app = _app();
      await app.state.init();
      await _fromCore(_snapshot());
      await _fromCore({
        'event': 'clipboard_apply',
        'format': 'text',
        'text': 'hello',
        'ttl_ms': 120000,
        'from': 'Windows PC',
      });
      await pumpEventQueue();
      expect(app.asked.map((c) => c.method), ['attach']);
      expect(app.state.toast!(t), t.clipArrived('Windows PC'));
    },
  );

  test('copied files are offered by macOS, and the owner hears how', () async {
    final app = _app();
    await app.state.init();
    await _fromCore(_snapshot());
    await _fromCore({
      'event': 'clipboard_files',
      'id': 'ab' * 16,
      'ttl_ms': 120000,
      'from': 'Windows PC',
    });
    await pumpEventQueue();
    expect(app.asked.map((c) => c.method), ['attach']);
    await _fromSystem('clipboardFilesOffered', {
      'from': 'Windows PC',
      'state': 'disabled',
    });
    expect(app.state.toast!(t), t.clipFilesSetUpFirst('Windows PC'));
    expect(app.state.filesNeedSetup, isTrue);
  });
}
