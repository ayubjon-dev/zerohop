// The few things only the operating system can do.

import 'dart:io';

import 'package:flutter/services.dart';

const _channel = MethodChannel('dev.zerohop/platform');

/// How this system lets copied content leave it.
enum ClipboardSupport { none, automatic, oneTap }

class PlatformBridge {
  const PlatformBridge({ClipboardSupport? clipboard}) : _clipboard = clipboard;

  final ClipboardSupport? _clipboard;

  bool get _isMac => Platform.isMacOS;
  bool get _isAndroid => Platform.isAndroid;

  ClipboardSupport get clipboard =>
      _clipboard ??
      (_isMac
          ? ClipboardSupport.automatic
          : _isAndroid
          ? ClipboardSupport.oneTap
          : ClipboardSupport.none);

  /// Systems whose key store protects the device key on disk.
  bool get sealsDeviceKey => _isMac || _isAndroid;

  /// The secret the system keeps for sealing the device key.
  Future<Uint8List?> deviceSecret() =>
      _channel.invokeMethod<Uint8List>('deviceSecret');

  /// Phones have no way to hand over a folder as a path.
  bool get canPickFolders => !_isAndroid && !Platform.isIOS;

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
    if (_isAndroid) {
      return await _channel.invokeMethod<String>('deviceName') ?? '';
    }
    return Platform.localHostname.split('.').first.replaceAll('-', ' ');
  }

  /// Shows a folder to the owner. Received files are never opened.
  Future<void> showFolder(String path) async {
    if (_isMac || _isAndroid) {
      await _channel.invokeMethod<void>('show', {'path': path});
    } else if (Platform.isWindows) {
      await Process.run('explorer', [path]);
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
    if (_isMac) await _channel.invokeMethod<void>('activate');
  }

  /// Hears about copies and permission changes on systems that report them.
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
          onCopied(arguments);
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
    if (_isAndroid) await _channel.invokeMethod<void>('clipboardReady');
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
    return copied ?? const {'state': 'empty'};
  }

  /// Puts what arrived on the system clipboard for its lifetime.
  Future<void> applyClipboard(Map<String, dynamic> clip) =>
      _channel.invokeMethod<void>('clipboardApply', clip);

  /// Opens the page of System Settings that controls clipboard access.
  Future<void> openClipboardSettings() =>
      _channel.invokeMethod<void>('clipboardSettings');

  /// Keeps a phone from sleeping in the middle of a transfer.
  Future<void> keepAwake(bool on) async {
    if (_isAndroid) await _channel.invokeMethod<void>('keepAwake', {'on': on});
  }
}
