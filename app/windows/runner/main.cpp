#include <flutter/dart_project.h>
#include <windows.h>

#include <string>
#include <utility>
#include <vector>

#include "app_host.h"
#include "shared_paths.h"
#include "utils.h"
#include "win32_window.h"

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

// Asks the copy that is already running to open its window, with any files
// from Send to.
void HandOverToRunningCopy(const std::wstring& title,
                           const std::vector<std::wstring>& paths) {
  HWND running = ::FindWindowW(kZerohopWindowClass, title.c_str());
  if (running == nullptr) {
    return;
  }
  // The owner started this copy, so the running one may come forward.
  DWORD process = 0;
  ::GetWindowThreadProcessId(running, &process);
  ::AllowSetForegroundWindow(process);
  if (paths.empty() || !SendPathsTo(running, paths)) {
    ::PostMessageW(running, OpenMessage(), 0, 0);
  }
}

}  // namespace

int APIENTRY wWinMain(_In_ HINSTANCE instance, _In_opt_ HINSTANCE prev,
                      _In_ wchar_t *command_line, _In_ int show_command) {
  const std::wstring profile = ProfileName();
  const std::wstring title = profile.empty()
                                 ? std::wstring(L"Zerohop")
                                 : L"Zerohop (" + profile + L")";
  CommandLine launch = ReadCommandLine();

  // One copy per profile: a second launch only brings the first forward.
  const std::wstring lock_name = L"Local\\dev.zerohop.app." + profile;
  HANDLE instance_lock = ::CreateMutexW(nullptr, FALSE, lock_name.c_str());
  if (instance_lock != nullptr && ::GetLastError() == ERROR_ALREADY_EXISTS) {
    // A start by Windows at sign-in leaves a running copy alone.
    if (!launch.background) {
      HandOverToRunningCopy(title, launch.paths);
    }
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

  LaunchOptions options;
  options.title = title;
  options.profile = profile;
  options.background = launch.background;
  options.paths = std::move(launch.paths);
  const bool started_out_of_sight = options.background;

  // Zerohop itself is a window that is never shown. The interface opens now,
  // unless Windows started Zerohop at sign-in.
  AppHost host(project, std::move(options));
  if (!host.Create()) {
    return EXIT_FAILURE;
  }
  if (!started_out_of_sight) {
    host.OpenInterface(false);
  }

  // Test profiles are left out: Send to would start the everyday copy.
  if (profile.empty()) {
    AddSendToEntry();
  }

  ::MSG msg;
  while (::GetMessage(&msg, nullptr, 0, 0)) {
    ::TranslateMessage(&msg);
    ::DispatchMessage(&msg);
  }

  ::CoUninitialize();
  return EXIT_SUCCESS;
}
