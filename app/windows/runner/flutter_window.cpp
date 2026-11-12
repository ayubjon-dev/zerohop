#include "flutter_window.h"

#include <flutter/standard_method_codec.h>
#include <flutter_windows.h>
#include <shellapi.h>

#include <optional>
#include <utility>

#include "app_host.h"
#include "flutter/generated_plugin_registrant.h"
#include "owner_check.h"

using flutter::EncodableMap;
using flutter::EncodableValue;

namespace {

// Where the core's messages reach the interface, as the core wrote them.
constexpr char kCoreChannel[] = "dev.zerohop/core";

std::string Text(const EncodableMap* arguments, const char* key) {
  if (arguments == nullptr) {
    return std::string();
  }
  const auto found = arguments->find(EncodableValue(key));
  if (found == arguments->end()) {
    return std::string();
  }
  const std::string* value = std::get_if<std::string>(&found->second);
  return value == nullptr ? std::string() : *value;
}

}  // namespace

FlutterWindow::FlutterWindow(const flutter::DartProject& project,
                             AppHost& host)
    : project_(project), host_(host) {}

FlutterWindow::~FlutterWindow() {}

bool FlutterWindow::OnCreate() {
  if (!Win32Window::OnCreate()) {
    return false;
  }

  RECT frame = GetClientArea();

  // The size here must match the window dimensions to avoid unnecessary surface
  // creation / destruction in the startup path.
  flutter_controller_ = std::make_unique<flutter::FlutterViewController>(
      frame.right - frame.left, frame.bottom - frame.top, project_);
  // Ensure that basic setup of the controller was successful.
  if (!flutter_controller_->engine() || !flutter_controller_->view()) {
    return false;
  }
  RegisterPlugins(flutter_controller_->engine());

  platform_channel_ = std::make_unique<flutter::MethodChannel<EncodableValue>>(
      flutter_controller_->engine()->messenger(), "dev.zerohop/platform",
      &flutter::StandardMethodCodec::GetInstance());
  platform_channel_->SetMethodCallHandler(
      [this](const flutter::MethodCall<EncodableValue>& call,
             std::unique_ptr<flutter::MethodResult<EncodableValue>> result) {
        HandleCall(call, std::move(result));
      });

  SetChildContent(flutter_controller_->view()->GetNativeWindow());

  flutter_controller_->engine()->SetNextFrameCallback(
      [this]() { host_.InterfaceDrawn(); });

  // Flutter can complete the first frame before the "show window" callback is
  // registered. The following call ensures a frame is pending to ensure the
  // window is shown. It is a no-op if the first frame hasn't completed yet.
  flutter_controller_->ForceRedraw();

  return true;
}

void FlutterWindow::OnDestroy() {
  owner_result_ = nullptr;
  // The channel talks through the engine, so it goes first.
  platform_channel_ = nullptr;
  if (flutter_controller_) {
    flutter_controller_ = nullptr;
  }

  Win32Window::OnDestroy();
}

void FlutterWindow::Deliver(const std::string& message) {
  if (!flutter_controller_ || message.empty()) {
    return;
  }
  flutter_controller_->engine()->messenger()->Send(
      kCoreChannel, reinterpret_cast<const uint8_t*>(message.data()),
      message.size());
}

void FlutterWindow::Tell(const std::string& method, EncodableMap arguments) {
  if (platform_channel_) {
    platform_channel_->InvokeMethod(
        method, std::make_unique<EncodableValue>(std::move(arguments)));
  }
}

void FlutterWindow::HandleCall(
    const flutter::MethodCall<EncodableValue>& call,
    std::unique_ptr<flutter::MethodResult<EncodableValue>> result) {
  const std::string& method = call.method_name();
  const auto* arguments = std::get_if<EncodableMap>(call.arguments());
  if (method == "attach") {
    std::shared_ptr<flutter::MethodResult<EncodableValue>> waiting =
        std::move(result);
    host_.Attach([waiting](EncodableMap answer) {
      waiting->Success(EncodableValue(std::move(answer)));
    });
  } else if (method == "activate") {
    BringForward();
    result->Success();
  } else if (method == "clipboardSettings") {
    ::ShellExecuteW(nullptr, L"open", L"ms-settings:clipboard", nullptr,
                    nullptr, SW_SHOWNORMAL);
    result->Success();
  } else if (method == "confirmOwner") {
    if (owner_result_) {
      result->Error("unavailable");
    } else if (owner_check::Start(GetHandle(), Text(arguments, "reason"))) {
      owner_result_ = std::move(result);
    } else {
      // A check that cannot even start must not let a removal through.
      result->Success(EncodableValue("refused"));
    }
  } else if (method == "clipboardReady") {
    host_.InterfaceReady();
    result->Success();
  } else {
    result->NotImplemented();
  }
}

bool FlutterWindow::BringForward() {
  HWND window = GetHandle();
  if (window == nullptr) {
    return false;
  }
  ::ShowWindow(window, ::IsIconic(window) ? SW_RESTORE : SW_SHOW);
  if (::SetForegroundWindow(window)) {
    return true;
  }
  FLASHWINFO flash = {};
  flash.cbSize = sizeof(flash);
  flash.hwnd = window;
  flash.dwFlags = FLASHW_ALL | FLASHW_TIMERNOFG;
  ::FlashWindowEx(&flash);
  return false;
}

LRESULT
FlutterWindow::MessageHandler(HWND hwnd, UINT const message,
                              WPARAM const wparam,
                              LPARAM const lparam) noexcept {
  // Give Flutter, including plugins, an opportunity to handle window messages.
  if (flutter_controller_) {
    std::optional<LRESULT> result =
        flutter_controller_->HandleTopLevelWindowProc(hwnd, message, wparam,
                                                      lparam);
    if (result) {
      return *result;
    }
  }

  if (message == owner_check::kAnswerMessage) {
    if (owner_result_) {
      owner_result_->Success(EncodableValue(owner_check::AnswerName(wparam)));
      owner_result_ = nullptr;
    }
    return 0;
  }

  switch (message) {
    case WM_FONTCHANGE:
      if (flutter_controller_) {
        flutter_controller_->engine()->ReloadSystemFonts();
      }
      break;
    case WM_GETMINMAXINFO: {
      // The same smallest window as on a Mac, in this screen's pixels.
      const int dpi = static_cast<int>(FlutterDesktopGetDpiForHWND(hwnd));
      auto* limits = reinterpret_cast<MINMAXINFO*>(lparam);
      limits->ptMinTrackSize.x = ::MulDiv(380, dpi, 96);
      limits->ptMinTrackSize.y = ::MulDiv(620, dpi, 96);
      return 0;
    }
    case WM_CLOSE:
      // The rest of Zerohop decides whether it stays next to the clock.
      host_.InterfaceClosing();
      return 0;
  }

  return Win32Window::MessageHandler(hwnd, message, wparam, lparam);
}
