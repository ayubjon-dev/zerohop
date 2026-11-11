#include "shared_paths.h"

// Defines the folder and class identifiers here rather than relying on a
// library to provide them.
#include <initguid.h>

#include <knownfolders.h>
#include <objbase.h>
#include <shellapi.h>
#include <shlobj.h>
#include <shobjidl.h>

#include <utility>

#include "utils.h"

namespace {

// Marks a WM_COPYDATA message as paths from another copy of Zerohop.
constexpr ULONG_PTR kPathsTag = 0x5A480001;
constexpr size_t kMaxPaths = 1000;
constexpr size_t kLongestPath = 32767;
constexpr size_t kMaxMessageBytes = 8 * 1024 * 1024;
constexpr DWORD kSendTimeoutMs = 5000;

// The full path of a file or folder that exists, or an empty string.
std::wstring ExistingFullPath(const std::wstring& path) {
  if (path.empty() || path.size() > kLongestPath) {
    return std::wstring();
  }
  const DWORD needed = ::GetFullPathNameW(path.c_str(), 0, nullptr, nullptr);
  if (needed == 0 || needed > kLongestPath + 1) {
    return std::wstring();
  }
  std::wstring full(needed, L'\0');
  const DWORD written =
      ::GetFullPathNameW(path.c_str(), needed, full.data(), nullptr);
  if (written == 0 || written >= needed) {
    return std::wstring();
  }
  full.resize(written);
  if (::GetFileAttributesW(full.c_str()) == INVALID_FILE_ATTRIBUTES) {
    return std::wstring();
  }
  return full;
}

}  // namespace

CommandLine ReadCommandLine() {
  CommandLine result;
  int count = 0;
  wchar_t** arguments = ::CommandLineToArgvW(::GetCommandLineW(), &count);
  if (arguments == nullptr) {
    return result;
  }
  for (int i = 1; i < count; ++i) {
    const std::wstring argument(arguments[i]);
    if (argument == L"--background") {
      result.background = true;
    } else if (argument.rfind(L"--", 0) != 0 &&
               result.paths.size() < kMaxPaths) {
      std::wstring path = ExistingFullPath(argument);
      if (!path.empty()) {
        result.paths.push_back(std::move(path));
      }
    }
  }
  ::LocalFree(arguments);
  return result;
}

bool SendPathsTo(HWND running, const std::vector<std::wstring>& paths) {
  std::wstring joined;
  for (const std::wstring& path : paths) {
    joined += path;
    joined.push_back(L'\0');
  }
  const size_t bytes = joined.size() * sizeof(wchar_t);
  if (bytes == 0 || bytes > kMaxMessageBytes) {
    return false;
  }
  COPYDATASTRUCT message = {};
  message.dwData = kPathsTag;
  message.cbData = static_cast<DWORD>(bytes);
  message.lpData = joined.data();
  DWORD_PTR answer = 0;
  return ::SendMessageTimeoutW(running, WM_COPYDATA, 0,
                               reinterpret_cast<LPARAM>(&message),
                               SMTO_ABORTIFHUNG, kSendTimeoutMs,
                               &answer) != 0 &&
         answer != 0;
}

std::vector<std::wstring> ReadPathsMessage(const COPYDATASTRUCT* message) {
  std::vector<std::wstring> paths;
  if (message == nullptr || message->dwData != kPathsTag ||
      message->lpData == nullptr || message->cbData == 0 ||
      message->cbData > kMaxMessageBytes ||
      message->cbData % sizeof(wchar_t) != 0) {
    return paths;
  }
  const auto* text = static_cast<const wchar_t*>(message->lpData);
  const size_t length = message->cbData / sizeof(wchar_t);
  size_t start = 0;
  for (size_t i = 0; i < length && paths.size() < kMaxPaths; ++i) {
    if (text[i] != L'\0') {
      continue;
    }
    std::wstring path = ExistingFullPath(std::wstring(text + start, i - start));
    if (!path.empty()) {
      paths.push_back(std::move(path));
    }
    start = i + 1;
  }
  return paths;
}

void AddSendToEntry() {
  const std::wstring target = ExecutablePath();
  if (target.empty()) {
    return;
  }
  PWSTR folder = nullptr;
  std::wstring entry;
  if (SUCCEEDED(::SHGetKnownFolderPath(FOLDERID_SendTo, KF_FLAG_DEFAULT,
                                       nullptr, &folder)) &&
      folder != nullptr) {
    entry = std::wstring(folder) + L"\\Zerohop.lnk";
  }
  ::CoTaskMemFree(folder);
  if (entry.empty()) {
    return;
  }

  IShellLinkW* link = nullptr;
  if (FAILED(::CoCreateInstance(CLSID_ShellLink, nullptr, CLSCTX_INPROC_SERVER,
                                IID_PPV_ARGS(&link)))) {
    return;
  }
  IPersistFile* file = nullptr;
  if (SUCCEEDED(link->QueryInterface(IID_PPV_ARGS(&file)))) {
    std::wstring current(kLongestPath + 1, L'\0');
    const bool found =
        SUCCEEDED(file->Load(entry.c_str(), STGM_READ)) &&
        SUCCEEDED(link->GetPath(current.data(),
                                static_cast<int>(current.size()), nullptr,
                                SLGP_RAWPATH));
    if (!found || ::CompareStringOrdinal(current.c_str(), -1, target.c_str(),
                                         -1, TRUE) != CSTR_EQUAL) {
      link->SetPath(target.c_str());
      link->SetArguments(L"");
      link->SetIconLocation(target.c_str(), 0);
      link->SetDescription(L"Zerohop");
      file->Save(entry.c_str(), TRUE);
    }
    file->Release();
  }
  link->Release();
}

UINT QuitMessage() {
  static const UINT message =
      ::RegisterWindowMessageW(L"dev.zerohop.app.quit");
  return message;
}
