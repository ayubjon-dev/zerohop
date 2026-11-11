import Cocoa
import FlutterMacOS

@main
class AppDelegate: FlutterAppDelegate {
  override func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
    return !BackgroundMode.shared.isOn
  }

  override func applicationShouldHandleReopen(
    _ sender: NSApplication, hasVisibleWindows flag: Bool
  ) -> Bool {
    BackgroundMode.shared.showWindow()
    return true
  }

  override func applicationDidFinishLaunching(_ notification: Notification) {
    // Started by the system at login: stay out of the way.
    let event = NSAppleEventManager.shared().currentAppleEvent
    let atLogin =
      event?.eventID == kAEOpenApplication
      && event?.paramDescriptor(forKeyword: keyAEPropData)?.enumCodeValue == keyAELaunchedAsLogInItem
    BackgroundMode.shared.start(visible: !atLogin)
    super.applicationDidFinishLaunching(notification)
  }

  override func applicationSupportsSecureRestorableState(_ app: NSApplication) -> Bool {
    return true
  }
}
