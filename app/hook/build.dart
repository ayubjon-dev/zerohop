// Builds the Rust core and hands it to Flutter as a native library.

import 'dart:io';

import 'package:code_assets/code_assets.dart';
import 'package:hooks/hooks.dart';

String _rustTarget(CodeConfig code) {
  final arch = code.targetArchitecture;
  switch (code.targetOS) {
    case OS.macOS:
      return arch == Architecture.arm64
          ? 'aarch64-apple-darwin'
          : 'x86_64-apple-darwin';
    case OS.windows:
      return arch == Architecture.arm64
          ? 'aarch64-pc-windows-msvc'
          : 'x86_64-pc-windows-msvc';
    case OS.linux:
      return arch == Architecture.arm64
          ? 'aarch64-unknown-linux-gnu'
          : 'x86_64-unknown-linux-gnu';
    case OS.android:
      if (arch == Architecture.arm64) return 'aarch64-linux-android';
      if (arch == Architecture.arm) return 'armv7-linux-androideabi';
      return 'x86_64-linux-android';
    case OS.iOS:
      final simulator = code.iOS.targetSdk == IOSSdk.iPhoneSimulator;
      if (arch == Architecture.x64) return 'x86_64-apple-ios';
      return simulator ? 'aarch64-apple-ios-sim' : 'aarch64-apple-ios';
  }
  throw UnsupportedError('No Rust target for ${code.targetOS}');
}

String _cargo() {
  final home =
      Platform.environment['HOME'] ?? Platform.environment['USERPROFILE'] ?? '';
  final installed = File(
    '$home/.cargo/bin/cargo${Platform.isWindows ? '.exe' : ''}',
  );
  return installed.existsSync() ? installed.path : 'cargo';
}

/// Points Cargo and the C builds at the NDK Flutter chose.
void _configureAndroid(
  CodeConfig code,
  String target,
  Map<String, String> environment,
) {
  final bin = code.cCompiler!.compiler.resolve('.');
  final ndk = bin.resolve('../../../../../');
  final api = code.android.targetNdkApi;
  final clangTarget = target.replaceFirst('armv7-', 'armv7a-');
  final clang = bin.resolve('$clangTarget$api-clang').toFilePath();
  final clangxx = bin.resolve('$clangTarget$api-clang++').toFilePath();
  final archiver = bin.resolve('llvm-ar').toFilePath();
  final envTarget = target.toUpperCase().replaceAll('-', '_');
  environment.addAll({
    'ANDROID_NDK_ROOT': ndk.toFilePath(),
    'ANDROID_NDK_HOME': ndk.toFilePath(),
    'ANDROID_NDK': ndk.toFilePath(),
    'CC_$target': clang,
    'CXX_$target': clangxx,
    'AR_$target': archiver,
    'CARGO_TARGET_${envTarget}_LINKER': clang,
    'CARGO_TARGET_${envTarget}_AR': archiver,
    // Android 15 devices may use 16 KB memory pages.
    'CARGO_TARGET_${envTarget}_RUSTFLAGS':
        '-C link-arg=-Wl,-z,max-page-size=16384',
  });
}

/// Hooks run with a reduced environment, in which the compiler Flutter puts
/// first on the path cannot find the system headers by itself.
Future<String> _appleSdkPath(String sdk) async {
  final result = await Process.run('/usr/bin/xcrun', [
    '--sdk',
    sdk,
    '--show-sdk-path',
  ]);
  if (result.exitCode != 0) {
    throw StateError('xcrun could not find the $sdk SDK:\n${result.stderr}');
  }
  return (result.stdout as String).trim();
}

void _collectSources(Directory directory, List<Uri> found) {
  for (final entity in directory.listSync(recursive: true)) {
    if (entity is File &&
        (entity.path.endsWith('.rs') || entity.path.endsWith('Cargo.toml'))) {
      found.add(entity.uri);
    }
  }
}

void main(List<String> args) async {
  await build(args, (input, output) async {
    if (!input.config.buildCodeAssets) return;
    final code = input.config.code;
    final target = _rustTarget(code);
    final repository = input.packageRoot.resolve('../');
    final targetDir = input.outputDirectoryShared.resolve('cargo/');

    final environment = <String, String>{};
    if (code.targetOS == OS.macOS) {
      environment['MACOSX_DEPLOYMENT_TARGET'] =
          '${code.macOS.deploymentVersion.major}';
      environment['SDKROOT'] = await _appleSdkPath('macosx');
    } else if (code.targetOS == OS.iOS) {
      final simulator = code.iOS.targetSdk == IOSSdk.iPhoneSimulator;
      // Without it Rust links for iOS 10 while C code is built for the SDK,
      // and the device build misses symbols that iOS 10 did not have.
      final version = code.iOS.deploymentVersion;
      environment['IPHONEOS_DEPLOYMENT_TARGET'] =
          '${version.major}.${version.minor}';
      environment['SDKROOT'] = await _appleSdkPath(
        simulator ? 'iphonesimulator' : 'iphoneos',
      );
    }

    if (code.targetOS == OS.android) {
      _configureAndroid(code, target, environment);
    }

    final result = await Process.run(
      _cargo(),
      [
        'build',
        '--release',
        '--package',
        'zerohop-ffi',
        '--target',
        target,
        '--target-dir',
        targetDir.toFilePath(),
      ],
      workingDirectory: repository.toFilePath(),
      environment: environment,
    );
    if (result.exitCode != 0) {
      throw StateError('cargo build failed:\n${result.stderr}');
    }

    final library = targetDir.resolve(
      '$target/release/${code.targetOS.dylibFileName('zerohop_ffi')}',
    );
    output.assets.code.add(
      CodeAsset(
        package: input.packageName,
        name: 'src/bridge/native.dart',
        linkMode: DynamicLoadingBundled(),
        file: library,
      ),
    );

    final sources = <Uri>[repository.resolve('Cargo.lock')];
    _collectSources(Directory.fromUri(repository.resolve('crates/')), sources);
    output.dependencies.addAll(sources);
  });
}
