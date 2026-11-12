#ifndef RUNNER_BONJOUR_H_
#define RUNNER_BONJOUR_H_

#include <windows.h>
#include <windns.h>

#include <cstdint>
#include <deque>
#include <map>
#include <memory>
#include <string>

// Finding Zerohop's devices through Windows' own DNS-SD service, and
// announcing this one. Whatever arrives is only a hint about which address to
// try: the core decides who is really there.
//
// Windows answers on threads of its own, which only post what they found to
// the window as kMessage. Everything else happens on the window's thread.
//
// Pairing rooms are announced and looked for by the interface, since pairing
// happens only while its window is open.
class Bonjour {
 public:
  // What Windows found, with a heap object in lparam that Handle frees.
  static constexpr UINT kMessage = WM_APP + 5;

  explicit Bonjour(HWND window);
  ~Bonjour();

  Bonjour(const Bonjour&) = delete;
  Bonjour& operator=(const Bonjour&) = delete;

  void Browse();

  // Announces this device, or stops when |instance| is null. The
  // announcement holds a random name and a rotating tag, never the device's
  // name.
  void Advertise(const char* instance, const char* tag, uint16_t port);

  // Handles a kMessage.
  void Handle(LPARAM lparam);

  // True for the timer it set, which tries looking again.
  bool HandleTimer(WPARAM id);

 private:
  struct Registration;

  void Resolve(const std::string& instance);
  void StopResolving(const std::string& instance);
  void StopBrowsing();
  void StopAdvertising();

  HWND window_;
  bool browsing_ = false;
  DNS_SERVICE_CANCEL browse_cancel_ = {};
  // What was seen while browsing: each instance's tag.
  std::map<std::string, std::string> tags_;
  std::map<std::string, std::unique_ptr<DNS_SERVICE_CANCEL>> resolving_;
  std::unique_ptr<Registration> registration_;
  // Announcements taken back. Windows may still answer for one for a while,
  // so each is kept until a few newer ones have come and gone.
  std::deque<std::unique_ptr<Registration>> retired_;
  std::string advert_key_;
};

#endif  // RUNNER_BONJOUR_H_
