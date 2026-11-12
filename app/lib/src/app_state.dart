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

  /// True while iOS hands over the photos and videos just picked, which
  /// takes a while for large videos and for originals held in iCloud.
  bool loadingPhotos = false;
  final ended = <String, Ended>{};
  final _known = <String>{};
  Timer? _toastTimer;
  bool _keptAwake = false;
  bool _coreStarted = false;
  List<String>? _sharedWaiting;

  /// Files handed over by another app or by Send to, while the owner decides
  /// where they go. Nothing leaves until the owner presses Send.
  List<Picked>? sharing;

  /// The device the shared files would go to.
  String? sharingTo;
  bool? _clipboardOn;
  bool? _menuEntry;
  DateTime _startedAt = DateTime.now();
  static const _discoveryGrace = Duration(seconds: 10);

  /// What macOS lets this app read without asking.
  String clipboardAccess = 'allow';

  /// Whether files copied on other devices can be pasted here: on a Mac,
  /// "disabled" until the owner switches on Zerohop's file provider.
  String filesState = 'unknown';

  /// Files copied on other devices cannot be pasted on this Mac until the
  /// owner switches on Zerohop's file provider in System Settings.
  bool get filesNeedSetup =>
      platform.pastesFiles &&
      (snapshot?.clipboard ?? false) &&
      filesState == 'disabled';

  /// Copies stay on this Mac until the owner allows reading them unasked.
  bool get clipboardNeedsAllowing =>
      platform.clipboard == ClipboardSupport.automatic &&
      (snapshot?.clipboard ?? false) &&
      (clipboardAccess == 'ask' || clipboardAccess == 'deny');
  StreamSubscription<Map<String, dynamic>>? _subscription;

  Future<void> init() async {
    try {
      if (!_coreStarted) {
        _subscription ??= core.events.listen(_onEvent);
        if (platform.clipboard != ClipboardSupport.none) {
          platform.listenToClipboard(
            onCopied: (copied) => unawaited(_offerCopied(copied)),
            onAccess: (access) {
              clipboardAccess = access;
              notifyListeners();
            },
            onSend: _sendFromSystem,
            onShared: receiveShared,
            onPasteFailed: () => show((t) => t.clipPasteFailed),
            onFilesState: (state) {
              filesState = state;
              notifyListeners();
            },
            onFilesOffered: filesOffered,
          );
        }
        final started = platform.ownsCore
            ? await _attachCore()
            : await _startCore();
        if (!started) {
          fatal = 'key_unavailable';
          notifyListeners();
          return;
        }
        _coreStarted = true;
      }
      // On a Mac the system's own code has already done this.
      if (!platform.ownsCore) {
        final settings =
            await core.call('load_settings') as Map<String, dynamic>;
        await _restoreFolderAccess(settings);
      }
      await _finishStartup();
    } on CoreError catch (error) {
      fatal = error.kind;
      notifyListeners();
    }
  }

  /// Starts the core here. False when the system will not hand over what
  /// opens the device key.
  Future<bool> _startCore() async {
    final support = await _stateBase();
    final profile = Platform.environment['ZEROHOP_PROFILE'];
    final stateDir = profile == null
        ? support
        : '$support${Platform.pathSeparator}profile-$profile';
    final folder = await _defaultFolder(profile);
    final secret = await _deviceSecret();
    if (platform.sealsDeviceKey && secret == null) return false;
    core.start(stateDir: stateDir, defaultFolder: folder, keySecret: secret);
    return true;
  }

  /// Joins the core that the system's own code keeps running while the
  /// window is closed. False when the keychain will not hand over what opens
  /// the device key.
  Future<bool> _attachCore() async {
    final answer = await platform.attachCore(core.receive);
    final state = answer['state'] as String?;
    if (state == 'key_unavailable') return false;
    if (state != 'running') throw const CoreError('start');
    clipboardAccess = answer['access'] as String? ?? clipboardAccess;
    filesState = answer['files'] as String? ?? filesState;
    return true;
  }

  /// Where this device keeps its key and settings.
  Future<String> _stateBase() async =>
      (await getApplicationSupportDirectory()).path;

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
    // Also brings the newest snapshot, which a Mac interface joining a core
    // already running would otherwise not have.
    await core.call('bootstrap', {'allow_loopback': false});
    if (!platform.ownsCore) unawaited(discovery.startBrowsing());
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
    // The Files app already shows the app's documents under its own name.
    if (Platform.isIOS && profile == null) {
      return (await getApplicationDocumentsDirectory()).path;
    }
    final base =
        await platform.downloadsBase() ??
        (await getDownloadsDirectory())?.path ??
        (await getApplicationDocumentsDirectory()).path;
    final name = profile == null ? 'Zerohop' : 'Zerohop-$profile';
    return '$base${Platform.pathSeparator}$name';
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
      // On a Mac the system's own code answers these, and the interface
      // only hears how pasting files went.
      case 'offer_arrived':
        if (!platform.ownsCore) unawaited(platform.activate());
      case 'clipboard_apply':
        unawaited(_applyClip(event));
      case 'clipboard_files':
        if (!platform.ownsCore) unawaited(offerFiles(event));
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
    if (!platform.ownsCore) unawaited(discovery.applyAdvert(next.advert));
    unawaited(discovery.applyRoom(next.pairing));
    if (_sharedWaiting != null) {
      final waiting = _sharedWaiting!;
      _sharedWaiting = null;
      receiveShared(waiting);
    }
    unawaited(_syncClipboard(next.clipboard));
    unawaited(_syncMenuEntry(next));
    notifyListeners();
  }

  Future<void> _syncClipboard(bool enabled) async {
    if (platform.clipboard == ClipboardSupport.none || platform.ownsCore) {
      return;
    }
    // Everything starts off.
    final known = _clipboardOn ?? false;
    if (enabled == known) return;
    _clipboardOn = enabled;
    // iOS gives the app no way to keep listening once it is out of sight.
    if (platform.clipboard != ClipboardSupport.tapWhileOpen) {
      await platform.setBackground(enabled);
    }
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
  void receiveShared(List<String> paths) {
    final current = snapshot;
    if (current == null) {
      (_sharedWaiting ??= []).addAll(paths);
      return;
    }
    if (current.devices.isEmpty) {
      _discard(
        paths.map((p) => Picked(path: p, name: p, isDirectory: false)).toList(),
      );
      show((t) => t.shareNoDevices);
      return;
    }
    final list = sharing ??= [];
    for (final path in paths) {
      if (list.any((p) => p.path == path)) continue;
      list.add(_pick(path));
    }
    if (current.deviceById(sharingTo) == null) {
      sharingTo = _likelyTarget(current.devices);
    }
    notifyListeners();
  }

  /// The only device there is, or the only one that can take files now.
  static String? _likelyTarget(List<Device> devices) {
    if (devices.length == 1) return devices.first.id;
    final ready = devices.where(canReceive).toList();
    return ready.length == 1 ? ready.first.id : null;
  }

  /// Whether files can be sent to |device| right now.
  static bool canReceive(Device device) =>
      device.nearby && !device.awaitingConfirmation;

  void chooseShareTarget(String device) {
    sharingTo = device;
    notifyListeners();
  }

  void cancelShared() {
    _discard(sharing ?? const []);
    sharing = null;
    sharingTo = null;
    notifyListeners();
  }

  /// Puts the shared files on the chosen device's page and sends them.
  void sendShared() {
    final items = sharing;
    final device = snapshot?.deviceById(sharingTo);
    if (items == null || device == null || !canReceive(device)) return;
    sharing = null;
    sharingTo = null;
    final list = picked.putIfAbsent(device.id, () => []);
    // Files picked earlier for this device, or a transfer still on its way,
    // would be mixed into this send. The owner then sees everything together
    // and presses Send on the device's page.
    final alone = list.isEmpty && snapshot?.transferFor(device.id) == null;
    for (final item in items) {
      if (!list.any((p) => p.path == item.path)) list.add(item);
    }
    ended.remove(device.id);
    selected = device.id;
    open(Screen.device);
    if (alone) unawaited(send(device.id));
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
    if (!platform.ownsCore) {
      final data = event['data'] as String?;
      await platform.applyClipboard({
        'format': event['format'],
        'text': event['text'],
        'data': data == null ? null : base64Decode(data),
        'ttl_ms': event['ttl_ms'],
      });
    }
    final name = event['from'] as String;
    show((t) => t.clipArrived(name));
  }

  /// Files copied on another device: only their list has come, and the
  /// paste here fetches them.
  @visibleForTesting
  Future<void> offerFiles(Map<String, dynamic> event) async {
    if (!platform.pastesFiles) return;
    final placed = await platform.offerPastedFiles(event);
    filesOffered(event['from'] as String, placed);
  }

  /// Says how offering files copied on device [name] went: "ready" or null
  /// once they can be pasted, "disabled" until the owner switches on
  /// Zerohop's file provider.
  @visibleForTesting
  void filesOffered(String name, String? placed) {
    switch (placed) {
      case null || 'ready':
        show((t) => t.clipFilesArrived(name));
      case 'disabled':
        filesState = 'disabled';
        show((t) => t.clipFilesSetUpFirst(name));
    }
  }

  Future<Map<String, dynamic>?> _offerCopied(
    Map<String, dynamic> copied,
  ) async {
    if (copied['format'] == 'files') {
      try {
        final reply = await core.call('clip_files', {
          'paths': List<String>.from(copied['paths'] as List),
        });
        return reply as Map<String, dynamic>;
      } on CoreError {
        return null;
      }
    }
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

  /// The owner hears about any photo or video iOS could not hand over,
  /// rather than finding it silently left out.
  Future<void> choosePhotos(String device) async {
    if (loadingPhotos) return;
    loadingPhotos = true;
    notifyListeners();
    try {
      final result = await platform.pickPhotos();
      if (result.failed > 0) show((t) => t.photosFailed);
      addPaths(device, result.paths);
    } on PlatformException {
      show((t) => t.photosFailed);
    } finally {
      loadingPhotos = false;
      notifyListeners();
    }
  }

  Future<void> chooseFolder(String device) async {
    final path = await getDirectoryPath();
    if (path != null) addPaths(device, [path]);
  }

  void addPaths(String device, Iterable<String> paths) {
    final list = picked.putIfAbsent(device, () => []);
    for (final path in paths) {
      if (list.any((p) => p.path == path)) continue;
      list.add(_pick(path));
    }
    ended.remove(device);
    dragging = false;
    notifyListeners();
  }

  Picked _pick(String path) {
    final isDirectory = FileSystemEntity.isDirectorySync(path);
    final name = path
        .split(Platform.pathSeparator)
        .lastWhere((part) => part.isNotEmpty, orElse: () => path);
    final item = Picked(path: path, name: name, isDirectory: isDirectory);
    if (isDirectory) {
      unawaited(_measure(item));
    } else {
      item.size = File(path).statSync().size;
    }
    return item;
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

  /// On Android and iOS the file picker hands over private copies. They take
  /// up room until removed. On Android each sits alone in a folder made for
  /// it in the cache, while iOS puts them all side by side in the system's
  /// temporary folder. Copies from the iOS photo library, and files shared
  /// from other iOS apps, each sit alone in a folder of their own again,
  /// under [PlatformBridge.photosFolder] or [PlatformBridge.sharedFolder].
  void _discard(List<Picked> items) {
    if (!Platform.isAndroid && !Platform.isIOS) return;
    unawaited(() async {
      final ios = Platform.isIOS;
      final cache = ios
          ? _realPath(Directory.systemTemp.path)
          : (await getTemporaryDirectory()).path;
      final ownFolders = {
        '$cache/${PlatformBridge.photosFolder}',
        '$cache/${PlatformBridge.sharedFolder}',
      };
      for (final item in items) {
        final path = ios ? _realPath(item.path) : item.path;
        if (!path.startsWith('$cache/')) continue;
        final copy = File(path);
        final alone = !ios || ownFolders.contains(copy.parent.parent.path);
        try {
          await (alone ? copy.parent : copy).delete(recursive: true);
        } on FileSystemException {
          // Only a leftover in the cache, which the system can clear.
        }
      }
    }());
  }

  /// The same path with links resolved, as iOS reaches one folder by two.
  static String _realPath(String path) {
    try {
      return File(path).resolveSymbolicLinksSync();
    } on FileSystemException {
      return path;
    }
  }

  /// Handles the system's back gesture. False means leave the app.
  bool goBack() {
    if (sharing != null) {
      cancelShared();
      return true;
    }
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
          'denied' =>
            platform.clipboard == ClipboardSupport.tapWhileOpen
                ? t.clipDeniedAsk
                : t.clipDenied,
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

  /// Lets a device added elsewhere in, or turns it away. Turning it away
  /// revokes it, so the owner confirms first.
  Future<void> confirmDevice(
    String device,
    bool accept, {
    required String title,
    required String reason,
  }) => _run(() async {
    if (!accept && !await _ownerAgrees(title, reason)) return;
    await core.call('confirm_device', {'device': device, 'accept': accept});
  });

  /// Removing a device signs a revocation, which must not happen just
  /// because someone holds this device unlocked (protocol section 5.3).
  Future<bool> _ownerAgrees(String title, String reason) async {
    final check = await platform.confirmOwner(title: title, reason: reason);
    if (check == OwnerCheck.refused) {
      show((t) => t.ownerCheckRefused);
      return false;
    }
    return true;
  }

  Future<void> removeDevice(
    String device, {
    required String title,
    required String reason,
  }) => _run(() async {
    if (!await _ownerAgrees(title, reason)) return;
    await core.call('remove_device', {'device': device});
    picked.remove(device);
    ended.remove(device);
    selected = null;
    open(Screen.home);
  });

  /// Leaves the circle. A device the others already removed has nothing
  /// left to revoke, so only a current member asks the owner first.
  Future<void> startOver({required String title, required String reason}) =>
      _run(() async {
        final member = snapshot?.me != null && snapshot?.me?.removed != true;
        if (member && !await _ownerAgrees(title, reason)) return;
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

  /// Reads the other device's QR code with the camera and joins it.
  Future<void> scanToJoin(Map<String, String> words) => _run(() async {
    final String? link;
    try {
      link = await platform.scanQr(words);
    } on PlatformException catch (error) {
      final kind = error.code;
      show((t) => describeError(t, kind));
      return;
    }
    if (link == null) return;
    await core.call('pair_scan', {'link': link});
  });

  /// Starts over with a fresh code after the last one was spent.
  Future<void> renewCode() => _run(() async {
    await core.call('pair_cancel');
    await core.call('pair_host');
  });

  Future<void> answerPairing(bool accept) => _run(() async {
    await core.call('pair_answer', {'accept': accept});
  });

  Future<void> cancelPairing() => _run(() async {
    await core.call('pair_cancel');
    open(Screen.pair);
  });
}
