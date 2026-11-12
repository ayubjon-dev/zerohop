// Requests to the core and the messages coming back. All behaviour lives in
// the core: this file only carries text across the boundary.

import 'dart:async';
import 'dart:convert';
import 'dart:ffi';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

import 'native.dart';

class CoreError implements Exception {
  const CoreError(this.kind, [this.detail = '']);

  final String kind;
  final String detail;

  @override
  String toString() => 'CoreError($kind: $detail)';
}

class Core {
  final _events = StreamController<Map<String, dynamic>>.broadcast();
  final _waiting = <int, Completer<dynamic>>{};
  NativeCallable<MessageCallback>? _callback;

  /// On a Mac the core outlives the interface, so an answer meant for an
  /// interface that was closed and opened again can still arrive. Numbers
  /// therefore never start over within one run of the app.
  int _next = DateTime.now().microsecondsSinceEpoch;

  Stream<Map<String, dynamic>> get events => _events.stream;

  void start({
    required String stateDir,
    required String defaultFolder,
    Uint8List? keySecret,
  }) {
    final callback = NativeCallable<MessageCallback>.listener(_onMessage);
    _callback = callback;
    final status = using((arena) {
      final length = keySecret?.length ?? 0;
      final secret = length == 0 ? nullptr.cast<Uint8>() : arena<Uint8>(length);
      if (keySecret != null) secret.asTypedList(length).setAll(0, keySecret);
      final result = zhStart(
        stateDir.toNativeUtf8(allocator: arena),
        defaultFolder.toNativeUtf8(allocator: arena),
        secret,
        length,
        callback.nativeFunction,
      );
      if (length != 0) secret.asTypedList(length).fillRange(0, length, 0);
      return result;
    });
    if (status != 0) {
      throw CoreError('start', 'status $status');
    }
  }

  /// The secret Windows keeps for this account to seal the device key, or
  /// null if Windows will not hand it over.
  Uint8List? systemSecret(String stateDir) {
    const length = 32;
    return using((arena) {
      final out = arena<Uint8>(length);
      final status = zhSystemSecret(
        stateDir.toNativeUtf8(allocator: arena),
        out,
        length,
      );
      final view = out.asTypedList(length);
      final secret = status == 0 ? Uint8List.fromList(view) : null;
      view.fillRange(0, length, 0);
      return secret;
    });
  }

  void dispose() {
    _callback?.close();
  }

  void _onMessage(Pointer<Char> pointer) {
    final text = pointer.cast<Utf8>().toDartString();
    zhFree(pointer);
    receive(text);
  }

  /// One message from the core. On a Mac they come through the system's own
  /// code, which started the core (see `PlatformBridge.ownsCore`).
  void receive(String text) {
    final message = jsonDecode(text) as Map<String, dynamic>;
    final reply = message['reply'];
    if (reply is! int) {
      _events.add(message);
      return;
    }
    final waiting = _waiting.remove(reply);
    if (waiting == null) return;
    final error = message['error'];
    if (error is Map<String, dynamic>) {
      waiting.completeError(
        CoreError(error['kind'] as String, error['detail'] as String? ?? ''),
      );
    } else {
      waiting.complete(message['ok']);
    }
  }

  Future<dynamic> call(String method, [Map<String, dynamic>? params]) {
    final id = _next++;
    final completer = Completer<dynamic>();
    _waiting[id] = completer;
    final text = jsonEncode({'id': id, 'method': method, 'params': ?params});
    using((arena) {
      final native = text.toNativeUtf8(allocator: arena);
      zhRequest(native);
      // A request may carry copied content.
      final length = native.length;
      native.cast<Uint8>().asTypedList(length).fillRange(0, length, 0);
    });
    return completer.future;
  }
}
