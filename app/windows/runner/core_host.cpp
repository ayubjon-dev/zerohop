#include "core_host.h"

#include <atomic>
#include <memory>

#include "utils.h"

namespace {

constexpr size_t kSecretSize = 32;
constexpr int kSecretAttempts = 3;
constexpr DWORD kSecretPauseMs = 1000;

// The functions the core exports, as declared in crates/zerohop-ffi.
using Callback = void (*)(char*);
using StartFunction = int32_t (*)(const char*, const char*, const uint8_t*,
                                  size_t, Callback);
using SecretFunction = int32_t (*)(const char*, uint8_t*, size_t);
using FreeFunction = void (*)(char*);

struct HostCalls {
  void* context;
  void (*snapshot)(void*, bool, bool, const char*, const char*, uint16_t);
  void (*offer_arrived)(void*);
  void (*clip_text)(void*, const char*, uint64_t, const char*);
  void (*clip_picture)(void*, const uint8_t*, size_t, uint64_t, const char*);
  void (*clip_files)(void*, const char*, uint64_t, const char*);
};

using ReadFunction = int32_t (*)(const char*, const HostCalls*);
using TextFunction = void (*)(const char*);
using PngFunction = void (*)(const uint8_t*, size_t);
using ListFunction = void (*)(const char* const*, size_t);
using SeenFunction = void (*)(const char*, const char* const*, size_t,
                              uint16_t, const char*);
using PlainFunction = void (*)();
using BootstrapFunction = int32_t (*)();
using BmpFunction = uint8_t* (*)(const uint8_t*, size_t, size_t*);
using DibFunction = uint8_t* (*)(const uint8_t*, uint32_t, uint32_t, size_t*);
using FreeBytesFunction = void (*)(uint8_t*, size_t);

struct Functions {
  StartFunction start = nullptr;
  SecretFunction secret = nullptr;
  FreeFunction free = nullptr;
  ReadFunction read = nullptr;
  TextFunction clip_text = nullptr;
  PngFunction clip_png = nullptr;
  ListFunction clip_files = nullptr;
  SeenFunction seen = nullptr;
  TextFunction lost = nullptr;
  PlainFunction pair_cancel = nullptr;
  BootstrapFunction bootstrap = nullptr;
  BmpFunction bmp_from_dib = nullptr;
  DibFunction dib_from_bgra = nullptr;
  FreeBytesFunction free_bytes = nullptr;
};

Functions g_core;
std::atomic<HWND> g_window{nullptr};

// What zh_host_read answers for a message the interface should hear too.
constexpr int32_t kForInterface = 0;

template <typename T>
bool Find(HMODULE library, const char* name, T& function) {
  function = reinterpret_cast<T>(
      reinterpret_cast<void*>(::GetProcAddress(library, name)));
  return function != nullptr;
}

void Wipe(std::string& text) {
  if (!text.empty()) {
    ::SecureZeroMemory(text.data(), text.size());
  }
}

// Each message from the core, on one of its own threads. It may carry copied
// content, so the core's copy is wiped as it is freed.
void Received(char* message) {
  if (message == nullptr) {
    return;
  }
  auto text = std::make_unique<std::string>(message);
  g_core.free(message);
  HWND window = g_window.load();
  if (window != nullptr &&
      ::PostMessageW(window, CoreHost::kMessage, 0,
                     reinterpret_cast<LPARAM>(text.get()))) {
    text.release();
    return;
  }
  Wipe(*text);
}

CoreHost::Listener* ListenerOf(void* context) {
  return static_cast<CoreHost::Listener*>(context);
}

void OnSnapshot(void* context, bool clipboard, bool pairing,
                const char* instance, const char* tag, uint16_t port) {
  ListenerOf(context)->OnSnapshot(clipboard, pairing, instance, tag, port);
}

void OnOfferArrived(void* context) { ListenerOf(context)->OnOfferArrived(); }

void OnClipText(void* context, const char* text, uint64_t ttl_ms,
                const char* from) {
  ListenerOf(context)->OnClipText(text, ttl_ms, from);
}

void OnClipPicture(void* context, const uint8_t* bytes, size_t size,
                   uint64_t ttl_ms, const char* from) {
  ListenerOf(context)->OnClipPicture(bytes, size, ttl_ms, from);
}

void OnClipFiles(void* context, const char* id, uint64_t ttl_ms,
                 const char* from) {
  ListenerOf(context)->OnClipFiles(id, ttl_ms, from);
}

std::vector<uint8_t> TakeBytes(uint8_t* bytes, size_t size) {
  if (bytes == nullptr) {
    return {};
  }
  std::vector<uint8_t> copy(bytes, bytes + size);
  g_core.free_bytes(bytes, size);
  return copy;
}

}  // namespace

bool CoreHost::Load(HWND window) {
  HMODULE library = ::LoadLibraryExW(L"zerohop_ffi.dll", nullptr,
                                     LOAD_LIBRARY_SEARCH_APPLICATION_DIR);
  if (library == nullptr) {
    return false;
  }
  Functions found;
  const bool complete =
      Find(library, "zh_start", found.start) &&
      Find(library, "zh_system_secret", found.secret) &&
      Find(library, "zh_free", found.free) &&
      Find(library, "zh_host_read", found.read) &&
      Find(library, "zh_host_clip_text", found.clip_text) &&
      Find(library, "zh_host_clip_png", found.clip_png) &&
      Find(library, "zh_host_clip_files", found.clip_files) &&
      Find(library, "zh_host_seen", found.seen) &&
      Find(library, "zh_host_lost", found.lost) &&
      Find(library, "zh_host_pair_cancel", found.pair_cancel) &&
      Find(library, "zh_host_bootstrap", found.bootstrap) &&
      Find(library, "zh_bmp_from_dib", found.bmp_from_dib) &&
      Find(library, "zh_dib_from_bgra", found.dib_from_bgra) &&
      Find(library, "zh_free_bytes", found.free_bytes);
  if (!complete) {
    return false;
  }
  g_core = found;
  g_window.store(window);
  return true;
}

CoreHost::Started CoreHost::Start(const std::wstring& state_dir,
                                  const std::wstring& folder) {
  if (g_core.start == nullptr) {
    return Started::kFailed;
  }
  const std::string state = Utf8FromUtf16(state_dir.c_str());
  const std::string files = Utf8FromUtf16(folder.c_str());
  if (state.empty() || files.empty()) {
    return Started::kFailed;
  }
  uint8_t secret[kSecretSize] = {};
  int32_t opened = -1;
  // The key store can be briefly unavailable while Windows is starting.
  for (int attempt = 0; attempt < kSecretAttempts; ++attempt) {
    if (attempt > 0) {
      ::Sleep(kSecretPauseMs);
    }
    opened = g_core.secret(state.c_str(), secret, kSecretSize);
    if (opened == 0) {
      break;
    }
  }
  if (opened != 0) {
    ::SecureZeroMemory(secret, sizeof(secret));
    return Started::kKeyUnavailable;
  }
  const int32_t started =
      g_core.start(state.c_str(), files.c_str(), secret, kSecretSize, &Received);
  ::SecureZeroMemory(secret, sizeof(secret));
  if (started != 0) {
    return Started::kFailed;
  }
  // A device key that will not open is the interface's to explain.
  g_core.bootstrap();
  return Started::kRunning;
}

std::string CoreHost::Take(LPARAM lparam, Listener& listener) {
  std::unique_ptr<std::string> message(reinterpret_cast<std::string*>(lparam));
  if (!message || g_core.read == nullptr) {
    return std::string();
  }
  const HostCalls calls = {&listener,     &OnSnapshot,    &OnOfferArrived,
                           &OnClipText,   &OnClipPicture, &OnClipFiles};
  std::string forward;
  if (g_core.read(message->c_str(), &calls) == kForInterface) {
    forward = *message;
  }
  Wipe(*message);
  return forward;
}

void CoreHost::ClipText(const std::string& utf8) {
  if (g_core.clip_text != nullptr && !utf8.empty()) {
    g_core.clip_text(utf8.c_str());
  }
}

void CoreHost::ClipPng(const std::vector<uint8_t>& png) {
  if (g_core.clip_png != nullptr && !png.empty()) {
    g_core.clip_png(png.data(), png.size());
  }
}

void CoreHost::ClipFiles(const std::vector<std::string>& paths) {
  if (g_core.clip_files == nullptr || paths.empty()) {
    return;
  }
  std::vector<const char*> list;
  list.reserve(paths.size());
  for (const std::string& path : paths) {
    list.push_back(path.c_str());
  }
  g_core.clip_files(list.data(), list.size());
}

void CoreHost::Seen(const std::string& instance,
                    const std::vector<std::string>& addresses, uint16_t port,
                    const std::string& tag) {
  if (g_core.seen == nullptr || addresses.empty()) {
    return;
  }
  std::vector<const char*> list;
  list.reserve(addresses.size());
  for (const std::string& address : addresses) {
    list.push_back(address.c_str());
  }
  g_core.seen(instance.c_str(), list.data(), list.size(), port,
              tag.empty() ? nullptr : tag.c_str());
}

void CoreHost::Lost(const std::string& instance) {
  if (g_core.lost != nullptr) {
    g_core.lost(instance.c_str());
  }
}

void CoreHost::PairCancel() {
  if (g_core.pair_cancel != nullptr) {
    g_core.pair_cancel();
  }
}

std::vector<uint8_t> CoreHost::BmpFromDib(const uint8_t* dib, size_t size) {
  if (g_core.bmp_from_dib == nullptr || dib == nullptr || size == 0) {
    return {};
  }
  size_t length = 0;
  uint8_t* bmp = g_core.bmp_from_dib(dib, size, &length);
  return TakeBytes(bmp, length);
}

std::vector<uint8_t> CoreHost::DibFromBgra(const std::vector<uint8_t>& pixels,
                                           uint32_t width, uint32_t height) {
  if (g_core.dib_from_bgra == nullptr || pixels.empty() ||
      pixels.size() != static_cast<size_t>(width) * height * 4) {
    return {};
  }
  size_t length = 0;
  uint8_t* dib = g_core.dib_from_bgra(pixels.data(), width, height, &length);
  return TakeBytes(dib, length);
}
