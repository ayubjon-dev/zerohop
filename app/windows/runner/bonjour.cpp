#include "bonjour.h"

#include <atomic>
#include <cstdio>
#include <iterator>
#include <utility>
#include <vector>

#include "core_host.h"
#include "utils.h"

namespace {

constexpr char kSuffix[] = "._zerohop._tcp";
constexpr wchar_t kBrowseName[] = L"_zerohop._tcp.local";
constexpr UINT_PTR kRetryTimer = 0x5A49;
constexpr UINT kRetryMs = 5000;
// Taken-back announcements kept before the oldest is freed.
constexpr size_t kRetiredKept = 4;

// Where Windows' threads post what they found. One Bonjour per program.
std::atomic<HWND> g_window{nullptr};

struct Found {
  enum class Kind { kAdded, kRemoved, kResolved, kBrowseFailed };
  Kind kind = Kind::kAdded;
  std::string instance;
  std::string tag;
  std::vector<std::string> addresses;
  uint16_t port = 0;
};

void Post(std::unique_ptr<Found> found) {
  HWND window = g_window.load();
  if (window != nullptr &&
      ::PostMessageW(window, Bonjour::kMessage, 0,
                     reinterpret_cast<LPARAM>(found.get()))) {
    found.release();
  }
}

std::wstring Utf16(const std::string& text) {
  if (text.empty()) {
    return std::wstring();
  }
  const int count = static_cast<int>(text.size());
  const int needed =
      ::MultiByteToWideChar(CP_UTF8, 0, text.data(), count, nullptr, 0);
  if (needed <= 0) {
    return std::wstring();
  }
  std::wstring wide(static_cast<size_t>(needed), L'\0');
  ::MultiByteToWideChar(CP_UTF8, 0, text.data(), count, wide.data(), needed);
  return wide;
}

// "a1b2._zerohop._tcp.local" gives "a1b2". Zerohop's instance names are hex,
// so nothing in them needs unescaping.
std::string InstanceOf(const wchar_t* name) {
  if (name == nullptr) {
    return std::string();
  }
  const std::string full = Utf8FromUtf16(name);
  const size_t end = full.find(kSuffix);
  if (end == std::string::npos || end == 0) {
    return std::string();
  }
  return full.substr(0, end);
}

// The tag in a "t=..." entry of a TXT record.
bool TagIn(const wchar_t* entry, std::string& tag) {
  if (entry == nullptr || entry[0] != L't' || entry[1] != L'=') {
    return false;
  }
  tag = Utf8FromUtf16(entry + 2);
  return true;
}

std::string Ip4(const IP4_ADDRESS* address) {
  const auto* b = reinterpret_cast<const uint8_t*>(address);
  char text[16] = {};
  std::snprintf(text, sizeof(text), "%u.%u.%u.%u", static_cast<unsigned>(b[0]),
                static_cast<unsigned>(b[1]), static_cast<unsigned>(b[2]),
                static_cast<unsigned>(b[3]));
  return text;
}

// Written out in full; the core reads every standard form.
std::string Ip6(const IP6_ADDRESS* address) {
  const BYTE* b = address->IP6Byte;
  unsigned groups[8] = {};
  for (int i = 0; i < 8; ++i) {
    groups[i] = (static_cast<unsigned>(b[2 * i]) << 8) |
                static_cast<unsigned>(b[2 * i + 1]);
  }
  char text[40] = {};
  std::snprintf(text, sizeof(text), "%x:%x:%x:%x:%x:%x:%x:%x", groups[0],
                groups[1], groups[2], groups[3], groups[4], groups[5],
                groups[6], groups[7]);
  return text;
}

VOID WINAPI Browsed(DWORD status, PVOID, PDNS_RECORD records) {
  if (status != ERROR_SUCCESS) {
    if (records != nullptr) {
      ::DnsRecordListFree(records, DnsFreeRecordList);
    }
    if (status != ERROR_CANCELLED) {
      auto failed = std::make_unique<Found>();
      failed->kind = Found::Kind::kBrowseFailed;
      Post(std::move(failed));
    }
    return;
  }
  auto found = std::make_unique<Found>();
  bool named = false;
  for (PDNS_RECORD record = records; record != nullptr;
       record = record->pNext) {
    if (record->wType == DNS_TYPE_PTR && !named) {
      found->instance = InstanceOf(record->Data.PTR.pNameHost);
      // A record that lives no longer is a device saying goodbye.
      found->kind =
          record->dwTtl == 0 ? Found::Kind::kRemoved : Found::Kind::kAdded;
      named = true;
    } else if (record->wType == DNS_TYPE_TEXT) {
      const DNS_TXT_DATAW& text = record->Data.TXT;
      for (DWORD i = 0; i < text.dwStringCount; ++i) {
        TagIn(text.pStringArray[i], found->tag);
      }
    }
  }
  if (records != nullptr) {
    ::DnsRecordListFree(records, DnsFreeRecordList);
  }
  if (named && !found->instance.empty()) {
    Post(std::move(found));
  }
}

VOID WINAPI Resolved(DWORD status, PVOID, PDNS_SERVICE_INSTANCE service) {
  if (service == nullptr) {
    return;
  }
  auto found = std::make_unique<Found>();
  found->kind = Found::Kind::kResolved;
  found->instance = InstanceOf(service->pszInstanceName);
  if (status == ERROR_SUCCESS) {
    found->port = service->wPort;
    if (service->ip4Address != nullptr) {
      found->addresses.push_back(Ip4(service->ip4Address));
    }
    if (service->ip6Address != nullptr) {
      found->addresses.push_back(Ip6(service->ip6Address));
    }
    for (DWORD i = 0; i < service->dwPropertyCount; ++i) {
      if (service->keys == nullptr || service->values == nullptr ||
          service->keys[i] == nullptr || service->values[i] == nullptr) {
        continue;
      }
      if (std::wstring(service->keys[i]) == L"t") {
        found->tag = Utf8FromUtf16(service->values[i]);
      }
    }
  }
  ::DnsServiceFreeInstance(service);
  if (!found->instance.empty()) {
    Post(std::move(found));
  }
}

// Called when an announcement is in place or has been taken back. The
// announcements themselves are freed on the window's thread.
VOID WINAPI Registered(DWORD, PVOID, PDNS_SERVICE_INSTANCE) {}

std::wstring HostName() {
  wchar_t name[256] = {};
  DWORD size = static_cast<DWORD>(std::size(name));
  if (!::GetComputerNameExW(ComputerNameDnsHostname, name, &size)) {
    return std::wstring();
  }
  return std::wstring(name, size) + L".local";
}

}  // namespace

struct Bonjour::Registration {
  DNS_SERVICE_REGISTER_REQUEST request = {};
  PDNS_SERVICE_INSTANCE instance = nullptr;
  DNS_SERVICE_CANCEL cancel = {};

  ~Registration() {
    if (instance != nullptr) {
      ::DnsServiceFreeInstance(instance);
    }
  }
};

Bonjour::Bonjour(HWND window) : window_(window) { g_window.store(window); }

Bonjour::~Bonjour() {
  ::KillTimer(window_, kRetryTimer);
  StopBrowsing();
  StopAdvertising();
  g_window.store(nullptr);
  // The program is ending. Windows may still answer for the announcements
  // just taken back, so their memory is left to go with the program.
  for (auto& retired : retired_) {
    retired.release();
  }
}

void Bonjour::Browse() {
  if (browsing_) {
    return;
  }
  DNS_SERVICE_BROWSE_REQUEST request = {};
  request.Version = DNS_QUERY_REQUEST_VERSION1;
  request.InterfaceIndex = 0;
  request.QueryName = kBrowseName;
  request.pBrowseCallback = Browsed;
  request.pQueryContext = nullptr;
  browse_cancel_ = {};
  if (::DnsServiceBrowse(&request, &browse_cancel_) == DNS_REQUEST_PENDING) {
    browsing_ = true;
  } else {
    ::SetTimer(window_, kRetryTimer, kRetryMs, nullptr);
  }
}

bool Bonjour::HandleTimer(WPARAM id) {
  if (id != kRetryTimer) {
    return false;
  }
  ::KillTimer(window_, kRetryTimer);
  Browse();
  return true;
}

void Bonjour::StopBrowsing() {
  for (auto& [instance, cancel] : resolving_) {
    ::DnsServiceResolveCancel(cancel.get());
  }
  resolving_.clear();
  tags_.clear();
  if (browsing_) {
    ::DnsServiceBrowseCancel(&browse_cancel_);
    browsing_ = false;
  }
}

void Bonjour::Handle(LPARAM lparam) {
  std::unique_ptr<Found> found(reinterpret_cast<Found*>(lparam));
  if (!found) {
    return;
  }
  switch (found->kind) {
    case Found::Kind::kBrowseFailed:
      // Looking stops for good when it fails, such as while the network
      // changes. Whatever is still there is found again.
      StopBrowsing();
      ::SetTimer(window_, kRetryTimer, kRetryMs, nullptr);
      break;
    case Found::Kind::kAdded: {
      const bool known = tags_.count(found->instance) != 0;
      tags_[found->instance] = found->tag;
      if (!known || resolving_.count(found->instance) == 0) {
        Resolve(found->instance);
      }
      break;
    }
    case Found::Kind::kRemoved:
      StopResolving(found->instance);
      if (tags_.erase(found->instance) != 0) {
        CoreHost::Lost(found->instance);
      }
      break;
    case Found::Kind::kResolved: {
      if (resolving_.erase(found->instance) == 0) {
        break;
      }
      const auto seen = tags_.find(found->instance);
      if (seen == tags_.end() || found->addresses.empty()) {
        break;
      }
      const std::string& tag = found->tag.empty() ? seen->second : found->tag;
      CoreHost::Seen(found->instance, found->addresses, found->port, tag);
      break;
    }
  }
}

void Bonjour::Resolve(const std::string& instance) {
  StopResolving(instance);
  std::wstring name = Utf16(instance) + L"._zerohop._tcp.local";
  DNS_SERVICE_RESOLVE_REQUEST request = {};
  request.Version = DNS_QUERY_REQUEST_VERSION1;
  request.InterfaceIndex = 0;
  request.QueryName = name.data();
  request.pResolveCompletionCallback = Resolved;
  request.pQueryContext = nullptr;
  auto cancel = std::make_unique<DNS_SERVICE_CANCEL>();
  if (::DnsServiceResolve(&request, cancel.get()) == DNS_REQUEST_PENDING) {
    resolving_[instance] = std::move(cancel);
  }
}

void Bonjour::StopResolving(const std::string& instance) {
  const auto pending = resolving_.find(instance);
  if (pending == resolving_.end()) {
    return;
  }
  ::DnsServiceResolveCancel(pending->second.get());
  resolving_.erase(pending);
}

void Bonjour::Advertise(const char* instance, const char* tag, uint16_t port) {
  const std::string key =
      instance == nullptr || tag == nullptr
          ? std::string()
          : std::string(instance) + ":" + tag + ":" + std::to_string(port);
  if (key == advert_key_) {
    return;
  }
  advert_key_ = key;
  StopAdvertising();
  if (key.empty() || port == 0) {
    return;
  }
  const std::wstring name = Utf16(instance) + L"._zerohop._tcp.local";
  const std::wstring host = HostName();
  const std::wstring tag_text = Utf16(tag);
  if (host.empty() || tag_text.empty()) {
    return;
  }
  PCWSTR keys[] = {L"v", L"t"};
  PCWSTR values[] = {L"1", tag_text.c_str()};
  auto registration = std::make_unique<Registration>();
  registration->instance = ::DnsServiceConstructInstance(
      name.c_str(), host.c_str(), nullptr, nullptr, port, 0, 0, 2, keys,
      values);
  if (registration->instance == nullptr) {
    return;
  }
  DNS_SERVICE_REGISTER_REQUEST& request = registration->request;
  request.Version = DNS_QUERY_REQUEST_VERSION1;
  request.InterfaceIndex = 0;
  request.pServiceInstance = registration->instance;
  request.pRegisterCompletionCallback = Registered;
  request.pQueryContext = nullptr;
  request.unicastEnabled = FALSE;
  if (::DnsServiceRegister(&request, &registration->cancel) ==
      DNS_REQUEST_PENDING) {
    registration_ = std::move(registration);
  }
}

void Bonjour::StopAdvertising() {
  if (!registration_) {
    return;
  }
  ::DnsServiceDeRegister(&registration_->request, nullptr);
  retired_.push_back(std::move(registration_));
  while (retired_.size() > kRetiredKept) {
    retired_.pop_front();
  }
}
