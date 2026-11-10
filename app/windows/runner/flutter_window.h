#ifndef RUNNER_FLUTTER_WINDOW_H_
#define RUNNER_FLUTTER_WINDOW_H_

#include <flutter/dart_project.h>
#include <flutter/encodable_value.h>
#include <flutter/flutter_view_controller.h>
#include <flutter/method_channel.h>

#include <memory>
#include <string>
#include <vector>

#include "background_mode.h"
#include "shared_clipboard.h"
#include "win32_window.h"

// How this copy of the app was started.
struct LaunchOptions {
  std::wstring title;
  // Empty for the owner's everyday copy, set for test copies side by side.
  std::wstring profile;
  // Started by Windows at sign-in: stay next to the clock.
  bool background = false;
  // Files handed over from Explorer's Send to menu.
  std::vector<std::wstring> paths;
};

// A window that hosts the Flutter view and the parts of Windows that Flutter
// does not reach by itself.
class FlutterWindow : public Win32Window {
 public:
  FlutterWindow(const flutter::DartProject& project, LaunchOptions options);
  virtual ~FlutterWindow();

 protected:
  // Win32Window:
  bool OnCreate() override;
  void OnDestroy() override;
  LRESULT MessageHandler(HWND window, UINT const message, WPARAM const wparam,
                         LPARAM const lparam) noexcept override;

 private:
  void HandleCall(
      const flutter::MethodCall<flutter::EncodableValue>& call,
      std::unique_ptr<flutter::MethodResult<flutter::EncodableValue>> result);

  // Sends a call to the interface.
  void Report(const std::string& method, flutter::EncodableMap arguments);

  // Hands files from Send to over once the interface can take them.
  void DeliverPaths();

  // Brings the window forward, or flashes it when Windows will not allow that.
  // |someone_waiting| also shows a note next to the clock in that case.
  void BringForward(bool someone_waiting);

  // Closes the window for good rather than hiding it.
  void Quit();

  // The project to run.
  flutter::DartProject project_;

  LaunchOptions options_;

  // The Flutter instance hosted by this window.
  std::unique_ptr<flutter::FlutterViewController> flutter_controller_;

  // The calls between the interface and Windows itself.
  std::unique_ptr<flutter::MethodChannel<flutter::EncodableValue>>
      platform_channel_;

  std::unique_ptr<BackgroundMode> background_;
  std::unique_ptr<SharedClipboard> clipboard_;
  std::vector<std::wstring> waiting_paths_;
  bool interface_ready_ = false;
  bool quitting_ = false;
};

#endif  // RUNNER_FLUTTER_WINDOW_H_
