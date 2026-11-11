// Renders every screen at every window shape, in both themes and languages.
// A widget that does not fit fails the test.

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:zerohop/main.dart';
import 'package:zerohop/src/app_state.dart';
import 'package:zerohop/src/bridge/core.dart';
import 'package:zerohop/src/discovery.dart';
import 'package:zerohop/src/model.dart';
import 'package:zerohop/src/platform.dart';

const _shots = String.fromEnvironment('ZH_SHOTS');
const _phone = 'a1';
const _laptop = 'b2';
const _longName = 'Очень длинное имя устройства пользователя';

Map<String, dynamic> _device(
  String id,
  String name,
  String platform,
  bool nearby, {
  bool awaiting = false,
}) => {
  'id': id,
  'short_id': 'A1B2C3D4',
  'name': name,
  'platform': platform,
  'nearby': nearby,
  'mode': null,
  'awaiting_confirmation': awaiting,
  'clipboard': {'send': true, 'receive': true, 'text': true, 'images': false},
};

Map<String, dynamic> _base(String theme, String language) => {
  'event': 'snapshot',
  'name': 'MacBook',
  'me': {
    'id': 'ff',
    'short_id': 'F0F0F0F0',
    'name': 'MacBook',
    'platform': 'mac',
    'removed': false,
  },
  'devices': [
    _device(_phone, _longName, 'ios', true),
    _device(_laptop, 'Рабочий', 'windows', false),
  ],
  'settings': {
    'default_mode': 'smart',
    'folder':
        '/Users/me/Library/Containers/dev.zerohop.app/Data/Downloads/Zerohop',
    'folder_is_default': true,
    'visible': true,
    'confirm_new': false,
    'ui': {'theme': theme, 'language': language},
  },
  'problem': null,
  'pairing': null,
  'offers': <dynamic>[],
  'transfers': <dynamic>[],
  'unfinished': <dynamic>[],
  'held': <dynamic>[],
  'history': <dynamic>[],
  'advert': null,
};

Map<String, dynamic> _transfer(int done) => {
  'id': 't1',
  'direction': 'sent',
  'peer': _phone,
  'peer_name': _longName,
  'files': 12,
  'total_bytes': 1500000000,
  'name': 'video.mp4',
  'done': done,
  'speed': 48000000.0,
};

Map<String, dynamic> _ended(
  String outcome, {
  bool sent = true,
  int held = 0,
  bool resume = false,
}) => {
  'id': 't1',
  'direction': sent ? 'sent' : 'received',
  'peer': _phone,
  'peer_name': _longName,
  'files': 3,
  'bytes': 41000000,
  'name': 'video.mp4',
  'outcome': outcome,
  'can_resume': resume,
  'held': held,
};

Map<String, dynamic> _pairing(String role, String phase) => {
  'role': role,
  'phase': phase,
  'code': '493027',
  'code_spent': false,
  'qr':
      'zerohop://pair/1?k=BAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQ&s=BQUFBQUFBQUFBQUFBQUFBQ'
      '&a=192.168.1.20:53321&a=10.0.0.4:53321',
  'addresses': ['192.168.1.20:53319', '10.0.0.4:53319'],
  'room': '4F2A',
  'port': 53319,
  'comparison': '284613',
  'peer_name': _longName,
  'peer_platform': 'android',
  'peer_short_id': 'A1B2C3D4',
  'deadline_ms': DateTime.now().millisecondsSinceEpoch + 90000,
};

typedef Setup = void Function(AppState state, Map<String, dynamic> raw);

void _select(AppState s, Screen screen) {
  s.selected = _phone;
  s.screen = screen;
}

List<Picked> _files() => [
  for (var i = 0; i < 14; i++)
    Picked(
      path: '/tmp/file$i.jpg',
      name: 'Очень длинное название файла с фотографией номер $i.jpg',
      isDirectory: i == 3,
    )..size = i == 3 ? null : 4800000 * (i + 1),
];

final Map<String, Setup> _scenarios = {
  'onboarding': (s, r) => r['name'] = null,
  'empty': (s, r) {
    r['devices'] = <dynamic>[];
    r['me'] = null;
  },
  'home': (s, r) => s.screen = Screen.home,
  'drop': (s, r) => _select(s, Screen.device),
  'drop_offline': (s, r) {
    _select(s, Screen.device);
    s.selected = _laptop;
  },
  'share': (s, r) {
    s.screen = Screen.home;
    s.sharing = _files();
    s.sharingTo = _phone;
  },
  'share_unchosen': (s, r) {
    s.screen = Screen.home;
    s.sharing = _files().take(2).toList();
    r['devices'] = [
      _device(_phone, _longName, 'ios', true),
      _device(_laptop, 'Рабочий', 'windows', true),
      _device('c3', 'Планшет', 'android', false),
    ];
  },
  'share_nobody': (s, r) {
    s.screen = Screen.home;
    s.sharing = _files().take(1).toList();
    r['devices'] = [_device(_laptop, 'Рабочий', 'windows', false)];
  },
  'chosen': (s, r) {
    _select(s, Screen.device);
    s.picked[_phone] = _files();
  },
  'drop_photos': (s, r) => _select(s, Screen.device),
  'chosen_photos': (s, r) {
    _select(s, Screen.device);
    s.picked[_phone] = _files();
  },
  'chosen_photos_loading': (s, r) {
    _select(s, Screen.device);
    s.picked[_phone] = _files();
    s.loadingPhotos = true;
  },
  'waiting': (s, r) {
    _select(s, Screen.device);
    r['transfers'] = [_transfer(0)];
  },
  'transfer': (s, r) {
    _select(s, Screen.device);
    r['transfers'] = [_transfer(640000000)];
  },
  'done_sent': (s, r) {
    _select(s, Screen.device);
    s.ended[_phone] = Ended(_ended('done', held: 1));
  },
  'done_received': (s, r) {
    _select(s, Screen.device);
    s.ended[_phone] = Ended(_ended('done', sent: false, held: 2));
  },
  'failed': (s, r) {
    _select(s, Screen.device);
    s.ended[_phone] = Ended(_ended('connection_lost', resume: true));
    r['unfinished'] = [
      {
        'id': 't1',
        'peer': _phone,
        'peer_name': _longName,
        'files': 3,
        'total_bytes': 41000000,
        'name': 'video.mp4',
      },
    ];
  },
  'device_settings': (s, r) => _select(s, Screen.deviceSettings),
  'remove': (s, r) => _select(s, Screen.removeDevice),
  'pair': (s, r) => s.screen = Screen.pair,
  'pair_show': (s, r) {
    s.screen = Screen.pairShow;
    r['pairing'] = _pairing('host', 'waiting');
  },
  'pair_phone': (s, r) => s.screen = Screen.pair,
  'pair_show_spent': (s, r) {
    s.screen = Screen.pairShow;
    r['pairing'] = _pairing('host', 'waiting')
      ..['code'] = null
      ..['code_spent'] = true;
  },
  'pair_show_offline': (s, r) {
    s.screen = Screen.pairShow;
    r['pairing'] = _pairing('host', 'waiting')
      ..['qr'] = null
      ..['addresses'] = <String>[];
  },
  'pair_enter': (s, r) {
    s.screen = Screen.pairEnter;
    s.discovery.rooms.value = const [
      Room(instance: 'a', label: '4F2A', addresses: ['192.168.1.2'], port: 1),
      Room(instance: 'b', label: '9C31', addresses: ['192.168.1.3'], port: 1),
    ];
  },
  'connecting': (s, r) => r['pairing'] = _pairing('join', 'connecting'),
  'compare': (s, r) => r['pairing'] = _pairing('host', 'compare'),
  'finishing': (s, r) => r['pairing'] = _pairing('join', 'finishing'),
  'history_empty': (s, r) => s.screen = Screen.history,
  'history': (s, r) {
    s.screen = Screen.history;
    r['held'] = [
      {
        'transfer': 't',
        'peer_name': _longName,
        'name': 'Очень длинное название файла который удержан.jpg',
        'raw_name': 'x',
        'is_dir': false,
        'bytes': 30,
        'arrived_ms': 1,
      },
    ];
    r['unfinished'] = [
      {
        'id': 't1',
        'peer': _phone,
        'peer_name': _longName,
        'files': 3,
        'total_bytes': 41000000,
        'name': 'video.mp4',
      },
    ];
    r['history'] = [
      for (final outcome in [
        'done',
        'declined',
        'cancelled',
        'connection_lost',
      ])
        {
          'id': 'h',
          'at_ms': 1794229000000,
          'direction': 'received',
          'peer_name': _longName,
          'files': 21,
          'bytes': 18400000,
          'name': 'a',
          'outcome': outcome,
          'held': 0,
        },
    ];
  },
  'settings': (s, r) => s.screen = Screen.settings,
  'offer': (s, r) {
    _select(s, Screen.device);
    r['offers'] = [
      {
        'id': 1,
        'peer': _phone,
        'peer_name': _longName,
        'files': 3,
        'total_bytes': 118000000,
        'has_program': true,
        'first_program': 'очень/длинный/путь/к/установщику/setup.exe',
      },
    ];
  },
  'clip_on': (s, r) {
    r['settings']['clipboard'] = true;
    s.screen = Screen.home;
  },
  'clip_tap': (s, r) {
    r['settings']['clipboard'] = true;
    s.screen = Screen.home;
  },
  'clip_tap_off': (s, r) => s.screen = Screen.home,
  'clip_tap_settings': (s, r) {
    r['settings']['clipboard'] = true;
    s.screen = Screen.settings;
  },
  'clip_allow_device': (s, r) {
    r['settings']['clipboard'] = true;
    s.clipboardAccess = 'ask';
    _select(s, Screen.device);
  },
  'clip_allow_home': (s, r) {
    r['settings']['clipboard'] = true;
    s.clipboardAccess = 'deny';
    s.screen = Screen.home;
  },
  'clip_allow_settings': (s, r) {
    r['settings']['clipboard'] = true;
    s.clipboardAccess = 'ask';
    s.screen = Screen.settings;
  },
  'clip_rules_on': (s, r) {
    r['settings']['clipboard'] = true;
    _select(s, Screen.deviceSettings);
  },
  'clip_device_off': (s, r) {
    r['settings']['clipboard'] = true;
    final device = _device(_phone, _longName, 'ios', true);
    device['clipboard'] = {'send': false, 'receive': true};
    r['devices'] = [device];
    _select(s, Screen.device);
  },
  'clip_unsupported': (s, r) => s.screen = Screen.settings,
  'key_unavailable': (s, r) => s.fatal = 'key_unavailable',
  'key_locked': (s, r) => s.fatal = 'key_locked',
  'notices': (s, r) {
    (r['me'] as Map)['removed'] = true;
    r['problem'] = 'folder_unavailable';
    r['devices'] = [_device(_phone, _longName, 'ios', true, awaiting: true)];
  },
};

/// Scenarios that show what a phone would, or a system without the feature.
const _clipboardSupport = {
  'clip_tap': ClipboardSupport.oneTap,
  'clip_tap_off': ClipboardSupport.oneTap,
  'clip_tap_settings': ClipboardSupport.oneTap,
  'clip_unsupported': ClipboardSupport.none,
};

/// Scenarios rendered as on iOS, with the button for the photo library.
const _withPhotos = {'drop_photos', 'chosen_photos', 'chosen_photos_loading'};

const _sizes = {
  'wide': Size(1280, 800),
  'window': Size(780, 620),
  'phone': Size(390, 844),
  'small': Size(380, 620),
};

Future<void> _render(
  WidgetTester tester,
  String name,
  Size size,
  String theme,
  String language,
) async {
  tester.view.physicalSize = size;
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  final core = Core();
  final state = AppState(
    core: core,
    discovery: Discovery(core),
    platform: PlatformBridge(
      clipboard: _clipboardSupport[name] ?? ClipboardSupport.automatic,
      photos: _withPhotos.contains(name),
      scanner: name == 'pair_phone',
    ),
  );
  final raw = _base(theme, language);
  _scenarios[name]!(state, raw);
  state.snapshot = Snapshot(raw);
  await tester.pumpWidget(ZerohopApp(state: state));
  await tester.pump(const Duration(milliseconds: 300));
  if (_shots.isNotEmpty) {
    await expectLater(
      find.byType(ZerohopApp),
      matchesGoldenFile(
        '$_shots/${name}_${size.width.toInt()}_${theme}_$language.png',
      ),
    );
  }
  expect(
    tester.takeException(),
    isNull,
    reason: '$name at $size, $theme, $language',
  );
  await tester.pumpWidget(const SizedBox());
  await tester.pump(const Duration(seconds: 2));
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  setUpAll(() async {
    final loader = FontLoader('Onest')
      ..addFont(rootBundle.load('assets/fonts/Onest-Variable.ttf'));
    await loader.load();
  });

  setUp(() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
          const MethodChannel('desktop_drop'),
          (_) async => null,
        );
  });

  if (_shots.isNotEmpty) {
    for (final name in _scenarios.keys) {
      testWidgets('shot $name', (tester) async {
        await _render(tester, name, _sizes['phone']!, 'light', 'ru');
      });
    }
    return;
  }

  for (final name in _scenarios.keys) {
    for (final entry in _sizes.entries) {
      testWidgets('$name fits ${entry.key}', (tester) async {
        await _render(tester, name, entry.value, 'light', 'ru');
        await _render(tester, name, entry.value, 'dark', 'en');
      });
    }
  }
}
