// The few things only the operating system can do.

import 'dart:io';

import 'package:flutter/services.dart';

import 'windows_images.dart';

const _channel = MethodChannel('dev.zerohop/platform');

/// How this system lets copied content leave it.
enum ClipboardSupport { none, automatic, oneTap }

/// How the system's own check that the owner is present went.
enum OwnerCheck {
  /// The owner confirmed with a fingerprint, face, PIN or password.
  passed,

  /// The device has no screen lock or similar to check with.
  unavailable,

  /// Cancelled, failed, or the check could not run.
  refused,
}

class PlatformBridge {
  const PlatformBridge({
    ClipboardSupport? clipboard,
    bool? photos,
    bool? scanner,
    bool? ownerCheck,
  }) : _clipboard = clipboard,
       _photos = photos,
       _scanner = scanner,
       _ownerCheck = ownerCheck;

  final ClipboardSupport? _clipboard;
  final bool? _photos;
  final bool? _scanner;
  final bool? _ownerCheck;

  bool get _isMac => Platform.isMacOS;
  bool get _isAndroid => Platform.isAndroid;
  bool get _isWindows => Platform.isWindows;

  ClipboardSupport get clipboard =>
      _clipboard ??
      (_isMac || _isWindows
          ? ClipboardSupport.automatic
          : _isAndroid
          ? ClipboardSupport.oneTap
          : ClipboardSupport.none);

  /// Systems whose key store protects the device key on disk.
  bool get sealsDeviceKey =>
      _isMac || _isAndroid || _isWindows || Platform.isIOS;

  /// The secret the keychain or the Android Keystore keeps for sealing the
  /// device key. On Windows the core asks the system itself.
  Future<Uint8List?> deviceSecret() =>
      _channel.invokeMethod<Uint8List>('deviceSecret');

  /// Systems whose own lock screen can confirm that the owner is present.
  bool get canConfirmOwner =>
      _ownerCheck ?? (_isMac || _isAndroid || _isWindows || Platform.isIOS);

  /// Asks the system to confirm that the owner is present, with Touch ID,
  /// Face ID, a fingerprint, Windows Hello or the device's password. [reason]
  /// completes "Zerohop is trying to ...". Anything unexpected counts as
  /// refused, so a broken check never lets a removal through.
  Future<OwnerCheck> confirmOwner({
    required String title,
    required String reason,
  }) async {
    if (!canConfirmOwner) return OwnerCheck.unavailable;
    try {
      final answer = await _channel.invokeMethod<String>('confirmOwner', {
        'title': title,
        'reason': reason,
      });
      return switch (answer) {
        'passed' => OwnerCheck.passed,
        'unavailable' => OwnerCheck.unavailable,
        _ => OwnerCheck.refused,
      };
    } on PlatformException {
      return OwnerCheck.refused;
    } on MissingPluginException {
      return OwnerCheck.refused;
    }
  }

  /// Phones can read a pairing QR code with their camera.
  bool get canScanQr => _scanner ?? (_isAndroid || Platform.isIOS);

  /// Opens the camera until it sees a Zerohop pairing code, and returns it.
  /// Null when the owner closes the scanner. Throws a [PlatformException]
  /// with code `camera_denied` or `no_camera` when it cannot start. [words]
  /// carries the scanner's text in the owner's language.
  Future<String?> scanQr(Map<String, String> words) =>
      _channel.invokeMethod<String>('scanQr', words);

  /// Phones have no way to hand over a folder as a path.
  bool get canPickFolders => !_isAndroid && !Platform.isIOS;

  /// iOS keeps photos and videos apart from the files the Files app shows.
  bool get canPickPhotos => _photos ?? Platform.isIOS;

  /// Lets the owner pick photos and videos. Each arrives as a copy of what
  /// iOS hands over, in a folder of its own under [photosFolder]. Failed
  /// counts the ones iOS could not hand over, such as an original held only
  /// in iCloud while offline.
  Future<({List<String> paths, int failed})> pickPhotos() async {
    final picked = await _channel.invokeMapMethod<String, dynamic>(
      'pickPhotos',
    );
    return (
      paths: List<String>.from(picked?['paths'] as List? ?? const []),
      failed: picked?['failed'] as int? ?? 0,
    );
  }

  /// The folder within the system's temporary folder that holds the copies
  /// of picked photos and videos.
  static const photosFolder = 'Photos';

  /// Remembers access to a folder the owner picked, across restarts.
  Future<String?> bookmark(String path) async {
    if (!_isMac) return null;
    return _channel.invokeMethod<String>('bookmark', {'path': path});
  }

  /// Regains access to a remembered folder and returns its path.
  Future<String?> resolveBookmark(String bookmark) async {
    if (!_isMac) return null;
    return _channel.invokeMethod<String>('resolve', {'bookmark': bookmark});
  }

  /// Where received files belong, if the system has a say in it.
  Future<String?> downloadsBase() async {
    if (!_isAndroid) return null;
    return _channel.invokeMethod<String>('downloads');
  }

  /// A name for this device to start from.
  Future<String> suggestedName() async {
    if (_isAndroid || Platform.isIOS) {
      return await _channel.invokeMethod<String>('deviceName') ?? '';
    }
    return Platform.localHostname.split('.').first.replaceAll('-', ' ');
  }

  /// Shows a folder to the owner. Received files are never opened.
  Future<void> showFolder(String path) async {
    if (_isMac || _isAndroid || Platform.isIOS) {
      await _channel.invokeMethod<void>('show', {'path': path});
    } else if (_isWindows) {
      // Explorer opens a different folder when given forward slashes.
      await Process.run('explorer', [path.replaceAll('/', r'\')]);
    }
  }

  /// Tells the system about new files so that other apps can see them.
  Future<void> announceFiles(String folder) async {
    if (_isAndroid) {
      await _channel.invokeMethod<void>('announce', {'path': folder});
    }
  }

  /// Brings the window forward when someone needs an answer.
  Future<void> activate() async {
    if (_isMac || _isWindows) {
      await _channel.invokeMethod<void>('activate');
    }
  }

  /// Hears about copies and permission changes on systems that report them,
  /// and about files handed over from other apps.
  void listenToClipboard({
    required void Function(Map<String, dynamic> copied) onCopied,
    required void Function(String access) onAccess,
    required Future<String> Function(Map<String, dynamic> copied) onSend,
    required void Function(List<String> paths) onShared,
  }) {
    _channel.setMethodCallHandler((call) async {
      final arguments = Map<String, dynamic>.from(call.arguments as Map);
      switch (call.method) {
        case 'clipboardCopied':
          onCopied(await _fromSystem(arguments));
        case 'clipboardAccess':
          onAccess(arguments['state'] as String);
        case 'clipboardSend':
          return onSend(arguments);
        case 'sharedFiles':
          onShared(List<String>.from(arguments['paths'] as List));
      }
      return null;
    });
  }

  /// Says that requests from outside the interface can now be answered.
  Future<void> clipboardReady() async {
    if (_isAndroid || _isWindows) {
      await _channel.invokeMethod<void>('clipboardReady');
    }
  }

  /// Starts or stops working with nothing on the screen.
  Future<void> setBackground(bool on) =>
      _channel.invokeMethod<void>('background', {'on': on});

  /// Asks for the right to show the notification with the send button.
  Future<void> askNotifications() async {
    if (_isAndroid) await _channel.invokeMethod<void>('askNotifications');
  }

  /// Adds or removes the entry in the text menu and the share sheet.
  Future<void> setMenuEntry(bool on) =>
      _channel.invokeMethod<void>('menuEntry', {'on': on});

  /// Offers to add the send button to the quick settings shade.
  Future<bool> addTile() async {
    final added = await _channel.invokeMethod<bool>('addTile');
    return added ?? false;
  }

  /// Starts or stops watching for copies, and says what macOS allows.
  Future<String> watchClipboard(bool on) async {
    final access = await _channel.invokeMethod<String>('clipboardWatch', {
      'on': on,
    });
    return access ?? 'allow';
  }

  /// Reads the clipboard now, on the owner's request.
  Future<Map<String, dynamic>> readClipboard() async {
    final copied = await _channel.invokeMapMethod<String, dynamic>(
      'clipboardRead',
    );
    return _fromSystem(copied ?? const {'state': 'empty'});
  }

  /// Puts what arrived on the system clipboard for its lifetime.
  Future<void> applyClipboard(Map<String, dynamic> clip) async {
    final ready = _isWindows && clip['format'] != 'text'
        ? await _forWindows(clip)
        : clip;
    if (ready != null) {
      await _channel.invokeMethod<void>('clipboardApply', ready);
    }
  }

  /// Windows hands over pictures as its own bitmaps, and a copied PNG can
  /// carry stray bytes after the image. Either way a fresh PNG leaves.
  Future<Map<String, dynamic>> _fromSystem(Map<String, dynamic> copied) async {
    final format = copied['format'];
    final data = copied['data'];
    if (!_isWindows ||
        copied['state'] != 'ok' ||
        (format != 'png' && format != 'dib') ||
        data is! Uint8List) {
      return copied;
    }
    try {
      final png = format == 'dib'
          ? await pngFromDib(data)
          : await pngFromImage(data);
      if (png == null) return const {'state': 'empty'};
      if (png.length > maxClipBytes) return const {'state': 'too_large'};
      return {'state': 'ok', 'format': 'png', 'data': png};
    } on PictureTooLarge {
      return const {'state': 'too_large'};
    }
  }

  /// Many Windows programs paste only the system's own bitmap, and others
  /// prefer PNG, so both go on the clipboard. Null when it is not a picture.
  Future<Map<String, dynamic>?> _forWindows(Map<String, dynamic> clip) async {
    final data = clip['data'];
    if (data is! Uint8List) return null;
    final isPng = clip['format'] == 'png';
    try {
      final dib = await dibFromImage(data);
      final png = isPng ? data : await pngFromImage(data);
      if (png == null || dib == null) return null;
      return {...clip, 'format': 'png', 'data': png, 'dib': dib};
    } on PictureTooLarge {
      // Programs that read PNG can still take it as it came.
      return isPng ? {...clip, 'dib': null} : null;
    }
  }

  /// Opens the page of System Settings that controls clipboard access.
  Future<void> openClipboardSettings() =>
      _channel.invokeMethod<void>('clipboardSettings');

  /// Keeps a phone from sleeping in the middle of a transfer.
  Future<void> keepAwake(bool on) async {
    if (_isAndroid || Platform.isIOS) {
      await _channel.invokeMethod<void>('keepAwake', {'on': on});
    }
  }
}
