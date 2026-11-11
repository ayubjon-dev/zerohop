#include "flutter_window.h"

#include <flutter/standard_method_codec.h>
#include <flutter_windows.h>
#include <shellapi.h>

#include <optional>
#include <utility>

#include "flutter/generated_plugin_registrant.h"
#include "owner_check.h"
#include "shared_paths.h"
#include "utils.h"

using flutter::EncodableMap;
using flutter::EncodableValue;

namespace {

// The most files from Send to that wait for the interface at once.
constexpr size_t kMaxWaitingPaths = 1000;

bool Flag(const EncodableMap* arguments, const char* key) {
  if (arguments == nullptr) {
    return false;
  }
  const auto found = arguments->find(EncodableValue(key));
  if (found == arguments->end()) {
    return false;
  }
  const bool* value = std::get_if<bool>(&found->second);
  return value != nullptr && *value;
}

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
                             LaunchOptions options)
    : project_(project), options_(std::move(options)) {}

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

  background_ = std::make_unique<BackgroundMode>(
      GetHandle(), options_.title, options_.profile.empty());
  clipboard_ = std::make_unique<SharedClipboard>(
      GetHandle(), [this](const std::string& method, EncodableMap arguments) {
        Report(method, std::move(arguments));
      });
  waiting_paths_ = options_.paths;
  // Started by Windows at sign-in: the icon is there at once, so the app can
  // be reached even before the interface has said anything.
  if (options_.background) {
    background_->Set(true);
  }

  platform_channel_ = std::make_unique<flutter::MethodChannel<EncodableValue>>(
      flutter_controller_->engine()->messenger(), "dev.zerohop/platform",
      &flutter::StandardMethodCodec::GetInstance());
  platform_channel_->SetMethodCallHandler(
      [this](const flutter::MethodCall<EncodableValue>& call,
             std::unique_ptr<flutter::MethodResult<EncodableValue>> result) {
        HandleCall(call, std::move(result));
      });

  SetChildContent(flutter_controller_->view()->GetNativeWindow());

  flutter_controller_->engine()->SetNextFrameCallback([this]() {
    if (!options_.background) {
      this->Show();
    }
  });

  // Flutter can complete the first frame before the "show window" callback is
  // registered. The following call ensures a frame is pending to ensure the
  // window is shown. It is a no-op if the first frame hasn't completed yet.
  flutter_controller_->ForceRedraw();

  return true;
}

void FlutterWindow::OnDestroy() {
  // What arrived on the clipboard goes with the app, and so does the icon.
  // Starting at sign-in stays as the owner set it.
  clipboard_ = nullptr;
  background_ = nullptr;
  owner_result_ = nullptr;
  // The channel talks through the engine, so it goes first.
  platform_channel_ = nullptr;
  if (flutter_controller_) {
    flutter_controller_ = nullptr;
  }

  Win32Window::OnDestroy();
}

void FlutterWindow::HandleCall(
    const flutter::MethodCall<EncodableValue>& call,
    std::unique_ptr<flutter::MethodResult<EncodableValue>> result) {
  const std::string& method = call.method_name();
  const auto* arguments = std::get_if<EncodableMap>(call.arguments());
  if (method == "activate") {
    BringForward(true);
    result->Success();
  } else if (method == "background") {
    background_->Set(Flag(arguments, "on"));
    // An app started out of sight must not go on running unseen.
    if (!background_->is_on() && !::IsWindowVisible(GetHandle())) {
      BringForward(false);
    }
    result->Success();
  } else if (method == "clipboardWatch") {
    clipboard_->Watch(Flag(arguments, "on"));
    // Windows does not ask the owner before an app reads the clipboard.
    result->Success(EncodableValue("allow"));
  } else if (method == "clipboardRead") {
    result->Success(EncodableValue(clipboard_->Read()));
  } else if (method == "clipboardApply") {
    if (arguments != nullptr) {
      clipboard_->Apply(*arguments);
    }
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
    interface_ready_ = true;
    DeliverPaths();
    result->Success();
  } else {
    result->NotImplemented();
  }
}

void FlutterWindow::Report(const std::string& method, EncodableMap arguments) {
  if (platform_channel_) {
    platform_channel_->InvokeMethod(
        method, std::make_unique<EncodableValue>(std::move(arguments)));
  }
}

void FlutterWindow::DeliverPaths() {
  if (!interface_ready_ || waiting_paths_.empty()) {
    return;
  }
  flutter::EncodableList paths;
  for (const std::wstring& path : waiting_paths_) {
    std::string utf8 = Utf8FromUtf16(path.c_str());
    if (!utf8.empty()) {
      paths.emplace_back(std::move(utf8));
    }
  }
  waiting_paths_.clear();
  if (paths.empty()) {
    return;
  }
  EncodableMap arguments;
  arguments[EncodableValue("paths")] = EncodableValue(std::move(paths));
  Report("sharedFiles", std::move(arguments));
}

void FlutterWindow::BringForward(bool someone_waiting) {
  HWND window = GetHandle();
  if (window == nullptr) {
    return;
  }
  ::ShowWindow(window, ::IsIconic(window) ? SW_RESTORE : SW_SHOW);
  if (::SetForegroundWindow(window)) {
    return;
  }
  FLASHWINFO flash = {};
  flash.cbSize = sizeof(flash);
  flash.hwnd = window;
  flash.dwFlags = FLASHW_ALL | FLASHW_TIMERNOFG;
  ::FlashWindowEx(&flash);
  if (someone_waiting && background_) {
    background_->NotifyOffer();
  }
}

void FlutterWindow::Quit() {
  quitting_ = true;
  ::PostMessageW(GetHandle(), WM_CLOSE, 0, 0);
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
  if (clipboard_ && clipboard_->HandleMessage(message, wparam)) {
    return 0;
  }
  if (background_) {
    if (const std::optional<BackgroundMode::Action> action =
            background_->HandleMessage(message, wparam, lparam)) {
      if (*action == BackgroundMode::Action::kOpen) {
        BringForward(false);
      } else if (*action == BackgroundMode::Action::kQuit) {
        Quit();
      }
      return 0;
    }
  }

  switch (message) {
    case WM_FONTCHANGE:
      flutter_controller_->engine()->ReloadSystemFonts();
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
      // With the shared clipboard on, closing the window leaves the app
      // working next to the clock, as on the Mac.
      if (!quitting_ && background_ && background_->is_on()) {
        ::ShowWindow(hwnd, SW_HIDE);
        return 0;
      }
      break;
    case WM_COPYDATA: {
      std::vector<std::wstring> paths =
          ReadPathsMessage(reinterpret_cast<const COPYDATASTRUCT*>(lparam));
      if (paths.empty()) {
        return FALSE;
      }
      for (std::wstring& path : paths) {
        if (waiting_paths_.size() >= kMaxWaitingPaths) {
          break;
        }
        waiting_paths_.push_back(std::move(path));
      }
      DeliverPaths();
      BringForward(false);
      return TRUE;
    }
    case WM_ENDSESSION:
      // Windows is shutting down and may not send anything more.
      if (wparam != FALSE && clipboard_) {
        clipboard_->ClearIfStillOurs();
      }
      return 0;
  }

  return Win32Window::MessageHandler(hwnd, message, wparam, lparam);
}
