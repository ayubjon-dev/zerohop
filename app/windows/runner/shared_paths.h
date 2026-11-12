#ifndef RUNNER_SHARED_PATHS_H_
#define RUNNER_SHARED_PATHS_H_

#include <windows.h>

#include <string>
#include <vector>

// Files handed to Zerohop from Explorer's Send to menu or a command line. They
// are only added to the list: nothing leaves until the owner presses Send.

struct CommandLine {
  // Started by Windows at sign-in.
  bool background = false;
  // Files and folders that exist, as full paths.
  std::vector<std::wstring> paths;
};

CommandLine ReadCommandLine();

// Hands paths to the copy that is already running. True when it took them.
bool SendPathsTo(HWND running, const std::vector<std::wstring>& paths);

// The paths in a WM_COPYDATA message from SendPathsTo. Empty for any other
// message, and only paths that exist are kept.
std::vector<std::wstring> ReadPathsMessage(const COPYDATASTRUCT* message);

// Adds Zerohop to Explorer's Send to menu, or points the entry at this copy
// when the app has moved.
void AddSendToEntry();

// Sent by the installer and uninstaller to make the running copy quit, even
// when closing its window would only hide it next to the clock.
UINT QuitMessage();

#endif  // RUNNER_SHARED_PATHS_H_
