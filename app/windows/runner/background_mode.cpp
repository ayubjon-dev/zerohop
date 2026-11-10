#include "background_mode.h"

#include <cwchar>

#include "resource.h"
#include "utils.h"

namespace {

// Clicks on the icon arrive as this window message.
constexpr UINT kIconMessage = WM_APP + 1;
constexpr UINT kIconId = 1;
constexpr int kOpenCommand = 1;
constexpr int kQuitCommand = 2;

constexpr wchar_t kRunKey[] =
    L"Software\\Microsoft\\Windows\\CurrentVersion\\Run";
constexpr wchar_t kRunValue[] = L"Zerohop";

}  // namespace

BackgroundMode::BackgroundMode(HWND window, const std::wstring& title,
                               bool can_start_at_sign_in)
    : window_(window),
      title_(title),
      can_start_at_sign_in_(can_start_at_sign_in) {
  icon_ = static_cast<HICON>(::LoadImageW(
      ::GetModuleHandleW(nullptr), MAKEINTRESOURCEW(IDI_APP_ICON), IMAGE_ICON,
      ::GetSystemMetrics(SM_CXSMICON), ::GetSystemMetrics(SM_CYSMICON),
      LR_DEFAULTCOLOR));
  // Explorer sends this when it starts again, and every icon has to be added
  // once more.
  taskbar_created_ = ::RegisterWindowMessageW(L"TaskbarCreated");
}

BackgroundMode::~BackgroundMode() {
  RemoveIcon();
  if (icon_ != nullptr) {
    ::DestroyIcon(icon_);
  }
}

void BackgroundMode::Set(bool on) {
  if (on == on_) {
    return;
  }
  on_ = on;
  if (on) {
    AddIcon();
  } else {
    RemoveIcon();
  }
  SetStartsAtSignIn(on);
}

void BackgroundMode::NotifyOffer() {
  if (!icon_added_) {
    return;
  }
  NOTIFYICONDATAW data = IconData();
  data.uFlags = NIF_INFO;
  data.dwInfoFlags = NIIF_INFO | NIIF_RESPECT_QUIET_TIME;
  wcsncpy_s(data.szInfoTitle, title_.c_str(), _TRUNCATE);
  wcsncpy_s(data.szInfo,
            PrefersRussian()
                ? L"Устройство хочет отправить вам файлы. Откройте Zerohop, "
                  L"чтобы ответить."
                : L"A device wants to send you files. Open Zerohop to answer.",
            _TRUNCATE);
  ::Shell_NotifyIconW(NIM_MODIFY, &data);
}

std::optional<BackgroundMode::Action> BackgroundMode::HandleMessage(
    UINT message, WPARAM wparam, LPARAM lparam) {
  if (taskbar_created_ != 0 && message == taskbar_created_) {
    if (on_) {
      AddIcon();
    }
    return Action::kNone;
  }
  if (message != kIconMessage) {
    return std::nullopt;
  }
  switch (LOWORD(lparam)) {
    case NIN_SELECT:
    case NIN_KEYSELECT:
    case NIN_BALLOONUSERCLICK:
      return Action::kOpen;
    case WM_CONTEXTMENU:
      return ShowMenu(static_cast<short>(LOWORD(wparam)),
                      static_cast<short>(HIWORD(wparam)));
    default:
      return Action::kNone;
  }
}

NOTIFYICONDATAW BackgroundMode::IconData() const {
  NOTIFYICONDATAW data = {};
  data.cbSize = sizeof(data);
  data.hWnd = window_;
  data.uID = kIconId;
  return data;
}

void BackgroundMode::AddIcon() {
  NOTIFYICONDATAW data = IconData();
  data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP;
  data.uCallbackMessage = kIconMessage;
  data.hIcon = icon_;
  wcsncpy_s(data.szTip, title_.c_str(), _TRUNCATE);
  // Right after sign-in the taskbar may not be there yet. It announces itself
  // with TaskbarCreated, and the icon is added then.
  icon_added_ = ::Shell_NotifyIconW(NIM_ADD, &data) != FALSE;
  if (icon_added_) {
    data.uVersion = NOTIFYICON_VERSION_4;
    ::Shell_NotifyIconW(NIM_SETVERSION, &data);
  }
}

void BackgroundMode::RemoveIcon() {
  if (!icon_added_) {
    return;
  }
  NOTIFYICONDATAW data = IconData();
  ::Shell_NotifyIconW(NIM_DELETE, &data);
  icon_added_ = false;
}

BackgroundMode::Action BackgroundMode::ShowMenu(int x, int y) {
  HMENU menu = ::CreatePopupMenu();
  if (menu == nullptr) {
    return Action::kNone;
  }
  const bool russian = PrefersRussian();
  ::AppendMenuW(menu, MF_STRING, static_cast<UINT_PTR>(kOpenCommand),
                russian ? L"Открыть Zerohop" : L"Open Zerohop");
  ::AppendMenuW(menu, MF_SEPARATOR, 0, nullptr);
  ::AppendMenuW(menu, MF_STRING, static_cast<UINT_PTR>(kQuitCommand),
                russian ? L"Выйти из Zerohop" : L"Quit Zerohop");
  ::SetMenuDefaultItem(menu, static_cast<UINT>(kOpenCommand), FALSE);
  // Without this the menu stays open when the owner clicks somewhere else.
  ::SetForegroundWindow(window_);
  const UINT align = ::GetSystemMetrics(SM_MENUDROPALIGNMENT) != 0
                         ? static_cast<UINT>(TPM_RIGHTALIGN)
                         : static_cast<UINT>(TPM_LEFTALIGN);
  const int chosen = ::TrackPopupMenuEx(
      menu,
      TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | align,
      x, y, window_, nullptr);
  ::PostMessageW(window_, WM_NULL, 0, 0);
  ::DestroyMenu(menu);
  if (chosen == kOpenCommand) {
    return Action::kOpen;
  }
  if (chosen == kQuitCommand) {
    return Action::kQuit;
  }
  return Action::kNone;
}

void BackgroundMode::SetStartsAtSignIn(bool on) {
  if (!can_start_at_sign_in_) {
    return;
  }
  if (!on) {
    ::RegDeleteKeyValueW(HKEY_CURRENT_USER, kRunKey, kRunValue);
    return;
  }
  const std::wstring path = ExecutablePath();
  if (path.empty()) {
    return;
  }
  // Started this way, the app stays next to the clock until it is opened.
  const std::wstring command = L"\"" + path + L"\" --background";
  ::RegSetKeyValueW(
      HKEY_CURRENT_USER, kRunKey, kRunValue, REG_SZ, command.c_str(),
      static_cast<DWORD>((command.size() + 1) * sizeof(wchar_t)));
}
