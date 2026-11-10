#ifndef RUNNER_BACKGROUND_MODE_H_
#define RUNNER_BACKGROUND_MODE_H_

#include <windows.h>
#include <shellapi.h>

#include <optional>
#include <string>

// Keeps the app working with its window closed, for as long as the owner has
// the shared clipboard turned on: an icon next to the clock, and a start when
// the owner signs in to Windows.
class BackgroundMode {
 public:
  // What the owner chose from the icon.
  enum class Action { kNone, kOpen, kQuit };

  // |can_start_at_sign_in| is false for test profiles, which Windows could not
  // start with the right settings.
  BackgroundMode(HWND window, const std::wstring& title,
                 bool can_start_at_sign_in);
  ~BackgroundMode();

  BackgroundMode(const BackgroundMode&) = delete;
  BackgroundMode& operator=(const BackgroundMode&) = delete;

  bool is_on() const { return on_; }

  void Set(bool on);

  // Shows a short note next to the icon, when the icon is there.
  void NotifyOffer();

  // Handles messages meant for the icon. Empty for any other message.
  std::optional<Action> HandleMessage(UINT message, WPARAM wparam,
                                      LPARAM lparam);

 private:
  NOTIFYICONDATAW IconData() const;
  void AddIcon();
  void RemoveIcon();
  Action ShowMenu(int x, int y);
  void SetStartsAtSignIn(bool on);

  HWND window_;
  std::wstring title_;
  bool can_start_at_sign_in_;
  HICON icon_ = nullptr;
  UINT taskbar_created_ = 0;
  bool on_ = false;
  bool icon_added_ = false;
};

#endif  // RUNNER_BACKGROUND_MODE_H_
