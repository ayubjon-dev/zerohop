#ifndef RUNNER_FILE_PASTE_H_
#define RUNNER_FILE_PASTE_H_

#include <windows.h>

#include <cstdint>
#include <string>

// Files copied on another device, ready to paste here. Only their list has
// arrived: the clipboard holds a promise of them, and when the owner pastes,
// Explorer asks for the files and Zerohop fetches them right then.
//
// The clipboard object lives on a thread of its own, which also waits for the
// fetch, so a long paste never stops the window.
class FilePaste {
 public:
  // Told with kFailedMessage when a paste could not fetch the files.
  explicit FilePaste(HWND window);
  ~FilePaste();

  FilePaste(const FilePaste&) = delete;
  FilePaste& operator=(const FilePaste&) = delete;

  // Puts the promise of these files on the clipboard for |lifetime_ms|.
  // |clip_id| is the core's name for them.
  void Offer(const std::string& clip_id, int64_t lifetime_ms);

  // Posted to the window when a paste could not fetch the files.
  static constexpr UINT kFailedMessage = WM_APP + 3;

  // Clipboard contents carrying this format are files offered from another
  // device, not something copied here.
  static UINT MarkerFormat();

 private:
  static DWORD WINAPI Run(void* self);
  void Loop();

  HWND window_;
  HANDLE thread_ = nullptr;
  DWORD thread_id_ = 0;
  HANDLE ready_ = nullptr;
};

#endif  // RUNNER_FILE_PASTE_H_
