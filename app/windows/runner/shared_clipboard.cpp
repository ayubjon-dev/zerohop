#include "shared_clipboard.h"

#include <shellapi.h>

#include <algorithm>
#include <climits>
#include <cstdint>
#include <cstring>
#include <cwchar>
#include <utility>
#include <vector>

#include "file_paste.h"

using flutter::EncodableList;
using flutter::EncodableMap;
using flutter::EncodableValue;

namespace {

// The most a copied picture may take, as on the Mac.
constexpr size_t kMaxPicture = 24 * 1024 * 1024;
// Counted in UTF-16 units: far beyond anything typed or pasted by hand.
constexpr size_t kMaxText = 8 * 1024 * 1024;
// Files and folders picked at once in Explorer, not counting what they hold.
constexpr UINT kMaxFiles = 1000;
constexpr UINT_PTR kExpiryTimer = 0x5A48;
constexpr int64_t kDefaultLifetimeMs = 120000;
constexpr int64_t kLongestLifetimeMs = 60 * 60 * 1000;
constexpr UINT kClearRetryMs = 1000;
constexpr int kOpenAttempts = 5;
constexpr DWORD kOpenPauseMs = 20;

EncodableMap State(const char* state) {
  EncodableMap clip;
  clip[EncodableValue("state")] = EncodableValue(state);
  return clip;
}

template <typename T>
const T* Find(const EncodableMap& map, const char* key) {
  const auto found = map.find(EncodableValue(key));
  if (found == map.end()) {
    return nullptr;
  }
  return std::get_if<T>(&found->second);
}

// Another program may be holding the clipboard for a moment.
class OpenClipboardScope {
 public:
  explicit OpenClipboardScope(HWND window) {
    for (int attempt = 0; attempt < kOpenAttempts && !open_; ++attempt) {
      if (attempt > 0) {
        ::Sleep(kOpenPauseMs);
      }
      open_ = ::OpenClipboard(window) != FALSE;
    }
  }
  ~OpenClipboardScope() {
    if (open_) {
      ::CloseClipboard();
    }
  }
  OpenClipboardScope(const OpenClipboardScope&) = delete;
  OpenClipboardScope& operator=(const OpenClipboardScope&) = delete;

  bool is_open() const { return open_; }

 private:
  bool open_ = false;
};

// The bytes behind a clipboard handle, readable while this lives.
class LockedBytes {
 public:
  explicit LockedBytes(HANDLE handle) : handle_(handle) {
    if (handle_ != nullptr) {
      data_ = static_cast<const uint8_t*>(::GlobalLock(handle_));
      if (data_ != nullptr) {
        size_ = ::GlobalSize(handle_);
      }
    }
  }
  ~LockedBytes() {
    if (data_ != nullptr) {
      ::GlobalUnlock(handle_);
    }
  }
  LockedBytes(const LockedBytes&) = delete;
  LockedBytes& operator=(const LockedBytes&) = delete;

  const uint8_t* data() const { return data_; }
  size_t size() const { return size_; }

 private:
  HANDLE handle_;
  const uint8_t* data_ = nullptr;
  size_t size_ = 0;
};

std::string Utf8(const wchar_t* text, size_t length) {
  if (length == 0 || length > static_cast<size_t>(INT_MAX)) {
    return std::string();
  }
  const int count = static_cast<int>(length);
  const int needed = ::WideCharToMultiByte(CP_UTF8, 0, text, count, nullptr,
                                           0, nullptr, nullptr);
  if (needed <= 0) {
    return std::string();
  }
  std::string utf8(static_cast<size_t>(needed), '\0');
  if (::WideCharToMultiByte(CP_UTF8, 0, text, count, utf8.data(), needed,
                            nullptr, nullptr) != needed) {
    return std::string();
  }
  return utf8;
}

std::wstring Utf16(const std::string& text) {
  if (text.empty() || text.size() > static_cast<size_t>(INT_MAX)) {
    return std::wstring();
  }
  const int count = static_cast<int>(text.size());
  const int needed =
      ::MultiByteToWideChar(CP_UTF8, 0, text.data(), count, nullptr, 0);
  if (needed <= 0) {
    return std::wstring();
  }
  std::wstring utf16(static_cast<size_t>(needed), L'\0');
  if (::MultiByteToWideChar(CP_UTF8, 0, text.data(), count, utf16.data(),
                            needed) != needed) {
    return std::wstring();
  }
  return utf16;
}

void Wipe(std::string& text) {
  if (!text.empty()) {
    ::SecureZeroMemory(text.data(), text.size());
  }
}

void Wipe(std::wstring& text) {
  if (!text.empty()) {
    ::SecureZeroMemory(text.data(), text.size() * sizeof(wchar_t));
  }
}

// Hands a copy of the bytes to the clipboard, which owns it from then on.
bool Put(UINT format, const void* bytes, size_t size) {
  if (format == 0 || bytes == nullptr || size == 0) {
    return false;
  }
  HGLOBAL handle = ::GlobalAlloc(GMEM_MOVEABLE, size);
  if (handle == nullptr) {
    return false;
  }
  void* target = ::GlobalLock(handle);
  if (target == nullptr) {
    ::GlobalFree(handle);
    return false;
  }
  std::memcpy(target, bytes, size);
  ::GlobalUnlock(handle);
  if (::SetClipboardData(format, handle) == nullptr) {
    ::GlobalFree(handle);
    return false;
  }
  return true;
}

}  // namespace

SharedClipboard::SharedClipboard(HWND window, Report report)
    : window_(window),
      report_(std::move(report)),
      png_(::RegisterClipboardFormatW(L"PNG")),
      exclude_(::RegisterClipboardFormatW(
          L"ExcludeClipboardContentFromMonitorProcessing")),
      history_(::RegisterClipboardFormatW(L"CanIncludeInClipboardHistory")),
      cloud_(::RegisterClipboardFormatW(L"CanUploadToCloudClipboard")),
      viewer_ignore_(::RegisterClipboardFormatW(L"Clipboard Viewer Ignore")) {}

SharedClipboard::~SharedClipboard() {
  Watch(false);
  ClearIfStillOurs();
}

void SharedClipboard::Watch(bool on) {
  if (on == watching_) {
    return;
  }
  watching_ = on;
  if (on) {
    // What was copied before this was turned on stays on this computer.
    seen_ = ::GetClipboardSequenceNumber();
    ::AddClipboardFormatListener(window_);
  } else {
    ::RemoveClipboardFormatListener(window_);
  }
}

bool SharedClipboard::HandleMessage(UINT message, WPARAM wparam) {
  if (message == WM_CLIPBOARDUPDATE) {
    OnChange();
    return true;
  }
  if (message == WM_TIMER && wparam == kExpiryTimer) {
    ::KillTimer(window_, kExpiryTimer);
    ClearIfStillOurs();
    return true;
  }
  return false;
}

void SharedClipboard::OnChange() {
  if (!watching_) {
    return;
  }
  const DWORD change = ::GetClipboardSequenceNumber();
  if (change == seen_) {
    return;
  }
  seen_ = change;
  if (ours_ && *ours_ == change) {
    return;
  }
  EncodableMap clip = Read();
  const std::string* state = Find<std::string>(clip, "state");
  if (state != nullptr && *state == "ok") {
    report_("clipboardCopied", std::move(clip));
  }
}

EncodableMap SharedClipboard::Read() {
  OpenClipboardScope board(window_);
  if (!board.is_open()) {
    return State("denied");
  }
  if (::CountClipboardFormats() == 0) {
    return State("empty");
  }
  // Files offered here from another device are not a new copy. Looked at
  // first, because reading any of their formats would ask that offer.
  const UINT offered = FilePaste::MarkerFormat();
  if (offered != 0 && ::IsClipboardFormatAvailable(offered)) {
    return State("same");
  }
  if (MarkedNotForOthers()) {
    return State("secret");
  }
  // Only the list goes now: the files move when the owner pastes them.
  if (::IsClipboardFormatAvailable(CF_HDROP)) {
    return ReadFiles();
  }
  if (::IsClipboardFormatAvailable(CF_UNICODETEXT)) {
    return ReadText();
  }
  if (png_ != 0 && ::IsClipboardFormatAvailable(png_)) {
    return ReadPicture(png_, "png");
  }
  // The fuller header keeps transparency, so it goes first.
  if (::IsClipboardFormatAvailable(CF_DIBV5)) {
    return ReadPicture(CF_DIBV5, "dib");
  }
  if (::IsClipboardFormatAvailable(CF_DIB)) {
    return ReadPicture(CF_DIB, "dib");
  }
  return State("empty");
}

// Password managers mark what has to stay on this computer: some with formats
// that only need to be present, others with a zero in a format of its own.
bool SharedClipboard::MarkedNotForOthers() {
  if ((exclude_ != 0 && ::IsClipboardFormatAvailable(exclude_)) ||
      (viewer_ignore_ != 0 && ::IsClipboardFormatAvailable(viewer_ignore_))) {
    return true;
  }
  for (const UINT format : {history_, cloud_}) {
    if (format == 0 || !::IsClipboardFormatAvailable(format)) {
      continue;
    }
    LockedBytes value(::GetClipboardData(format));
    DWORD allowed = 0;
    if (value.data() != nullptr && value.size() >= sizeof(allowed)) {
      std::memcpy(&allowed, value.data(), sizeof(allowed));
    }
    // A mark that cannot be read counts as a no.
    if (allowed == 0) {
      return true;
    }
  }
  return false;
}

EncodableMap SharedClipboard::ReadText() {
  LockedBytes bytes(::GetClipboardData(CF_UNICODETEXT));
  if (bytes.data() == nullptr) {
    return State("denied");
  }
  const auto* text = reinterpret_cast<const wchar_t*>(bytes.data());
  const size_t length = ::wcsnlen(text, bytes.size() / sizeof(wchar_t));
  if (length == 0) {
    return State("empty");
  }
  if (length > kMaxText) {
    return State("too_large");
  }
  std::string utf8 = Utf8(text, length);
  if (utf8.empty()) {
    return State("denied");
  }
  EncodableMap clip = State("ok");
  clip[EncodableValue("format")] = EncodableValue("text");
  clip[EncodableValue("text")] = EncodableValue(utf8);
  Wipe(utf8);
  return clip;
}

EncodableMap SharedClipboard::ReadFiles() {
  HANDLE handle = ::GetClipboardData(CF_HDROP);
  if (handle == nullptr) {
    return State("denied");
  }
  HDROP drop = static_cast<HDROP>(handle);
  const UINT count = ::DragQueryFileW(drop, 0xFFFFFFFF, nullptr, 0);
  if (count == 0) {
    return State("empty");
  }
  if (count > kMaxFiles) {
    return State("too_large");
  }
  EncodableList paths;
  for (UINT index = 0; index < count; ++index) {
    const UINT length = ::DragQueryFileW(drop, index, nullptr, 0);
    if (length == 0) {
      continue;
    }
    std::wstring path(static_cast<size_t>(length) + 1, L'\0');
    if (::DragQueryFileW(drop, index, path.data(), length + 1) == 0) {
      continue;
    }
    path.resize(length);
    std::string utf8 = Utf8(path.c_str(), path.size());
    if (!utf8.empty()) {
      paths.emplace_back(std::move(utf8));
    }
  }
  if (paths.empty()) {
    return State("empty");
  }
  EncodableMap clip = State("ok");
  clip[EncodableValue("format")] = EncodableValue("files");
  clip[EncodableValue("paths")] = EncodableValue(std::move(paths));
  return clip;
}

EncodableMap SharedClipboard::ReadPicture(UINT format, const char* name) {
  LockedBytes bytes(::GetClipboardData(format));
  if (bytes.data() == nullptr) {
    return State("denied");
  }
  if (bytes.size() == 0) {
    return State("empty");
  }
  if (bytes.size() > kMaxPicture) {
    return State("too_large");
  }
  EncodableMap clip = State("ok");
  clip[EncodableValue("format")] = EncodableValue(name);
  clip[EncodableValue("data")] = EncodableValue(
      std::vector<uint8_t>(bytes.data(), bytes.data() + bytes.size()));
  return clip;
}

void SharedClipboard::Apply(const EncodableMap& clip) {
  const std::string* format = Find<std::string>(clip, "format");
  if (format == nullptr) {
    return;
  }
  // Everything is made ready first: the owner's clipboard is emptied only
  // when there is something to put in its place.
  std::wstring text;
  const std::vector<uint8_t>* png = nullptr;
  const std::vector<uint8_t>* dib = nullptr;
  if (*format == "text") {
    const std::string* utf8 = Find<std::string>(clip, "text");
    if (utf8 != nullptr) {
      text = Utf16(*utf8);
    }
    if (text.empty()) {
      return;
    }
  } else {
    png = Find<std::vector<uint8_t>>(clip, "data");
    dib = Find<std::vector<uint8_t>>(clip, "dib");
    if (png != nullptr && (png->empty() || png->size() > kMaxPicture)) {
      png = nullptr;
    }
    if (dib != nullptr && dib->empty()) {
      dib = nullptr;
    }
    if (png == nullptr && dib == nullptr) {
      return;
    }
  }

  bool placed = false;
  {
    OpenClipboardScope board(window_);
    if (board.is_open() && ::EmptyClipboard() != FALSE) {
      if (!text.empty()) {
        placed = Put(CF_UNICODETEXT, text.c_str(),
                     (text.size() + 1) * sizeof(wchar_t));
      } else {
        if (png != nullptr) {
          placed = Put(png_, png->data(), png->size());
        }
        if (dib != nullptr) {
          placed = Put(CF_DIB, dib->data(), dib->size()) || placed;
        }
      }
      if (placed) {
        // Clipboard history, the cloud clipboard and clipboard tools leave
        // it alone.
        const DWORD no = 0;
        Put(history_, &no, sizeof(no));
        Put(cloud_, &no, sizeof(no));
        Put(exclude_, &no, sizeof(no));
      }
    }
  }
  Wipe(text);
  if (!placed) {
    return;
  }

  const DWORD change = ::GetClipboardSequenceNumber();
  ours_ = change;
  seen_ = change;
  int64_t lifetime = kDefaultLifetimeMs;
  if (const auto* as_int32 = Find<int32_t>(clip, "ttl_ms")) {
    lifetime = *as_int32;
  } else if (const auto* as_int64 = Find<int64_t>(clip, "ttl_ms")) {
    lifetime = *as_int64;
  }
  lifetime = std::clamp<int64_t>(lifetime, 1, kLongestLifetimeMs);
  ::SetTimer(window_, kExpiryTimer, static_cast<UINT>(lifetime), nullptr);
}

void SharedClipboard::ClearIfStillOurs() {
  ::KillTimer(window_, kExpiryTimer);
  if (!ours_) {
    return;
  }
  // Anything copied since belongs to the owner and is left alone.
  if (::GetClipboardSequenceNumber() != *ours_) {
    ours_.reset();
    return;
  }
  bool cleared = false;
  {
    OpenClipboardScope board(window_);
    cleared = board.is_open() && ::EmptyClipboard() != FALSE;
  }
  if (!cleared) {
    ::SetTimer(window_, kExpiryTimer, kClearRetryMs, nullptr);
    return;
  }
  ours_.reset();
  seen_ = ::GetClipboardSequenceNumber();
}
