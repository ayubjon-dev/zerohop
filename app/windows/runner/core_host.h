#ifndef RUNNER_CORE_HOST_H_
#define RUNNER_CORE_HOST_H_

#include <windows.h>

#include <cstddef>
#include <cstdint>
#include <string>
#include <vector>

// The Rust core, run by this program itself so that it keeps working with no
// window open and no Flutter loaded. Its messages arrive on the core's own
// threads and are posted to the window given to Load as kMessage, to be read
// there with Take. The core reads and writes everything else as JSON itself,
// so nothing here parses what other devices sent.
class CoreHost {
 public:
  // What a message from the core is about. Every pointer lasts only for the
  // call.
  class Listener {
   public:
    virtual ~Listener() = default;
    // Whether the shared clipboard is on, whether a pairing is under way, and
    // what to announce: a null |instance| means nothing.
    virtual void OnSnapshot(bool clipboard, bool pairing, const char* instance,
                            const char* tag, uint16_t port) = 0;
    virtual void OnOfferArrived() = 0;
    virtual void OnClipText(const char* text, uint64_t ttl_ms,
                            const char* from) = 0;
    // PNG or JPEG bytes.
    virtual void OnClipPicture(const uint8_t* bytes, size_t size,
                               uint64_t ttl_ms, const char* from) = 0;
    virtual void OnClipFiles(const char* id, uint64_t ttl_ms,
                             const char* from) = 0;
  };

  enum class Started { kRunning, kKeyUnavailable, kFailed };

  // A message from the core, with a std::string* in lparam that Take frees.
  static constexpr UINT kMessage = WM_APP + 4;

  // Loads the core's library next to zerohop.exe. False when it is missing.
  static bool Load(HWND window);

  // Starts the core: opens the secret Windows keeps for sealing the device
  // key, then loads the key and starts listening. Waits for all of that, so
  // never on the window's thread.
  static Started Start(const std::wstring& state_dir,
                       const std::wstring& folder);

  // Reads a message posted as kMessage and makes the matching call. Returns
  // the message when the interface should hear it too, or an empty string.
  static std::string Take(LPARAM lparam, Listener& listener);

  // Copied on this computer, for the other devices.
  static void ClipText(const std::string& utf8);
  static void ClipPng(const std::vector<uint8_t>& png);
  static void ClipFiles(const std::vector<std::string>& paths);

  // A device seen on the network, and one no longer there.
  static void Seen(const std::string& instance,
                   const std::vector<std::string>& addresses, uint16_t port,
                   const std::string& tag);
  static void Lost(const std::string& instance);

  // Ends a pairing under way, as when the window that showed it closes.
  static void PairCancel();

  // A BMP file for the system's decoder from a clipboard bitmap, or nothing
  // when its header does not add up.
  static std::vector<uint8_t> BmpFromDib(const uint8_t* dib, size_t size);

  // A clipboard bitmap from rows of blue, green, red and alpha, top down.
  static std::vector<uint8_t> DibFromBgra(const std::vector<uint8_t>& pixels,
                                          uint32_t width, uint32_t height);
};

#endif  // RUNNER_CORE_HOST_H_
