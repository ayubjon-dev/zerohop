#include <flutter/dart_project.h>
#include <flutter/flutter_view_controller.h>
#include <windows.h>

#include <string>

#include "flutter_window.h"
#include "utils.h"

namespace {

// A test profile lets several copies run side by side, each with its own state.
std::wstring ProfileName() {
  wchar_t buffer[64];
  const DWORD length =
      ::GetEnvironmentVariableW(L"ZEROHOP_PROFILE", buffer, 64);
  if (length == 0 || length >= 64) {
    return L"";
  }
  std::wstring name(buffer, length);
  for (wchar_t& c : name) {
    if (c == L'\\') {
      c = L'_';
    }
  }
  return name;
}

// Shows the copy that is already running.
void ShowRunningCopy(const std::wstring& title) {
  HWND running = ::FindWindowW(kZerohopWindowClass, title.c_str());
  if (running == nullptr) {
    return;
  }
  ::ShowWindow(running, ::IsIconic(running) ? SW_RESTORE : SW_SHOW);
  ::SetForegroundWindow(running);
}

// Centres the window on the work area of its screen, and keeps it no larger.
void FitToWorkArea(HWND window) {
  RECT frame;
  MONITORINFO monitor = {};
  monitor.cbSize = sizeof(monitor);
  if (!::GetWindowRect(window, &frame) ||
      !::GetMonitorInfoW(
          ::MonitorFromWindow(window, MONITOR_DEFAULTTOPRIMARY), &monitor)) {
    return;
  }
  const RECT& work = monitor.rcWork;
  const int work_width = static_cast<int>(work.right - work.left);
  const int work_height = static_cast<int>(work.bottom - work.top);
  int width = static_cast<int>(frame.right - frame.left);
  int height = static_cast<int>(frame.bottom - frame.top);
  if (width > work_width) {
    width = work_width;
  }
  if (height > work_height) {
    height = work_height;
  }
  ::SetWindowPos(window, nullptr,
                 static_cast<int>(work.left) + (work_width - width) / 2,
                 static_cast<int>(work.top) + (work_height - height) / 2,
                 width, height, SWP_NOZORDER | SWP_NOACTIVATE);
}

}  // namespace

int APIENTRY wWinMain(_In_ HINSTANCE instance, _In_opt_ HINSTANCE prev,
                      _In_ wchar_t *command_line, _In_ int show_command) {
  const std::wstring profile = ProfileName();
  const std::wstring title = profile.empty()
                                 ? std::wstring(L"Zerohop")
                                 : L"Zerohop (" + profile + L")";

  // One copy per profile: a second launch only brings the first forward.
  const std::wstring lock_name = L"Local\\dev.zerohop.app." + profile;
  HANDLE instance_lock = ::CreateMutexW(nullptr, FALSE, lock_name.c_str());
  if (instance_lock != nullptr && ::GetLastError() == ERROR_ALREADY_EXISTS) {
    ShowRunningCopy(title);
    ::CloseHandle(instance_lock);
    return EXIT_SUCCESS;
  }

  // Attach to console when present (e.g., 'flutter run') or create a
  // new console when running with a debugger.
  if (!::AttachConsole(ATTACH_PARENT_PROCESS) && ::IsDebuggerPresent()) {
    CreateAndAttachConsole();
  }

  // Initialize COM, so that it is available for use in the library and/or
  // plugins.
  ::CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);

  flutter::DartProject project(L"data");

  std::vector<std::string> command_line_arguments =
      GetCommandLineArguments();

  project.set_dart_entrypoint_arguments(std::move(command_line_arguments));

  // The same size as the Mac window.
  FlutterWindow window(project);
  Win32Window::Point origin(10, 10);
  Win32Window::Size size(1120, 760);
  if (!window.Create(title, origin, size)) {
    return EXIT_FAILURE;
  }
  FitToWorkArea(window.GetHandle());
  window.SetQuitOnClose(true);

  ::MSG msg;
  while (::GetMessage(&msg, nullptr, 0, 0)) {
    ::TranslateMessage(&msg);
    ::DispatchMessage(&msg);
  }

  ::CoUninitialize();
  return EXIT_SUCCESS;
}
