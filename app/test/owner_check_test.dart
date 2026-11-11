// Removing a device, turning one away and leaving the circle each revoke a
// key, so the system's own check that the owner is present comes first.

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:zerohop/l10n/app_localizations_en.dart';
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

Snapshot _snapshot({bool removed = false}) => Snapshot({
  'event': 'snapshot',
  'name': 'Mac',
  'me': {
    'id': 'ff',
    'short_id': 'F0F0F0F0',
    'name': 'Mac',
    'platform': 'mac',
    'removed': removed,
  },
  'devices': <dynamic>[],
  'settings': {
    'default_mode': 'smart',
    'folder': '/tmp',
    'folder_is_default': true,
    'visible': true,
    'confirm_new': false,
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

({AppState state, _RecordingCore core, List<Map<Object?, Object?>> asked})
_app({String? answer, bool fails = false, bool removed = false}) {
  final asked = <Map<Object?, Object?>>[];
  TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
      .setMockMethodCallHandler(const MethodChannel('dev.zerohop/platform'), (
        call,
      ) async {
        if (call.method != 'confirmOwner') return null;
        asked.add(call.arguments as Map<Object?, Object?>);
        if (fails) throw PlatformException(code: 'broken');
        return answer;
      });
  final core = _RecordingCore();
  final state = AppState(
    core: core,
    discovery: Discovery(core),
    platform: const PlatformBridge(ownerCheck: true),
  )..snapshot = _snapshot(removed: removed);
  return (state: state, core: core, asked: asked);
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

  Future<void> remove(AppState state) =>
      state.removeDevice('aa', title: 'Confirm', reason: 'remove Phone');

  test('a confirmed owner removes the device', () async {
    final app = _app(answer: 'passed');
    await remove(app.state);
    expect(app.core.methods, ['remove_device']);
  });

  test('the system is told why it asks', () async {
    final app = _app(answer: 'passed');
    await remove(app.state);
    expect(app.asked.single, {'title': 'Confirm', 'reason': 'remove Phone'});
  });

  test('a cancelled check removes nothing', () async {
    final app = _app(answer: 'refused');
    await remove(app.state);
    expect(app.core.methods, isEmpty);
  });

  test('a cancelled check says nothing changed', () async {
    final app = _app(answer: 'refused');
    await remove(app.state);
    expect(app.state.toast?.call(L10nEn()), L10nEn().ownerCheckRefused);
  });

  test('a device with no screen lock still removes', () async {
    final app = _app(answer: 'unavailable');
    await remove(app.state);
    expect(app.core.methods, ['remove_device']);
  });

  test('a check that breaks removes nothing', () async {
    final app = _app(fails: true);
    await remove(app.state);
    expect(app.core.methods, isEmpty);
  });

  test('an answer it does not know removes nothing', () async {
    final app = _app(answer: 'maybe');
    await remove(app.state);
    expect(app.core.methods, isEmpty);
  });

  test('turning a new device away asks first', () async {
    final app = _app(answer: 'refused');
    await app.state.confirmDevice('aa', false, title: 'C', reason: 'r');
    expect((app.asked.length, app.core.methods.isEmpty), (1, true));
  });

  test('letting a new device in does not ask', () async {
    final app = _app(answer: 'refused');
    await app.state.confirmDevice('aa', true, title: 'C', reason: 'r');
    expect(
      (app.asked.isEmpty, app.core.methods.join()),
      (true, 'confirm_device'),
    );
  });

  test('a member starting over asks first', () async {
    final app = _app(answer: 'refused');
    await app.state.startOver(title: 'C', reason: 'r');
    expect((app.asked.length, app.core.methods.isEmpty), (1, true));
  });

  test('a device already removed starts over without asking', () async {
    final app = _app(answer: 'refused', removed: true);
    await app.state.startOver(title: 'C', reason: 'r');
    expect((app.asked.isEmpty, app.core.methods.join()), (true, 'start_over'));
  });
}
