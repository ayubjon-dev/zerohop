#include "app_host.h"

#include <shlobj.h>

#include <algorithm>
#include <cstdint>
#include <cstring>
#include <thread>
#include <utility>

#include "pictures.h"
#include "shared_paths.h"
#include "utils.h"
#include "win32_window.h"

using flutter::EncodableList;
using flutter::EncodableMap;
using flutter::EncodableValue;

namespace {

// The core has started, or could not, with a CoreHost::Started in wparam.
constexpr UINT kStartedMessage = WM_APP + 6;
// The interface's window closed: it goes once its own messages are done.
constexpr UINT kDropMessage = WM_APP + 7;

// The most files from Send to that wait for the interface at once.
constexpr size_t kMaxWaitingPaths = 1000;

constexpr unsigned char kPngSignature[] = {0x89, 'P',  'N',  'G',
                                           '\r', '\n', 0x1A, '\n'};

template <typename T>
const T* Find(const EncodableMap& map, const char* key) {
  const auto found = map.find(EncodableValue(key));
  if (found == map.end()) {
    return nullptr;
  }
  return std::get_if<T>(&found->second);
}

void Wipe(std::string& text) {
  if (!text.empty()) {
    ::SecureZeroMemory(text.data(), text.size());
  }
}

void Wipe(std::vector<uint8_t>& bytes) {
  if (!bytes.empty()) {
    ::SecureZeroMemory(bytes.data(), bytes.size());
  }
}

int64_t Lifetime(uint64_t ttl_ms) {
  return static_cast<int64_t>(
      std::min(ttl_ms, static_cast<uint64_t>(INT64_MAX)));
}

std::wstring KnownFolder(REFKNOWNFOLDERID id) {
  PWSTR path = nullptr;
  std::wstring found;
  if (SUCCEEDED(::SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, nullptr, &path)) &&
      path != nullptr) {
    found = path;
  }
  ::CoTaskMemFree(path);
  return found;
}

// Where this computer keeps its key and settings: the local profile, since
// the roaming one can be copied to other computers. The same place the
// interface used when it started the core itself.
std::wstring StateDir(const std::wstring& profile) {
  wchar_t local[MAX_PATH] = {};
  const DWORD length =
      ::GetEnvironmentVariableW(L"LOCALAPPDATA", local, MAX_PATH);
  std::wstring base = length > 0 && length < MAX_PATH
                          ? std::wstring(local, length)
                          : KnownFolder(FOLDERID_LocalAppData);
  if (base.empty()) {
    return std::wstring();
  }
  base += L"\\Zerohop";
  if (!profile.empty()) {
    base += L"\\profile-" + profile;
  }
  return base;
}

// Where files go until the owner picks another folder.
std::wstring DefaultFolder(const std::wstring& profile) {
  std::wstring base = KnownFolder(FOLDERID_Downloads);
  if (base.empty()) {
    base = KnownFolder(FOLDERID_Documents);
  }
  if (base.empty()) {
    return std::wstring();
  }
  return base + (profile.empty() ? L"\\Zerohop" : L"\\Zerohop-" + profile);
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
  const int width =
      std::min(static_cast<int>(frame.right - frame.left), work_width);
  const int height =
      std::min(static_cast<int>(frame.bottom - frame.top), work_height);
  ::SetWindowPos(window, nullptr,
                 static_cast<int>(work.left) + (work_width - width) / 2,
                 static_cast<int>(work.top) + (work_height - height) / 2,
                 width, height, SWP_NOZORDER | SWP_NOACTIVATE);
}

const char* StartedName(CoreHost::Started started) {
  switch (started) {
    case CoreHost::Started::kRunning:
      return "running";
    case CoreHost::Started::kKeyUnavailable:
      return "key_unavailable";
    case CoreHost::Started::kFailed:
      break;
  }
  return "failed";
}

}  // namespace

AppHost::AppHost(const flutter::DartProject& project, LaunchOptions options)
    : project_(project), options_(std::move(options)) {
  waiting_paths_ = options_.paths;
}

AppHost::~AppHost() {
  if (window_ != nullptr) {
    quitting_ = true;
    ::DestroyWindow(window_);
  }
}

bool AppHost::Create() {
  WNDCLASSW window_class = {};
  window_class.lpfnWndProc = WndProc;
  window_class.hInstance = ::GetModuleHandleW(nullptr);
  window_class.lpszClassName = kZerohopWindowClass;
  ::RegisterClassW(&window_class);
  // Never shown, and kept off the taskbar.
  window_ = ::CreateWindowExW(WS_EX_TOOLWINDOW, kZerohopWindowClass,
                              options_.title.c_str(), WS_OVERLAPPED, 0, 0, 0, 0,
                              nullptr, nullptr, ::GetModuleHandleW(nullptr),
                              this);
  if (window_ == nullptr) {
    return false;
  }
  background_ = std::make_unique<BackgroundMode>(window_, options_.title,
                                                 options_.profile.empty());
  clipboard_ = std::make_unique<SharedClipboard>(
      window_, [this](const std::string& method, EncodableMap arguments) {
        if (method == "clipboardCopied") {
          Copied(arguments);
        } else {
          Tell(method, std::move(arguments));
        }
      });
  file_paste_ = std::make_unique<FilePaste>(window_);
  bonjour_ = std::make_unique<Bonjour>(window_);
  // Started by Windows at sign-in: the icon is there at once, so the app can
  // be reached even before the core has said anything.
  if (options_.background) {
    background_->Set(true);
  }
  CoreHost::Load(window_);
  StartCore();
  return true;
}

// static
LRESULT CALLBACK AppHost::WndProc(HWND window, UINT message, WPARAM wparam,
                                  LPARAM lparam) noexcept {
  if (message == WM_NCCREATE) {
    const auto* create = reinterpret_cast<const CREATESTRUCTW*>(lparam);
    auto* host = static_cast<AppHost*>(create->lpCreateParams);
    ::SetWindowLongPtrW(window, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(host));
    // Messages come before CreateWindowEx returns.
    host->window_ = window;
  } else if (auto* host = reinterpret_cast<AppHost*>(
                 ::GetWindowLongPtrW(window, GWLP_USERDATA))) {
    return host->Handle(message, wparam, lparam);
  }
  return ::DefWindowProcW(window, message, wparam, lparam);
}

LRESULT AppHost::Handle(UINT message, WPARAM wparam, LPARAM lparam) {
  if (message == CoreHost::kMessage) {
    std::string forward = CoreHost::Take(lparam, *this);
    if (!forward.empty() && interface_ && attached_) {
      interface_->Deliver(forward);
    }
    Wipe(forward);
    return 0;
  }
  if (message == kStartedMessage) {
    Started(static_cast<CoreHost::Started>(wparam));
    return 0;
  }
  if (message == kDropMessage) {
    DropInterface();
    return 0;
  }
  if (message == Bonjour::kMessage) {
    if (bonjour_) {
      bonjour_->Handle(lparam);
    }
    return 0;
  }
  if (message == FilePaste::kFailedMessage) {
    Tell("clipboardPasteFailed", EncodableMap());
    return 0;
  }
  if (message == QuitMessage()) {
    Quit();
    return 0;
  }
  if (message == OpenMessage()) {
    OpenInterface(false);
    return 0;
  }
  if (clipboard_ && clipboard_->HandleMessage(message, wparam)) {
    return 0;
  }
  if (message == WM_TIMER && bonjour_ && bonjour_->HandleTimer(wparam)) {
    return 0;
  }
  if (background_) {
    if (const std::optional<BackgroundMode::Action> action =
            background_->HandleMessage(message, wparam, lparam)) {
      if (*action == BackgroundMode::Action::kOpen) {
        OpenInterface(false);
      } else if (*action == BackgroundMode::Action::kQuit) {
        Quit();
      }
      return 0;
    }
  }
  switch (message) {
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
      OpenInterface(false);
      DeliverPaths();
      return TRUE;
    }
    case WM_ENDSESSION:
      // Windows is shutting down and may not send anything more.
      if (wparam != FALSE && clipboard_) {
        clipboard_->ClearIfStillOurs();
      }
      // An installer replacing Zerohop closes it this way too, through the
      // Restart Manager, and expects it to go.
      if (wparam != FALSE && (lparam & ENDSESSION_CLOSEAPP) != 0) {
        Quit();
      }
      return 0;
    case WM_DESTROY:
      // What arrived on the clipboard goes with the app, and so does the
      // icon. Starting at sign-in stays as the owner set it.
      CloseInterfaceWindow();
      clipboard_ = nullptr;
      file_paste_ = nullptr;
      bonjour_ = nullptr;
      background_ = nullptr;
      ::SetWindowLongPtrW(window_, GWLP_USERDATA, 0);
      window_ = nullptr;
      ::PostQuitMessage(0);
      return 0;
  }
  return ::DefWindowProcW(window_, message, wparam, lparam);
}

// ---- The core ----

void AppHost::StartCore() {
  if (starting_ || started_ == CoreHost::Started::kRunning) {
    return;
  }
  starting_ = true;
  const std::wstring state = StateDir(options_.profile);
  const std::wstring folder = DefaultFolder(options_.profile);
  HWND window = window_;
  // Opening the key and starting to listen takes a moment, which the window
  // does not wait for.
  std::thread([window, state, folder]() {
    const CoreHost::Started result =
        state.empty() || folder.empty() ? CoreHost::Started::kFailed
                                        : CoreHost::Start(state, folder);
    ::PostMessageW(window, kStartedMessage, static_cast<WPARAM>(result), 0);
  }).detach();
}

void AppHost::Started(CoreHost::Started result) {
  starting_ = false;
  started_ = result;
  if (result == CoreHost::Started::kRunning && bonjour_) {
    bonjour_->Browse();
  }
  std::vector<Answer> waiting = std::move(attaching_);
  attaching_.clear();
  for (Answer& answer : waiting) {
    if (result == CoreHost::Started::kRunning) {
      attached_ = true;
    }
    answer(AttachAnswer());
  }
}

EncodableMap AppHost::AttachAnswer() const {
  EncodableMap answer;
  answer[EncodableValue("state")] = EncodableValue(
      StartedName(started_.value_or(CoreHost::Started::kFailed)));
  // Windows does not ask the owner before an app reads the clipboard, and
  // files copied elsewhere can always be pasted.
  answer[EncodableValue("access")] = EncodableValue("allow");
  answer[EncodableValue("files")] = EncodableValue("ready");
  return answer;
}

void AppHost::OnSnapshot(bool clipboard, bool pairing, const char* instance,
                         const char* tag, uint16_t port) {
  pairing_ = pairing;
  if (bonjour_) {
    bonjour_->Advertise(instance, tag, port);
  }
  if (clipboard_on_ == clipboard) {
    return;
  }
  clipboard_on_ = clipboard;
  if (background_) {
    background_->Set(clipboard);
  }
  if (clipboard_) {
    clipboard_->Watch(clipboard);
  }
  // An app started out of sight must not go on running unseen.
  if (!clipboard && !interface_) {
    OpenInterface(false);
  }
}

void AppHost::OnOfferArrived() { OpenInterface(true); }

void AppHost::OnClipText(const char* text, uint64_t ttl_ms, const char*) {
  if (!clipboard_ || text == nullptr) {
    return;
  }
  EncodableMap clip;
  clip[EncodableValue("format")] = EncodableValue("text");
  clip[EncodableValue("text")] = EncodableValue(std::string(text));
  clip[EncodableValue("ttl_ms")] = EncodableValue(Lifetime(ttl_ms));
  clipboard_->Apply(clip);
  if (auto* stored = std::get_if<std::string>(&clip[EncodableValue("text")])) {
    Wipe(*stored);
  }
}

void AppHost::OnClipPicture(const uint8_t* bytes, size_t size,
                            uint64_t ttl_ms, const char*) {
  if (!clipboard_ || bytes == nullptr || size == 0) {
    return;
  }
  const bool is_png = size >= sizeof(kPngSignature) &&
                      std::memcmp(bytes, kPngSignature,
                                  sizeof(kPngSignature)) == 0;
  std::vector<uint8_t> pixels;
  uint32_t width = 0;
  uint32_t height = 0;
  const pictures::Decoded decoded =
      pictures::Decode(bytes, size, pixels, width, height);
  // Many Windows programs paste only the system's own bitmap, and others
  // prefer PNG, so both go on the clipboard.
  std::vector<uint8_t> png;
  std::vector<uint8_t> dib;
  if (decoded == pictures::Decoded::kPicture) {
    dib = CoreHost::DibFromBgra(pixels, width, height);
    png = is_png ? std::vector<uint8_t>(bytes, bytes + size)
                 : pictures::EncodePng(pixels, width, height);
  } else if (decoded == pictures::Decoded::kTooLarge && is_png) {
    // Programs that read PNG can still take it as it came.
    png.assign(bytes, bytes + size);
  }
  Wipe(pixels);
  if (png.empty() && dib.empty()) {
    return;
  }
  EncodableMap clip;
  clip[EncodableValue("format")] = EncodableValue("png");
  if (!png.empty()) {
    clip[EncodableValue("data")] = EncodableValue(std::move(png));
  }
  if (!dib.empty()) {
    clip[EncodableValue("dib")] = EncodableValue(std::move(dib));
  }
  clip[EncodableValue("ttl_ms")] = EncodableValue(Lifetime(ttl_ms));
  clipboard_->Apply(clip);
  for (const char* key : {"data", "dib"}) {
    if (auto* stored = std::get_if<std::vector<uint8_t>>(
            &clip[EncodableValue(key)])) {
      Wipe(*stored);
    }
  }
}

void AppHost::OnClipFiles(const char* id, uint64_t ttl_ms, const char* from) {
  if (!file_paste_ || id == nullptr) {
    return;
  }
  file_paste_->Offer(id, Lifetime(ttl_ms));
  EncodableMap arguments;
  arguments[EncodableValue("from")] =
      EncodableValue(std::string(from == nullptr ? "" : from));
  arguments[EncodableValue("state")] = EncodableValue("ready");
  Tell("clipboardFilesOffered", std::move(arguments));
}

void AppHost::Copied(const EncodableMap& clip) {
  const std::string* format = Find<std::string>(clip, "format");
  if (format == nullptr) {
    return;
  }
  if (*format == "text") {
    if (const std::string* text = Find<std::string>(clip, "text")) {
      CoreHost::ClipText(*text);
    }
    return;
  }
  if (*format == "files") {
    const EncodableList* list = Find<EncodableList>(clip, "paths");
    if (list == nullptr) {
      return;
    }
    std::vector<std::string> paths;
    for (const EncodableValue& value : *list) {
      if (const auto* path = std::get_if<std::string>(&value)) {
        paths.push_back(*path);
      }
    }
    CoreHost::ClipFiles(paths);
    return;
  }
  const std::vector<uint8_t>* data = Find<std::vector<uint8_t>>(clip, "data");
  if (data == nullptr || data->empty()) {
    return;
  }
  // Windows hands over pictures as its own bitmaps, and a copied PNG can
  // carry stray bytes after the image. Either way a fresh PNG leaves.
  std::vector<uint8_t> source = *format == "dib"
                                    ? CoreHost::BmpFromDib(data->data(),
                                                           data->size())
                                    : *data;
  std::vector<uint8_t> pixels;
  uint32_t width = 0;
  uint32_t height = 0;
  const pictures::Decoded decoded =
      pictures::Decode(source.data(), source.size(), pixels, width, height);
  Wipe(source);
  if (decoded != pictures::Decoded::kPicture) {
    return;
  }
  std::vector<uint8_t> png = pictures::EncodePng(pixels, width, height);
  Wipe(pixels);
  CoreHost::ClipPng(png);
  Wipe(png);
}

// ---- The interface ----

void AppHost::OpenInterface(bool someone_waiting) {
  if (quitting_ || window_ == nullptr) {
    return;
  }
  if (interface_) {
    // Reopened before the closed window was let go: it stays.
    dropping_ = false;
    if (!interface_->BringForward() && someone_waiting && background_) {
      background_->NotifyOffer();
    }
    return;
  }
  someone_waiting_ = someone_waiting;
  attached_ = false;
  interface_ready_ = false;
  interface_ = std::make_unique<FlutterWindow>(project_, *this);
  // The same size as the Mac window.
  const Win32Window::Point origin(10, 10);
  const Win32Window::Size size(1120, 760);
  if (!interface_->Create(options_.title, origin, size)) {
    interface_ = nullptr;
    return;
  }
  FitToWorkArea(interface_->GetHandle());
}

void AppHost::Attach(Answer answer) {
  if (started_ == CoreHost::Started::kRunning) {
    attached_ = true;
    answer(AttachAnswer());
    return;
  }
  attaching_.push_back(std::move(answer));
  // A start that failed is tried again, as the owner may have fixed it.
  StartCore();
}

void AppHost::InterfaceReady() {
  interface_ready_ = true;
  DeliverPaths();
}

void AppHost::InterfaceDrawn() {
  if (!interface_) {
    return;
  }
  if (!interface_->BringForward() && someone_waiting_ && background_) {
    background_->NotifyOffer();
  }
  someone_waiting_ = false;
}

void AppHost::InterfaceClosing() {
  // With the shared clipboard on, closing the window leaves the app working
  // next to the clock, as on the Mac: the window goes, with all of Flutter.
  if (!quitting_ && background_ && background_->is_on() && interface_) {
    ::ShowWindow(interface_->GetHandle(), SW_HIDE);
    dropping_ = true;
    ::PostMessageW(window_, kDropMessage, 0, 0);
    return;
  }
  // Not from inside the interface's own message, which still runs.
  if (interface_) {
    ::ShowWindow(interface_->GetHandle(), SW_HIDE);
  }
  ::PostMessageW(window_, QuitMessage(), 0, 0);
}

void AppHost::DropInterface() {
  if (!dropping_ || !interface_) {
    return;
  }
  dropping_ = false;
  CloseInterfaceWindow();
  // Pairing needs the screen, so it ends with the window.
  if (pairing_) {
    CoreHost::PairCancel();
  }
}

void AppHost::Tell(const std::string& method, EncodableMap arguments) {
  if (interface_ && attached_) {
    interface_->Tell(method, std::move(arguments));
  }
}

void AppHost::DeliverPaths() {
  if (!interface_ || !interface_ready_ || waiting_paths_.empty()) {
    return;
  }
  EncodableList paths;
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
  Tell("sharedFiles", std::move(arguments));
}

void AppHost::Quit() {
  if (quitting_) {
    return;
  }
  quitting_ = true;
  CloseInterfaceWindow();
  if (window_ != nullptr) {
    ::DestroyWindow(window_);
  }
}

void AppHost::CloseInterfaceWindow() {
  attached_ = false;
  interface_ready_ = false;
  if (!interface_) {
    return;
  }
  // Answers still owed to this interface go while its engine can carry them.
  attaching_.clear();
  // Destroyed while the object is whole, so that its own clean-up runs and
  // the engine shuts down before the window goes.
  interface_->Destroy();
  interface_ = nullptr;
}
