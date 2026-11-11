// Pairing by QR code: what the camera hands back decides what the core is
// asked to do, and nothing is sent when the owner closes the scanner.

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:zerohop/l10n/app_localizations_en.dart';
import 'package:zerohop/src/app_state.dart';
import 'package:zerohop/src/bridge/core.dart';
import 'package:zerohop/src/discovery.dart';
import 'package:zerohop/src/platform.dart';

class _RecordingCore extends Core {
  final calls = <(String, Map<String, dynamic>?)>[];

  @override
  Future<dynamic> call(String method, [Map<String, dynamic>? params]) async {
    calls.add((method, params));
    return null;
  }

  List<String> get methods => calls.map((c) => c.$1).toList();
}

const _link = 'zerohop://pair/1?k=AAAA&s=BBBB&a=192.168.1.20:53321';
const _words = {'title': 'Point', 'notZerohop': 'Not ours', 'cancel': 'Cancel'};

({AppState state, _RecordingCore core}) _app() {
  final core = _RecordingCore();
  final state = AppState(
    core: core,
    discovery: Discovery(core),
    platform: const PlatformBridge(scanner: true),
  );
  return (state: state, core: core);
}

void _camera(Future<Object?> Function(MethodCall call) answer) {
  TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
      .setMockMethodCallHandler(const MethodChannel('dev.zerohop/platform'), (
        call,
      ) {
        if (call.method != 'scanQr') return Future.value();
        return answer(call);
      });
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  tearDown(
    () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
          const MethodChannel('dev.zerohop/platform'),
          null,
        ),
  );

  test('a scanned code is handed to the core as it was read', () async {
    _camera((_) async => _link);
    final app = _app();
    await app.state.scanToJoin(_words);
    final call = app.core.calls.single;
    expect('${call.$1} ${call.$2?['link']}', 'pair_scan $_link');
  });

  test('the scanner gets its text in the owner\'s language', () async {
    Map<Object?, Object?>? seen;
    _camera((call) async {
      seen = call.arguments as Map<Object?, Object?>;
      return null;
    });
    await _app().state.scanToJoin(_words);
    expect(seen, _words);
  });

  test('closing the scanner asks the core for nothing', () async {
    _camera((_) async => null);
    final app = _app();
    await app.state.scanToJoin(_words);
    expect(app.core.calls, isEmpty);
  });

  test('a camera the owner refused is explained, not tried', () async {
    _camera((_) async => throw PlatformException(code: 'camera_denied'));
    final app = _app();
    await app.state.scanToJoin(_words);
    expect(
      (app.core.calls.isEmpty, app.state.toast?.call(L10nEn())),
      (true, L10nEn().errCameraDenied),
    );
  });

  test('a phone without a camera is told to use the digits', () async {
    _camera((_) async => throw PlatformException(code: 'no_camera'));
    final app = _app();
    await app.state.scanToJoin(_words);
    expect(app.state.toast?.call(L10nEn()), L10nEn().errNoCamera);
  });

  test(
    'a new code ends the old pairing mode before opening a new one',
    () async {
      final app = _app();
      await app.state.renewCode();
      expect(app.core.methods, ['pair_cancel', 'pair_host']);
    },
  );
}
