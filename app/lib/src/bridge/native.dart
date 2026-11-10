// The functions the Rust core exports.

import 'dart:ffi';

import 'package:ffi/ffi.dart';

typedef MessageCallback = Void Function(Pointer<Char>);

@Native<
  Int32 Function(
    Pointer<Utf8>,
    Pointer<Utf8>,
    Pointer<Uint8>,
    Size,
    Pointer<NativeFunction<MessageCallback>>,
  )
>(symbol: 'zh_start')
external int zhStart(
  Pointer<Utf8> stateDir,
  Pointer<Utf8> defaultFolder,
  Pointer<Uint8> secret,
  int secretLength,
  Pointer<NativeFunction<MessageCallback>> callback,
);

@Native<Int32 Function(Pointer<Utf8>, Pointer<Uint8>, Size)>(
  symbol: 'zh_system_secret',
)
external int zhSystemSecret(
  Pointer<Utf8> stateDir,
  Pointer<Uint8> out,
  int outLength,
);

@Native<Void Function(Pointer<Utf8>)>(symbol: 'zh_request')
external void zhRequest(Pointer<Utf8> request);

@Native<Void Function(Pointer<Char>)>(symbol: 'zh_free')
external void zhFree(Pointer<Char> message);
