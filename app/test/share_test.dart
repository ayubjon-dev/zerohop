// Files handed over by Send to or another app's Share: the owner sees what they
// are, picks a device, and nothing leaves before Send is pressed.

import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
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
    return null;
  }

  Iterable<Map<String, dynamic>?> get sends =>
      calls.where((c) => c.$1 == 'send').map((c) => c.$2);
}

Map<String, dynamic> _device(String id, String name, {bool nearby = true}) => {
  'id': id,
  'short_id': 'A1B2C3D4',
  'name': name,
  'platform': 'mac',
  'nearby': nearby,
  'mode': null,
  'awaiting_confirmation': false,
  'clipboard': {'send': true, 'receive': true},
};

Snapshot _snapshot(List<Map<String, dynamic>> devices) => Snapshot({
  'event': 'snapshot',
  'name': 'Windows',
  'me': {
    'id': 'ff',
    'short_id': 'F0F0F0F0',
    'name': 'Windows',
    'platform': 'windows',
    'removed': false,
  },
  'devices': devices,
  'settings': {
    'default_mode': 'smart',
    'folder': '/tmp',
    'folder_is_default': true,
    'visible': true,
    'confirm_new': false,
    'ui': {'theme': 'light', 'language': 'ru'},
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
  List<Map<String, dynamic>> devices,
) {
  final core = _RecordingCore();
  final state = AppState(
    core: core,
    discovery: Discovery(core),
    platform: PlatformBridge(clipboard: ClipboardSupport.automatic),
  );
  state.snapshot = _snapshot(devices);
  return (state: state, core: core);
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  late Directory folder;
  late String photo;
  late String notes;

  setUp(() {
    folder = Directory.systemTemp.createTempSync('zerohop_share_');
    photo = (File(
      '${folder.path}/photo.jpg',
    )..writeAsBytesSync([1, 2, 3])).path;
    notes = (File('${folder.path}/notes.txt')..writeAsBytesSync([4])).path;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
          const MethodChannel('desktop_drop'),
          (_) async => null,
        );
  });

  tearDown(() => folder.deleteSync(recursive: true));

  group('choosing a device', () {
    test('the only device is chosen at once', () {
      final app = _app([_device('a', 'MacBook', nearby: false)]);
      app.state.receiveShared([photo]);
      expect(app.state.sharingTo, 'a');
    });

    test('the only nearby device is chosen among several', () {
      final app = _app([
        _device('a', 'MacBook', nearby: false),
        _device('b', 'Телефон'),
      ]);
      app.state.receiveShared([photo]);
      expect(app.state.sharingTo, 'b');
    });

    test('nothing is chosen when several devices are nearby', () {
      final app = _app([_device('a', 'MacBook'), _device('b', 'Телефон')]);
      app.state.receiveShared([photo]);
      expect(app.state.sharingTo, isNull);
    });

    test('files shared again join the ones already waiting', () {
      final app = _app([_device('a', 'MacBook'), _device('b', 'Телефон')]);
      app.state.receiveShared([photo]);
      app.state.chooseShareTarget('b');
      app.state.receiveShared([photo, notes]);
      expect(
        (app.state.sharing!.map((p) => p.name).join(', '), app.state.sharingTo),
        ('photo.jpg, notes.txt', 'b'),
      );
    });

    test('with no devices the files are not kept', () {
      final app = _app([]);
      app.state.receiveShared([photo]);
      expect(app.state.sharing, isNull);
    });
  });

  group('sending', () {
    test('Send sends only the shared files to the chosen device', () async {
      final app = _app([_device('a', 'MacBook'), _device('b', 'Телефон')]);
      app.state.receiveShared([photo, notes]);
      app.state.chooseShareTarget('a');
      app.state.sendShared();
      await Future<void>.delayed(Duration.zero);
      expect(app.core.sends.toList(), [
        {
          'device': 'a',
          'paths': [photo, notes],
        },
      ]);
    });

    test('Send opens the chosen device', () {
      final app = _app([_device('a', 'MacBook'), _device('b', 'Телефон')]);
      app.state.receiveShared([photo]);
      app.state.chooseShareTarget('b');
      app.state.sendShared();
      expect(
        (app.state.screen, app.state.selected, app.state.sharing),
        (Screen.device, 'b', null),
      );
    });

    test('a device that went away cannot be sent to', () async {
      final app = _app([_device('a', 'MacBook'), _device('b', 'Телефон')]);
      app.state.receiveShared([photo]);
      app.state.chooseShareTarget('a');
      app.state.snapshot = _snapshot([
        _device('a', 'MacBook', nearby: false),
        _device('b', 'Телефон'),
      ]);
      app.state.sendShared();
      await Future<void>.delayed(Duration.zero);
      expect((app.core.sends.isEmpty, app.state.sharing?.length), (true, 1));
    });

    test('files picked earlier wait on the device page with these', () async {
      final app = _app([_device('a', 'MacBook'), _device('b', 'Телефон')]);
      app.state.addPaths('a', [notes]);
      app.state.receiveShared([photo]);
      app.state.chooseShareTarget('a');
      app.state.sendShared();
      await Future<void>.delayed(Duration.zero);
      expect(
        (
          app.core.sends.isEmpty,
          app.state.picked['a']!.map((p) => p.name).join(', '),
        ),
        (true, 'notes.txt, photo.jpg'),
      );
    });

    test('Cancel sends nothing and closes the choice', () async {
      final app = _app([_device('a', 'MacBook')]);
      app.state.receiveShared([photo]);
      app.state.cancelShared();
      await Future<void>.delayed(Duration.zero);
      expect((app.core.sends.isEmpty, app.state.sharing), (true, null));
    });

    test('Back closes the choice', () {
      final app = _app([_device('a', 'MacBook')]);
      app.state.receiveShared([photo]);
      expect((app.state.goBack(), app.state.sharing), (true, null));
    });
  });

  group('the window', () {
    Future<_RecordingCore> show(
      WidgetTester tester,
      List<Map<String, dynamic>> devices,
      List<String> paths,
    ) async {
      tester.view.physicalSize = const Size(1280, 800);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final app = _app(devices);
      app.state.receiveShared(paths);
      await tester.pumpWidget(ZerohopApp(state: app.state));
      await tester.pump(const Duration(milliseconds: 300));
      return app.core;
    }

    testWidgets('names the files and asks for a device', (tester) async {
      await show(
        tester,
        [_device('a', 'MacBook'), _device('b', 'Телефон')],
        [photo, notes],
      );
      expect(
        (
          find.text('photo.jpg').evaluate().length,
          find.text('Выберите устройство').evaluate().length,
        ),
        (1, 1),
      );
    });

    testWidgets('tapping a device chooses it without sending', (tester) async {
      final core = await show(
        tester,
        [_device('a', 'MacBook'), _device('b', 'Телефон')],
        [photo],
      );
      await tester.tap(find.text('Телефон').last);
      await tester.pump();
      expect(
        (
          core.sends.isEmpty,
          find.text('Отправить на Телефон').evaluate().length,
        ),
        (true, 1),
      );
    });

    testWidgets('the Send button sends to the chosen device', (tester) async {
      final core = await show(
        tester,
        [_device('a', 'MacBook'), _device('b', 'Телефон')],
        [photo],
      );
      await tester.tap(find.text('Телефон').last);
      await tester.pump();
      await tester.tap(find.text('Отправить на Телефон'));
      await tester.pump();
      expect(core.sends.single?['device'], 'b');
    });

    testWidgets('Escape closes the window without sending', (tester) async {
      final core = await show(tester, [_device('a', 'MacBook')], [photo]);
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pump();
      expect(
        (core.sends.isEmpty, find.text('Куда отправить?').evaluate().isEmpty),
        (true, true),
      );
    });
  });
}
