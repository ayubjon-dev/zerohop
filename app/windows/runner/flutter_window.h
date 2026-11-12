#ifndef RUNNER_FLUTTER_WINDOW_H_
#define RUNNER_FLUTTER_WINDOW_H_

#include <flutter/dart_project.h>
#include <flutter/encodable_value.h>
#include <flutter/flutter_view_controller.h>
#include <flutter/method_channel.h>

#include <memory>
#include <string>

#include "win32_window.h"

class AppHost;

// The window with the interface, and with it all of Flutter: it exists only
// while it is open. It asks the rest of Zerohop for what Windows does, and
// hears the core through it.
class FlutterWindow : public Win32Window {
 public:
  FlutterWindow(const flutter::DartProject& project, AppHost& host);
  virtual ~FlutterWindow();

  // A message from the core, as the core wrote it.
  void Deliver(const std::string& message);

  // Something the rest of Zerohop reports to the interface.
  void Tell(const std::string& method, flutter::EncodableMap arguments);

  // Brings the window forward, or flashes it when Windows will not allow
  // that. False when it could not come forward.
  bool BringForward();

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

  // The project to run.
  flutter::DartProject project_;

  AppHost& host_;

  // The Flutter instance hosted by this window.
  std::unique_ptr<flutter::FlutterViewController> flutter_controller_;

  // The calls between the interface and Windows itself.
  std::unique_ptr<flutter::MethodChannel<flutter::EncodableValue>>
      platform_channel_;

  // Waits for Windows Hello to answer, one check at a time.
  std::unique_ptr<flutter::MethodResult<flutter::EncodableValue>>
      owner_result_;
};

#endif  // RUNNER_FLUTTER_WINDOW_H_
