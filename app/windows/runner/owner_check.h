#ifndef RUNNER_OWNER_CHECK_H_
#define RUNNER_OWNER_CHECK_H_

#include <windows.h>

#include <string>

// The system's own check that the owner is present, with Windows Hello,
// before Zerohop signs a revocation (protocol section 5.3).
namespace owner_check {

// Posted to the window when the check ends, with the answer in wparam.
constexpr UINT kAnswerMessage = WM_APP + 2;

// Starts the check over |window|, saying |reason| (UTF-8). The answer arrives
// later as kAnswerMessage. False if it could not start at all.
bool Start(HWND window, const std::string& reason);

// The text the interface expects for an answer: "passed", "unavailable" or
// "refused".
const char* AnswerName(WPARAM answer);

}  // namespace owner_check

#endif  // RUNNER_OWNER_CHECK_H_
