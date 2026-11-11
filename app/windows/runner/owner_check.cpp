#include "owner_check.h"

#include <UserConsentVerifierInterop.h>
#include <roapi.h>
#include <windows.foundation.h>
#include <windows.security.credentials.ui.h>
#include <wrl/client.h>
#include <wrl/event.h>
#include <wrl/implements.h>
#include <wrl/wrappers/corewrappers.h>

namespace owner_check {

namespace {

namespace ui = ABI::Windows::Security::Credentials::UI;
using ABI::Windows::Foundation::AsyncStatus;
using ABI::Windows::Foundation::IAsyncOperation;
using ABI::Windows::Foundation::IAsyncOperationCompletedHandler;
using Microsoft::WRL::Callback;
using Microsoft::WRL::ClassicCom;
using Microsoft::WRL::ComPtr;
using Microsoft::WRL::FtmBase;
using Microsoft::WRL::Implements;
using Microsoft::WRL::RuntimeClassFlags;
using Microsoft::WRL::Wrappers::HStringReference;

using Operation = IAsyncOperation<ui::UserConsentVerificationResult>;
using Done = IAsyncOperationCompletedHandler<ui::UserConsentVerificationResult>;

enum Answer : WPARAM { kPassed = 1, kUnavailable = 2, kRefused = 3 };

Answer FromResult(ui::UserConsentVerificationResult result) {
  switch (result) {
    case ui::UserConsentVerificationResult_Verified:
      return kPassed;
    // No Windows Hello on this computer or for this account: nothing to
    // check with.
    case ui::UserConsentVerificationResult_DeviceNotPresent:
    case ui::UserConsentVerificationResult_NotConfiguredForUser:
    case ui::UserConsentVerificationResult_DisabledByPolicy:
      return kUnavailable;
    default:
      return kRefused;
  }
}

std::wstring Widen(const std::string& text) {
  if (text.empty()) {
    return std::wstring();
  }
  const int count = static_cast<int>(text.size());
  const int length =
      ::MultiByteToWideChar(CP_UTF8, 0, text.data(), count, nullptr, 0);
  if (length <= 0) {
    return std::wstring();
  }
  std::wstring wide(static_cast<size_t>(length), L'\0');
  ::MultiByteToWideChar(CP_UTF8, 0, text.data(), count, wide.data(), length);
  return wide;
}

}  // namespace

bool Start(HWND window, const std::string& reason) {
  std::wstring message = Widen(reason);
  // The reason completes a sentence on other systems; here it stands alone.
  if (!message.empty()) {
    ::CharUpperBuffW(message.data(), 1);
  }
  ComPtr<IUserConsentVerifierInterop> interop;
  HRESULT hr = ::RoGetActivationFactory(
      HStringReference(
          RuntimeClass_Windows_Security_Credentials_UI_UserConsentVerifier)
          .Get(),
      IID_PPV_ARGS(&interop));
  if (FAILED(hr)) {
    return false;
  }
  ComPtr<Operation> operation;
  hr = interop->RequestVerificationForWindowAsync(
      window,
      HStringReference(message.c_str(), static_cast<unsigned int>(message.size()))
          .Get(),
      IID_PPV_ARGS(&operation));
  if (FAILED(hr)) {
    return false;
  }
  // The answer may come on another thread, so it goes back as a message.
  auto done = Callback<Implements<RuntimeClassFlags<ClassicCom>, Done, FtmBase>>(
      [window](Operation* finished, AsyncStatus status) -> HRESULT {
        Answer answer = kRefused;
        ui::UserConsentVerificationResult result =
            ui::UserConsentVerificationResult_Canceled;
        if (status == AsyncStatus::Completed &&
            SUCCEEDED(finished->GetResults(&result))) {
          answer = FromResult(result);
        }
        ::PostMessageW(window, kAnswerMessage, static_cast<WPARAM>(answer), 0);
        return S_OK;
      });
  return done && SUCCEEDED(operation->put_Completed(done.Get()));
}

const char* AnswerName(WPARAM answer) {
  switch (answer) {
    case kPassed:
      return "passed";
    case kUnavailable:
      return "unavailable";
    default:
      return "refused";
  }
}

}  // namespace owner_check
