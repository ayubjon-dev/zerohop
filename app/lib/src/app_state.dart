// What the interface is showing. The facts come from the core's snapshots;
// only where the owner is looking and what they have picked lives here.

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:file_selector/file_selector.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:path_provider/path_provider.dart';

import '../l10n/app_localizations.dart';
import 'bridge/core.dart';
import 'discovery.dart';
import 'format.dart';
import 'model.dart';
import 'platform.dart';

enum Screen {
  home,
  device,
  deviceSettings,
  removeDevice,
  history,
  settings,
  pair,
  pairShow,
  pairEnter,
}

typedef Message = String Function(L10n);

class AppState extends ChangeNotifier {
  AppState({
    required this.core,
    required this.discovery,
    required this.platform,
  });

  final Core core;
  final Discovery discovery;
  final PlatformBridge platform;

  Snapshot? snapshot;
  String? fatal;
  Screen screen = Screen.home;
  String? selected;
  Message? toast;
  bool dragging = false;
  final picked = <String, List<Picked>>{};
  final ended = <String, Ended>{};
  final _known = <String>{};
  Timer? _toastTimer;
  bool _keptAwake = false;
  bool _coreStarted = false;
  List<String>? _sharedWaiting;
  List<String>? _sharedForChoice;
  bool _clipboardOn = false;
  bool? _menuEntry;
  DateTime _startedAt = DateTime.now();
  static const _discoveryGrace = Duration(seconds: 10);

  /// What macOS lets this app read without asking.
  String clipboardAccess = 'allow';

  /// Copies stay on this Mac until the owner allows reading them unasked.
  bool get clipboardNeedsAllowing =>
      platform.clipboard == ClipboardSupport.automatic &&
      (snapshot?.clipboard ?? false) &&
      (clipboardAccess == 'ask' || clipboardAccess == 'deny');
  StreamSubscription<Map<String, dynamic>>? _subscription;

  Future<void> init() async {
    try {
      final support = await getApplicationSupportDirectory();
      final profile = Platform.environment['ZEROHOP_PROFILE'];
      final stateDir = profile == null
          ? support.path
          : '${support.path}/profile-$profile';
      final folder = await _defaultFolder(profile);
      if (!_coreStarted) {
        final secret = await _deviceSecret();
        if (platform.sealsDeviceKey && secret == null) {
          fatal = 'key_unavailable';
          notifyListeners();
          return;
        }
        core.start(
          stateDir: stateDir,
          defaultFolder: folder,
          keySecret: secret,
        );
        _coreStarted = true;
        _subscription = core.events.listen(_onEvent);
        if (platform.clipboard != ClipboardSupport.none) {
          platform.listenToClipboard(
            onCopied: (copied) => unawaited(_offerCopied(copied)),
            onAccess: (access) {
              clipboardAccess = access;
              notifyListeners();
            },
            onSend: _sendFromSystem,
            onShared: _receiveShared,
          );
        }
      }
      final settings = await core.call('load_settings') as Map<String, dynamic>;
      await _restoreFolderAccess(settings);
      await _finishStartup();
    } on CoreError catch (error) {
      fatal = error.kind;
      notifyListeners();
    }
  }

  /// The key store can be briefly unavailable while the system is starting.
  Future<Uint8List?> _deviceSecret() async {
    if (!platform.sealsDeviceKey) return null;
    for (var attempt = 0; attempt < 3; attempt++) {
      try {
        final secret = await platform.deviceSecret();
        if (secret != null) return secret;
      } on PlatformException {
        // Treated like no answer: the caller reports it.
      }
      await Future<void>.delayed(const Duration(seconds: 1));
    }
    return null;
  }

  Future<void> _finishStartup() async {
    await core.call('bootstrap', {'allow_loopback': false});
    unawaited(discovery.startBrowsing());
    _startedAt = DateTime.now();
    unawaited(platform.clipboardReady());
  }

  /// Tries again after the system would not hand over the device key.
  Future<void> retryStartup() async {
    fatal = null;
    notifyListeners();
    await init();
  }

  /// The key can never be opened again, so this device starts as new.
  Future<void> startOverAfterLostKey() async {
    fatal = null;
    notifyListeners();
    try {
      await core.call('start_over');
      await _finishStartup();
    } on CoreError catch (error) {
      fatal = error.kind;
      notifyListeners();
    }
  }

  Future<String> _defaultFolder(String? profile) async {
    final base =
        await platform.downloadsBase() ??
        (await getDownloadsDirectory())?.path ??
        (await getApplicationDocumentsDirectory()).path;
    return profile == null ? '$base/Zerohop' : '$base/Zerohop-$profile';
  }

  Future<void> _restoreFolderAccess(Map<String, dynamic> settings) async {
    final ui = settings['ui'] as Map<String, dynamic>? ?? const {};
    final bookmark = ui['folder_bookmark'] as String?;
    if (bookmark != null) await platform.resolveBookmark(bookmark);
  }

  @override
  void dispose() {
    _subscription?.cancel();
    _toastTimer?.cancel();
    core.dispose();
    super.dispose();
  }

  void _onEvent(Map<String, dynamic> event) {
    switch (event['event']) {
      case 'snapshot':
        _applySnapshot(Snapshot(event));
      case 'progress':
        _applyProgress(event['items'] as List<dynamic>);
      case 'transfer_ended':
        _applyEnded(Ended(event));
      case 'pairing_ended':
        _applyPairingEnded(event);
      case 'offer_arrived':
        unawaited(platform.activate());
      case 'clipboard_apply':
        unawaited(_applyClip(event));
    }
  }

  void _applySnapshot(Snapshot next) {
    for (final transfer in next.transfers) {
      if (_known.add(transfer.id)) _onTransferStarted(transfer);
    }
    _known.retainAll(next.transfers.map((t) => t.id));
    snapshot = next;
    if (selected != null && next.deviceById(selected) == null) {
      selected = null;
      if (screen == Screen.device ||
          screen == Screen.deviceSettings ||
          screen == Screen.removeDevice) {
        screen = Screen.home;
      }
    }
    final busy = next.transfers.isNotEmpty;
    if (busy != _keptAwake) {
      _keptAwake = busy;
      unawaited(platform.keepAwake(busy));
    }
    unawaited(discovery.applyAdvert(next.advert));
    unawaited(discovery.applyRoom(next.pairing));
    if (_sharedWaiting != null) {
      final waiting = _sharedWaiting!;
      _sharedWaiting = null;
      _receiveShared(waiting);
    }
    unawaited(_syncClipboard(next.clipboard));
    unawaited(_syncMenuEntry(next));
    notifyListeners();
  }

  Future<void> _syncClipboard(bool enabled) async {
    if (platform.clipboard == ClipboardSupport.none) return;
    if (enabled == _clipboardOn) return;
    _clipboardOn = enabled;
    await platform.setBackground(enabled);
    if (platform.clipboard == ClipboardSupport.automatic) {
      clipboardAccess = await platform.watchClipboard(enabled);
    }
    notifyListeners();
  }

  /// The entry exists only while the owner wants it and the clipboard is on.
  Future<void> _syncMenuEntry(Snapshot next) async {
    if (platform.clipboard != ClipboardSupport.oneTap) return;
    final wanted = next.clipboard && next.ui['menu_entry'] == true;
    if (wanted == _menuEntry) return;
    _menuEntry = wanted;
    await platform.setMenuEntry(wanted);
  }

  /// Files shared from another app. Nothing leaves until the owner taps Send.
  void _receiveShared(List<String> paths) {
    final current = snapshot;
    if (current == null) {
      _sharedWaiting = paths;
      return;
    }
    final devices = current.devices;
    if (devices.isEmpty) {
      _discard(
        paths.map((p) => Picked(path: p, name: p, isDirectory: false)).toList(),
      );
      show((t) => t.shareNoDevices);
    } else if (devices.length == 1) {
      addPaths(devices.first.id, paths);
      openDevice(devices.first.id);
    } else {
      _sharedForChoice = paths;
      open(Screen.home);
      show((t) => t.shareChooseDevice);
    }
  }

  /// Sends a copy made while the interface was not open. Just after starting
  /// the other devices may not have been found yet, so it tries a few times.
  Future<String> _sendFromSystem(Map<String, dynamic> copied) async {
    for (var attempt = 0; ; attempt++) {
      final reply = await _offerCopied(copied);
      final outcome = reply?['state'] as String? ?? 'failed';
      final justStarted =
          DateTime.now().difference(_startedAt) < _discoveryGrace;
      if (outcome != 'no_devices' || !justStarted || attempt >= 3) {
        return outcome;
      }
      await Future<void>.delayed(const Duration(seconds: 2));
    }
  }

  Future<void> _applyClip(Map<String, dynamic> event) async {
    if (platform.clipboard == ClipboardSupport.none) return;
    final data = event['data'] as String?;
    await platform.applyClipboard({
      'format': event['format'],
      'text': event['text'],
      'data': data == null ? null : base64Decode(data),
      'ttl_ms': event['ttl_ms'],
    });
    final name = event['from'] as String;
    show((t) => t.clipArrived(name));
  }

  Future<Map<String, dynamic>?> _offerCopied(
    Map<String, dynamic> copied,
  ) async {
    final data = copied['data'] as Uint8List?;
    try {
      final reply = await core.call('clip_changed', {
        'format': copied['format'],
        'text': ?copied['text'],
        if (data != null) 'data': base64Encode(data),
      });
      return reply as Map<String, dynamic>;
    } on CoreError {
      return null;
    }
  }

  void _onTransferStarted(Transfer transfer) {
    ended.remove(transfer.peer);
    final browsing = screen == Screen.home || screen == Screen.device;
    final free = (picked[selected] ?? const []).isEmpty;
    if (browsing && free) {
      selected = transfer.peer;
      screen = Screen.device;
    }
  }

  void _applyProgress(List<dynamic> items) {
    final current = snapshot;
    if (current == null) return;
    for (final item in items) {
      final map = item as Map<String, dynamic>;
      for (final transfer in current.transfers) {
        if (transfer.id == map['id']) {
          transfer.done = map['done'] as int;
          transfer.speed = (map['speed'] as num).toDouble();
        }
      }
    }
    notifyListeners();
  }

  void _applyEnded(Ended info) {
    ended[info.peer] = info;
    if (info.isDone && !info.isSent) unawaited(_announceReceived());
    if (info.isSent && (info.isDone || info.outcome == 'declined')) {
      _discard(picked.remove(info.peer) ?? const []);
    }
    final showing = screen == Screen.device && selected == info.peer;
    if (!showing) {
      if (info.isDone) {
        final name = info.peerName;
        show(
          (t) => info.isSent
              ? '${t.sentCount(info.files)}: $name'
              : '${t.receivedCount(info.files)}: $name',
        );
      } else if (info.outcome != 'cancelled') {
        show((t) => describeError(t, info.outcome));
      }
    }
    notifyListeners();
  }

  void _applyPairingEnded(Map<String, dynamic> event) {
    final outcome = event['outcome'] as String;
    discovery.forgetRooms();
    if (outcome == 'paired') {
      final name = event['name'] as String? ?? '';
      final device = event['device'] as String? ?? '';
      selected = device.isEmpty ? null : device;
      screen = device.isEmpty ? Screen.home : Screen.device;
      show((t) => t.pairedToast(name));
    } else if (outcome != 'cancelled') {
      screen = Screen.pair;
      show((t) => describePairing(t, outcome));
    }
    notifyListeners();
  }

  void show(Message message) {
    toast = message;
    _toastTimer?.cancel();
    _toastTimer = Timer(const Duration(milliseconds: 3200), () {
      toast = null;
      notifyListeners();
    });
    notifyListeners();
  }

  Future<void> _run(Future<void> Function() action) async {
    try {
      await action();
    } on CoreError catch (error) {
      show((t) => describeError(t, error.kind));
    }
  }

  void open(Screen next) {
    if (screen == Screen.pairEnter && next != Screen.pairEnter) {
      unawaited(discovery.browseRooms(false));
    }
    screen = next;
    notifyListeners();
  }

  void openDevice(String id) {
    final shared = _sharedForChoice;
    if (shared != null) {
      _sharedForChoice = null;
      addPaths(id, shared);
    }
    selected = id;
    open(Screen.device);
  }

  Future<void> setName(String name) => _run(() async {
    await core.call('set_name', {'name': name});
  });

  // ---- Picking ----

  Future<void> chooseFiles(String device) async {
    final files = await openFiles();
    addPaths(device, files.map((f) => f.path));
  }

  Future<void> chooseFolder(String device) async {
    final path = await getDirectoryPath();
    if (path != null) addPaths(device, [path]);
  }

  void addPaths(String device, Iterable<String> paths) {
    final list = picked.putIfAbsent(device, () => []);
    for (final path in paths) {
      if (list.any((p) => p.path == path)) continue;
      final isDirectory = FileSystemEntity.isDirectorySync(path);
      final name = path
          .split(Platform.pathSeparator)
          .lastWhere((part) => part.isNotEmpty, orElse: () => path);
      final item = Picked(path: path, name: name, isDirectory: isDirectory);
      list.add(item);
      if (isDirectory) {
        unawaited(_measure(item));
      } else {
        item.size = File(path).statSync().size;
      }
    }
    ended.remove(device);
    dragging = false;
    notifyListeners();
  }

  Future<void> _measure(Picked item) async {
    var total = 0;
    var files = 0;
    try {
      await for (final entity in Directory(
        item.path,
      ).list(recursive: true, followLinks: false)) {
        if (entity is File) {
          total += await entity.length();
          files++;
        }
      }
    } on FileSystemException {
      // The size is only informative; sending reports real problems.
    }
    item.size = total;
    item.files = files;
    notifyListeners();
  }

  void removePicked(String device, Picked item) {
    picked[device]?.remove(item);
    _discard([item]);
    notifyListeners();
  }

  void clearPicked(String device) {
    _discard(picked.remove(device) ?? const []);
    notifyListeners();
  }

  /// On Android the file picker hands over private copies. They take up room
  /// until removed, and each sits alone in a folder made for it.
  void _discard(List<Picked> items) {
    if (!Platform.isAndroid) return;
    unawaited(() async {
      final cache = (await getTemporaryDirectory()).path;
      for (final item in items) {
        if (!item.path.startsWith('$cache/')) continue;
        try {
          await File(item.path).parent.delete(recursive: true);
        } on FileSystemException {
          // Only a leftover in the cache, which the system can clear.
        }
      }
    }());
  }

  /// Handles the system's back gesture. False means leave the app.
  bool goBack() {
    final pairing = snapshot?.pairing;
    if (pairing != null && pairing.phase != 'waiting') return true;
    switch (screen) {
      case Screen.home:
        return false;
      case Screen.deviceSettings:
      case Screen.removeDevice:
        open(Screen.device);
      case Screen.pairShow:
        unawaited(cancelPairing());
      case Screen.pairEnter:
        open(Screen.pair);
      case Screen.device:
      case Screen.history:
      case Screen.settings:
      case Screen.pair:
        open(Screen.home);
    }
    return true;
  }

  void setDragging(bool value) {
    if (dragging == value) return;
    dragging = value;
    notifyListeners();
  }

  void dismissEnded(String device) {
    ended.remove(device);
    notifyListeners();
  }

  // ---- Actions ----

  Future<void> send(String device) => _run(() async {
    final paths = (picked[device] ?? const <Picked>[]).map((p) => p.path);
    await core.call('send', {'device': device, 'paths': paths.toList()});
  });

  Future<void> cancelTransfer(String id) => _run(() async {
    await core.call('cancel', {'transfer': id});
  });

  Future<void> resume(String id) => _run(() async {
    final peer = snapshot?.unfinished
        .where((u) => u.id == id)
        .map((u) => u.peer)
        .firstOrNull;
    if (peer != null) ended.remove(peer);
    await core.call('resume', {'transfer': id});
  });

  Future<void> forgetUnfinished(String id) => _run(() async {
    await core.call('forget_unfinished', {'transfer': id});
  });

  Future<void> answerOffer(int offer, bool accept) => _run(() async {
    await core.call('answer_offer', {'offer': offer, 'accept': accept});
  });

  Future<void> releaseHeld(Held item) => _run(() async {
    await core.call('release_held', {
      'transfer': item.transfer,
      'name': item.rawName,
    });
    await _announceReceived();
  });

  Future<void> _announceReceived() async {
    final folder = snapshot?.folder;
    if (folder != null) await platform.announceFiles(folder);
  }

  Future<void> discardHeld(Held item) => _run(() async {
    await core.call('discard_held', {
      'transfer': item.transfer,
      'name': item.rawName,
    });
  });

  /// Sends what is on the clipboard now, on the owner's request.
  Future<void> sendClipboard() async {
    final copied = await platform.readClipboard();
    final state = copied['state'] as String;
    if (state != 'ok') {
      show(
        (t) => switch (state) {
          'secret' => t.clipSecret,
          'same' => t.clipSame,
          'too_large' => t.clipTooLarge,
          'denied' => t.clipDenied,
          _ => t.clipEmpty,
        },
      );
      return;
    }
    final reply = await _offerCopied(copied);
    final outcome = reply?['state'] as String? ?? 'failed';
    show(
      (t) => switch (outcome) {
        'sent' || 'sending' => t.clipSent,
        'same' => t.clipSame,
        'no_devices' => t.clipNoDevices,
        'too_large' => t.clipTooLarge,
        'empty' => t.clipEmpty,
        _ => t.clipFailed,
      },
    );
  }

  Future<void> setClipboard(bool enabled) => _run(() async {
    if (enabled) await platform.askNotifications();
    await core.call('set_clipboard', {'enabled': enabled});
  });

  Future<void> addClipboardTile() async {
    final added = await platform.addTile();
    if (!added) show((t) => t.clipTileManual);
  }

  Future<void> setClipboardRule(String device, Map<String, bool> change) =>
      _run(() async {
        await core.call('set_clipboard_device', {'device': device, ...change});
      });

  Future<void> clearHistory() => _run(() async {
    await core.call('clear_history');
  });

  Future<void> setDeviceMode(String device, String? mode) => _run(() async {
    await core.call('set_device_mode', {'device': device, 'mode': mode});
  });

  Future<void> confirmDevice(String device, bool accept) => _run(() async {
    await core.call('confirm_device', {'device': device, 'accept': accept});
  });

  Future<void> removeDevice(String device) => _run(() async {
    await core.call('remove_device', {'device': device});
    picked.remove(device);
    ended.remove(device);
    selected = null;
    open(Screen.home);
  });

  Future<void> startOver() => _run(() async {
    await core.call('start_over');
    picked.clear();
    ended.clear();
    selected = null;
    open(Screen.home);
  });

  Future<void> setSetting(String key, Object? value) => _run(() async {
    await core.call('set_settings', {key: value});
  });

  Future<void> setUi(String key, Object? value) => _run(() async {
    final ui = Map<String, dynamic>.of(snapshot?.ui ?? const {});
    ui[key] = value;
    await core.call('set_settings', {'ui': ui});
  });

  Future<void> changeFolder() => _run(() async {
    final path = await getDirectoryPath();
    if (path == null) return;
    final bookmark = await platform.bookmark(path);
    final ui = Map<String, dynamic>.of(snapshot?.ui ?? const {});
    ui['folder_bookmark'] = bookmark;
    await core.call('set_settings', {'folder': path, 'ui': ui});
  });

  Future<void> showFolder() async {
    final folder = snapshot?.folder;
    if (folder != null) await platform.showFolder(folder);
  }

  // ---- Pairing ----

  Future<void> startHosting() => _run(() async {
    open(Screen.pairShow);
    try {
      await core.call('pair_host');
    } on CoreError {
      open(Screen.pair);
      rethrow;
    }
  });

  void startEntering() {
    open(Screen.pairEnter);
    unawaited(discovery.browseRooms(true));
  }

  Future<void> joinWith(String address, String code) => _run(() async {
    await core.call('pair_join', {'address': address, 'code': code});
  });

  Future<void> answerPairing(bool accept) => _run(() async {
    await core.call('pair_answer', {'accept': accept});
  });

  Future<void> cancelPairing() => _run(() async {
    await core.call('pair_cancel');
    open(Screen.pair);
  });
}
