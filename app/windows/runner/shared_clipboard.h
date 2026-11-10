#ifndef RUNNER_SHARED_CLIPBOARD_H_
#define RUNNER_SHARED_CLIPBOARD_H_

#include <flutter/encodable_value.h>
#include <windows.h>

#include <functional>
#include <optional>
#include <string>

// Copying on this computer and pasting on another device, and the other way
// round. Windows reports every change; the contents are read only then.
// Contents stay in memory: nothing here writes them to disk or to a log.
class SharedClipboard {
 public:
  using Report =
      std::function<void(const std::string& method, flutter::EncodableMap)>;

  SharedClipboard(HWND window, Report report);
  ~SharedClipboard();

  SharedClipboard(const SharedClipboard&) = delete;
  SharedClipboard& operator=(const SharedClipboard&) = delete;

  void Watch(bool on);

  // What is on the clipboard now: a "state", and for text or a picture the
  // "format" with its "text" or "data".
  flutter::EncodableMap Read();

  // Puts what arrived on the clipboard for its lifetime.
  void Apply(const flutter::EncodableMap& clip);

  // Removes what arrived, unless the owner has copied something since.
  void ClearIfStillOurs();

  // True for the messages it handled.
  bool HandleMessage(UINT message, WPARAM wparam);

 private:
  void OnChange();
  bool MarkedNotForOthers();
  flutter::EncodableMap ReadText();
  flutter::EncodableMap ReadPicture(UINT format, const char* name);

  HWND window_;
  Report report_;
  UINT png_;
  UINT exclude_;
  UINT history_;
  UINT cloud_;
  UINT viewer_ignore_;
  bool watching_ = false;
  DWORD seen_ = 0;
  std::optional<DWORD> ours_;
};

#endif  // RUNNER_SHARED_CLIPBOARD_H_
