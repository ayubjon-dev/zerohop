#include "file_paste.h"

#include <ole2.h>
#include <shlobj.h>
#include <shobjidl.h>

#include <algorithm>
#include <climits>
#include <cstring>
#include <memory>
#include <utility>
#include <vector>

namespace {

constexpr UINT kOfferMessage = WM_APP + 1;
constexpr int64_t kLongestLifetimeMs = 60 * 60 * 1000;
constexpr DWORD kStopWaitMs = 2000;
constexpr size_t kClipIdLength = 32;

using CallFunction = char* (*)(const char*);
using FreeFunction = void (*)(char*);

struct Request {
  std::string clip_id;
  int64_t lifetime_ms;
};

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

// Reads the JSON string whose opening quote is at |at|, and moves past it.
bool ReadJsonString(const std::string& json, size_t& at, std::wstring& out) {
  if (at >= json.size() || json[at] != '"') {
    return false;
  }
  ++at;
  // Plain text is gathered as UTF-8 and converted whole, between escapes.
  std::string plain;
  const auto flush = [&]() {
    out += Utf16(plain);
    plain.clear();
  };
  while (at < json.size()) {
    const char c = json[at++];
    if (c == '"') {
      flush();
      return true;
    }
    if (c != '\\') {
      plain.push_back(c);
      continue;
    }
    if (at >= json.size()) {
      return false;
    }
    flush();
    const char escaped = json[at++];
    switch (escaped) {
      case '"':
        out.push_back(L'"');
        break;
      case '\\':
        out.push_back(L'\\');
        break;
      case '/':
        out.push_back(L'/');
        break;
      case 'b':
        out.push_back(L'\b');
        break;
      case 'f':
        out.push_back(L'\f');
        break;
      case 'n':
        out.push_back(L'\n');
        break;
      case 'r':
        out.push_back(L'\r');
        break;
      case 't':
        out.push_back(L'\t');
        break;
      case 'u': {
        if (json.size() - at < 4) {
          return false;
        }
        unsigned int value = 0;
        for (int digit = 0; digit < 4; ++digit) {
          const char h = json[at++];
          value <<= 4;
          if (h >= '0' && h <= '9') {
            value |= static_cast<unsigned int>(h - '0');
          } else if (h >= 'a' && h <= 'f') {
            value |= static_cast<unsigned int>(h - 'a' + 10);
          } else if (h >= 'A' && h <= 'F') {
            value |= static_cast<unsigned int>(h - 'A' + 10);
          } else {
            return false;
          }
        }
        // UTF-16 units, so the two halves of a pair simply follow each other.
        out.push_back(static_cast<wchar_t>(value));
        break;
      }
      default:
        return false;
    }
  }
  return false;
}

// The paths in the core's answer: {"reply":1,"ok":{"paths":[...]}}. Empty
// for an error, or anything else.
std::vector<std::wstring> PathsIn(const std::string& answer) {
  const std::string key = "\"ok\":{\"paths\":[";
  size_t at = answer.find(key);
  if (at == std::string::npos) {
    return {};
  }
  at += key.size();
  std::vector<std::wstring> paths;
  while (at < answer.size()) {
    if (answer[at] == ']') {
      return paths;
    }
    if (answer[at] == ',') {
      ++at;
      continue;
    }
    std::wstring path;
    if (!ReadJsonString(answer, at, path)) {
      return {};
    }
    if (!path.empty()) {
      paths.push_back(std::move(path));
    }
  }
  return {};
}

// Asks the core for the files, which fetches them now. Blocks until then.
std::vector<std::wstring> Fetch(const std::string& clip_id) {
  // The interface loaded the core long before anything can be pasted.
  HMODULE core = ::GetModuleHandleW(L"zerohop_ffi.dll");
  if (core == nullptr) {
    core = ::LoadLibraryExW(L"zerohop_ffi.dll", nullptr,
                            LOAD_LIBRARY_SEARCH_APPLICATION_DIR);
  }
  if (core == nullptr) {
    return {};
  }
  const auto call = reinterpret_cast<CallFunction>(
      reinterpret_cast<void*>(::GetProcAddress(core, "zh_call")));
  const auto release = reinterpret_cast<FreeFunction>(
      reinterpret_cast<void*>(::GetProcAddress(core, "zh_free")));
  if (call == nullptr || release == nullptr) {
    return {};
  }
  const std::string request =
      "{\"id\":1,\"method\":\"paste_files\",\"params\":{\"id\":\"" + clip_id +
      "\"}}";
  char* answer = call(request.c_str());
  if (answer == nullptr) {
    return {};
  }
  std::vector<std::wstring> paths = PathsIn(answer);
  release(answer);
  return paths;
}

// Explorer's list of files to copy, from the folder the core fetched into.
HGLOBAL DropFiles(const std::vector<std::wstring>& paths) {
  size_t characters = 1;
  for (const std::wstring& path : paths) {
    characters += path.size() + 1;
  }
  const size_t size = sizeof(DROPFILES) + characters * sizeof(wchar_t);
  HGLOBAL handle = ::GlobalAlloc(GHND, size);
  if (handle == nullptr) {
    return nullptr;
  }
  auto* drop = static_cast<DROPFILES*>(::GlobalLock(handle));
  if (drop == nullptr) {
    ::GlobalFree(handle);
    return nullptr;
  }
  drop->pFiles = sizeof(DROPFILES);
  drop->fWide = TRUE;
  auto* out = reinterpret_cast<wchar_t*>(reinterpret_cast<BYTE*>(drop) +
                                         sizeof(DROPFILES));
  for (const std::wstring& path : paths) {
    std::memcpy(out, path.c_str(), (path.size() + 1) * sizeof(wchar_t));
    out += path.size() + 1;
  }
  // GHND zeroed the memory, so the list already ends with a second NUL.
  ::GlobalUnlock(handle);
  return handle;
}

HGLOBAL Number(DWORD value) {
  HGLOBAL handle = ::GlobalAlloc(GMEM_MOVEABLE, sizeof(value));
  if (handle == nullptr) {
    return nullptr;
  }
  void* target = ::GlobalLock(handle);
  if (target == nullptr) {
    ::GlobalFree(handle);
    return nullptr;
  }
  std::memcpy(target, &value, sizeof(value));
  ::GlobalUnlock(handle);
  return handle;
}

FORMATETC Format(UINT format) {
  FORMATETC entry = {};
  entry.cfFormat = static_cast<CLIPFORMAT>(format);
  entry.dwAspect = DVASPECT_CONTENT;
  entry.lindex = -1;
  entry.tymed = TYMED_HGLOBAL;
  return entry;
}

// The promise of files on the clipboard. Explorer extracts it on a thread
// of its own, so a long fetch never stops it either.
class FilesData : public IDataObject, public IDataObjectAsyncCapability {
 public:
  FilesData(std::string clip_id, HWND window)
      : clip_id_(std::move(clip_id)),
        window_(window),
        effect_(::RegisterClipboardFormatW(CFSTR_PREFERREDDROPEFFECT)),
        marker_(FilePaste::MarkerFormat()),
        history_(::RegisterClipboardFormatW(L"CanIncludeInClipboardHistory")),
        cloud_(::RegisterClipboardFormatW(L"CanUploadToCloudClipboard")),
        exclude_(::RegisterClipboardFormatW(
            L"ExcludeClipboardContentFromMonitorProcessing")) {
    for (const UINT format :
         {static_cast<UINT>(CF_HDROP), effect_, marker_, history_, cloud_,
          exclude_}) {
      if (format != 0) {
        formats_.push_back(Format(format));
      }
    }
  }

  FilesData(const FilesData&) = delete;
  FilesData& operator=(const FilesData&) = delete;

  // IUnknown
  STDMETHODIMP QueryInterface(REFIID id, void** out) override {
    if (out == nullptr) {
      return E_POINTER;
    }
    if (id == IID_IUnknown || id == IID_IDataObject) {
      *out = static_cast<IDataObject*>(this);
    } else if (id == __uuidof(IDataObjectAsyncCapability)) {
      *out = static_cast<IDataObjectAsyncCapability*>(this);
    } else {
      *out = nullptr;
      return E_NOINTERFACE;
    }
    AddRef();
    return S_OK;
  }

  STDMETHODIMP_(ULONG) AddRef() override {
    return static_cast<ULONG>(::InterlockedIncrement(&references_));
  }

  STDMETHODIMP_(ULONG) Release() override {
    const LONG left = ::InterlockedDecrement(&references_);
    if (left == 0) {
      delete this;
    }
    return static_cast<ULONG>(left);
  }

  // IDataObject
  STDMETHODIMP GetData(FORMATETC* format, STGMEDIUM* medium) override {
    if (format == nullptr || medium == nullptr) {
      return E_INVALIDARG;
    }
    if ((format->tymed & TYMED_HGLOBAL) == 0) {
      return DV_E_TYMED;
    }
    const UINT asked = format->cfFormat;
    HGLOBAL data = nullptr;
    if (asked == CF_HDROP) {
      // The owner pasted: only now do the files move.
      if (paths_.empty()) {
        paths_ = Fetch(clip_id_);
      }
      if (paths_.empty()) {
        if (!reported_) {
          reported_ = true;
          ::PostMessageW(window_, FilePaste::kFailedMessage, 0, 0);
        }
        return E_FAIL;
      }
      data = DropFiles(paths_);
    } else if (asked == effect_) {
      data = Number(DROPEFFECT_COPY);
    } else if (asked == marker_) {
      data = Number(1);
    } else if (asked == history_ || asked == cloud_ || asked == exclude_) {
      data = Number(0);
    } else {
      return DV_E_FORMATETC;
    }
    if (data == nullptr) {
      return E_OUTOFMEMORY;
    }
    medium->tymed = TYMED_HGLOBAL;
    medium->hGlobal = data;
    medium->pUnkForRelease = nullptr;
    return S_OK;
  }

  STDMETHODIMP GetDataHere(FORMATETC*, STGMEDIUM*) override {
    return E_NOTIMPL;
  }

  STDMETHODIMP QueryGetData(FORMATETC* format) override {
    if (format == nullptr) {
      return E_INVALIDARG;
    }
    if ((format->tymed & TYMED_HGLOBAL) == 0) {
      return DV_E_TYMED;
    }
    const bool offered =
        std::any_of(formats_.begin(), formats_.end(), [&](const FORMATETC& f) {
          return f.cfFormat == format->cfFormat;
        });
    return offered ? S_OK : DV_E_FORMATETC;
  }

  STDMETHODIMP GetCanonicalFormatEtc(FORMATETC* in, FORMATETC* out) override {
    if (in == nullptr || out == nullptr) {
      return E_INVALIDARG;
    }
    *out = *in;
    out->ptd = nullptr;
    return DATA_S_SAMEFORMATETC;
  }

  // Explorer reports how the paste went; nothing here depends on it.
  STDMETHODIMP SetData(FORMATETC*, STGMEDIUM* medium, BOOL release) override {
    if (release && medium != nullptr) {
      ::ReleaseStgMedium(medium);
    }
    return S_OK;
  }

  STDMETHODIMP EnumFormatEtc(DWORD direction, IEnumFORMATETC** out) override {
    if (direction != DATADIR_GET) {
      return E_NOTIMPL;
    }
    return ::SHCreateStdEnumFmtEtc(static_cast<UINT>(formats_.size()),
                                   formats_.data(), out);
  }

  STDMETHODIMP DAdvise(FORMATETC*, DWORD, IAdviseSink*, DWORD*) override {
    return OLE_E_ADVISENOTSUPPORTED;
  }

  STDMETHODIMP DUnadvise(DWORD) override { return OLE_E_ADVISENOTSUPPORTED; }

  STDMETHODIMP EnumDAdvise(IEnumSTATDATA**) override {
    return OLE_E_ADVISENOTSUPPORTED;
  }

  // IDataObjectAsyncCapability
  STDMETHODIMP SetAsyncMode(BOOL async) override {
    async_ = async;
    return S_OK;
  }

  STDMETHODIMP GetAsyncMode(BOOL* async) override {
    if (async == nullptr) {
      return E_INVALIDARG;
    }
    *async = async_;
    return S_OK;
  }

  STDMETHODIMP StartOperation(IBindCtx*) override {
    in_operation_ = TRUE;
    return S_OK;
  }

  STDMETHODIMP InOperation(BOOL* in_operation) override {
    if (in_operation == nullptr) {
      return E_INVALIDARG;
    }
    *in_operation = in_operation_;
    return S_OK;
  }

  STDMETHODIMP EndOperation(HRESULT, IBindCtx*, DWORD) override {
    in_operation_ = FALSE;
    return S_OK;
  }

 private:
  ~FilesData() = default;

  LONG references_ = 1;
  std::string clip_id_;
  HWND window_;
  UINT effect_;
  UINT marker_;
  UINT history_;
  UINT cloud_;
  UINT exclude_;
  std::vector<FORMATETC> formats_;
  std::vector<std::wstring> paths_;
  bool reported_ = false;
  BOOL async_ = TRUE;
  BOOL in_operation_ = FALSE;
};

bool IsClipId(const std::string& text) {
  return text.size() == kClipIdLength &&
         std::all_of(text.begin(), text.end(), [](char c) {
           return (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f');
         });
}

}  // namespace

FilePaste::FilePaste(HWND window) : window_(window) {
  ready_ = ::CreateEventW(nullptr, TRUE, FALSE, nullptr);
  if (ready_ == nullptr) {
    return;
  }
  thread_ = ::CreateThread(nullptr, 0, &FilePaste::Run, this, 0, &thread_id_);
  if (thread_ != nullptr) {
    // Offers posted before the thread has a message queue would be lost.
    ::WaitForSingleObject(ready_, INFINITE);
  }
}

FilePaste::~FilePaste() {
  if (thread_ != nullptr) {
    ::PostThreadMessageW(thread_id_, WM_QUIT, 0, 0);
    // A paste still fetching is left behind rather than waited for.
    ::WaitForSingleObject(thread_, kStopWaitMs);
    ::CloseHandle(thread_);
  }
  if (ready_ != nullptr) {
    ::CloseHandle(ready_);
  }
}

UINT FilePaste::MarkerFormat() {
  static const UINT format =
      ::RegisterClipboardFormatW(L"Zerohop files from another device");
  return format;
}

void FilePaste::Offer(const std::string& clip_id, int64_t lifetime_ms) {
  if (thread_ == nullptr || !IsClipId(clip_id)) {
    return;
  }
  auto request = std::make_unique<Request>(Request{
      clip_id, std::clamp<int64_t>(lifetime_ms, 1, kLongestLifetimeMs)});
  if (::PostThreadMessageW(thread_id_, kOfferMessage, 0,
                           reinterpret_cast<LPARAM>(request.get()))) {
    request.release();
  }
}

DWORD WINAPI FilePaste::Run(void* self) {
  static_cast<FilePaste*>(self)->Loop();
  return 0;
}

void FilePaste::Loop() {
  const bool ole = SUCCEEDED(::OleInitialize(nullptr));
  MSG message;
  // Makes the message queue before anyone is told it is ready.
  ::PeekMessageW(&message, nullptr, WM_USER, WM_USER, PM_NOREMOVE);
  ::SetEvent(ready_);

  IDataObject* current = nullptr;
  UINT_PTR timer = 0;
  const auto withdraw = [&]() {
    if (timer != 0) {
      ::KillTimer(nullptr, timer);
      timer = 0;
    }
    if (current != nullptr) {
      // Anything copied since belongs to the owner and is left alone.
      if (::OleIsCurrentClipboard(current) == S_OK) {
        ::OleSetClipboard(nullptr);
      }
      current->Release();
      current = nullptr;
    }
  };

  while (::GetMessageW(&message, nullptr, 0, 0) > 0) {
    if (message.hwnd == nullptr && message.message == kOfferMessage) {
      std::unique_ptr<Request> request(
          reinterpret_cast<Request*>(message.lParam));
      withdraw();
      if (!ole) {
        continue;
      }
      auto* data = new FilesData(request->clip_id, window_);
      if (::OleSetClipboard(data) == S_OK) {
        current = data;
        timer = ::SetTimer(nullptr, 0,
                           static_cast<UINT>(request->lifetime_ms), nullptr);
      } else {
        data->Release();
      }
      continue;
    }
    if (message.hwnd == nullptr && message.message == WM_TIMER &&
        timer != 0 && message.wParam == timer) {
      withdraw();
      continue;
    }
    ::TranslateMessage(&message);
    ::DispatchMessageW(&message);
  }
  withdraw();
  if (ole) {
    ::OleUninitialize();
  }
}
