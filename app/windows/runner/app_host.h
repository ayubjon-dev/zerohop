#ifndef RUNNER_APP_HOST_H_
#define RUNNER_APP_HOST_H_

#include <flutter/dart_project.h>
#include <flutter/encodable_value.h>
#include <windows.h>

#include <cstdint>
#include <functional>
#include <memory>
#include <optional>
#include <string>
#include <vector>

#include "background_mode.h"
#include "bonjour.h"
#include "core_host.h"
#include "file_paste.h"
#include "flutter_window.h"
#include "shared_clipboard.h"

// How this copy of the app was started.
struct LaunchOptions {
  std::wstring title;
  // Empty for the owner's everyday copy, set for test copies side by side.
  std::wstring profile;
  // Started by Windows at sign-in: stay next to the clock.
  bool background = false;
  // Files handed over from Explorer's Send to menu.
  std::vector<std::wstring> paths;
};

// Zerohop itself: a window that is never shown, which keeps the core, finding
// devices, the shared clipboard, pasting files and the icon next to the clock.
// The interface, with all of Flutter, exists only while its window is open,
// so that none of it takes memory while Zerohop works out of sight.
//
// A second launch, Send to and the installer find Zerohop by this window.
class AppHost : public CoreHost::Listener {
 public:
  using Answer = std::function<void(flutter::EncodableMap)>;

  AppHost(const flutter::DartProject& project, LaunchOptions options);
  ~AppHost() override;

  AppHost(const AppHost&) = delete;
  AppHost& operator=(const AppHost&) = delete;

  // Makes the window and starts the core. False if the window could not be
  // made.
  bool Create();

  // Opens the interface, or brings it forward. |someone_waiting| also shows a
  // note next to the clock when Windows will not let it come forward.
  void OpenInterface(bool someone_waiting);

  // The interface joins the core, starting it first if needed, and hears
  // everything it says from then on.
  void Attach(Answer answer);
  // The interface can take files from Send to.
  void InterfaceReady();
  // The interface has drawn its first frame.
  void InterfaceDrawn();
  // The owner closed the interface's window.
  void InterfaceClosing();

  // CoreHost::Listener:
  void OnSnapshot(bool clipboard, bool pairing, const char* instance,
                  const char* tag, uint16_t port) override;
  void OnOfferArrived() override;
  void OnClipText(const char* text, uint64_t ttl_ms,
                  const char* from) override;
  void OnClipPicture(const uint8_t* bytes, size_t size, uint64_t ttl_ms,
                     const char* from) override;
  void OnClipFiles(const char* id, uint64_t ttl_ms, const char* from) override;

 private:
  static LRESULT CALLBACK WndProc(HWND window, UINT message, WPARAM wparam,
                                  LPARAM lparam) noexcept;
  LRESULT Handle(UINT message, WPARAM wparam, LPARAM lparam);

  void StartCore();
  void Started(CoreHost::Started result);
  flutter::EncodableMap AttachAnswer() const;

  // Something copied on this computer, as SharedClipboard read it.
  void Copied(const flutter::EncodableMap& clip);
  void Tell(const std::string& method, flutter::EncodableMap arguments);
  void DeliverPaths();
  void DropInterface();
  // Lets the interface go now, with its engine, while it is still whole.
  void CloseInterfaceWindow();
  void Quit();

  flutter::DartProject project_;
  LaunchOptions options_;
  HWND window_ = nullptr;

  std::unique_ptr<BackgroundMode> background_;
  std::unique_ptr<SharedClipboard> clipboard_;
  std::unique_ptr<FilePaste> file_paste_;
  std::unique_ptr<Bonjour> bonjour_;
  std::unique_ptr<FlutterWindow> interface_;

  std::optional<CoreHost::Started> started_;
  bool starting_ = false;
  std::vector<Answer> attaching_;
  // The interface has joined and hears the core.
  bool attached_ = false;
  bool interface_ready_ = false;
  // The interface's window closed and goes once the message loop is free.
  bool dropping_ = false;
  // The interface is being opened because someone waits for an answer.
  bool someone_waiting_ = false;

  std::optional<bool> clipboard_on_;
  bool pairing_ = false;
  std::vector<std::wstring> waiting_paths_;
  bool quitting_ = false;
};

#endif  // RUNNER_APP_HOST_H_
