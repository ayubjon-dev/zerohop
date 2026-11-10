// ignore: unused_import
import 'package:intl/intl.dart' as intl;
import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for English (`en`).
class L10nEn extends L10n {
  L10nEn([String locale = 'en']) : super(locale);

  @override
  String get onboardTitle => 'What should this device be called?';

  @override
  String get onboardHint => 'Only your own devices will see this name.';

  @override
  String get nameLabel => 'Device name';

  @override
  String get continueAction => 'Continue';

  @override
  String get myDevices => 'My devices';

  @override
  String get addDevice => 'Add device';

  @override
  String get add => 'Add';

  @override
  String get history => 'History';

  @override
  String get settings => 'Settings';

  @override
  String get devices => 'Devices';

  @override
  String get nearby => 'Nearby';

  @override
  String get offline => 'Offline';

  @override
  String get homeHint => 'Pick a device to send files to it';

  @override
  String get emptyTitle => 'This is your only device so far';

  @override
  String get emptyBody =>
      'Install Zerohop on another device and link it once. After that, files go directly between them, without the internet.';

  @override
  String get dropTitle => 'Drop files here';

  @override
  String dropHint(String name) {
    return 'Files go directly to $name, without the internet';
  }

  @override
  String dropHintOffline(String name) {
    return '$name is not on this network right now';
  }

  @override
  String get chooseFiles => 'Choose files';

  @override
  String get chooseFolder => 'Choose folder';

  @override
  String get addFiles => 'Add files';

  @override
  String get clear => 'Clear';

  @override
  String get send => 'Send';

  @override
  String sendTo(String name) {
    return 'Send to $name';
  }

  @override
  String removeItem(String name) {
    return 'Remove $name';
  }

  @override
  String get folderKind => 'Folder';

  @override
  String fileCount(int count) {
    String _temp0 = intl.Intl.pluralLogic(
      count,
      locale: localeName,
      other: '$count files',
      one: '$count file',
    );
    return '$_temp0';
  }

  @override
  String sentCount(int count) {
    String _temp0 = intl.Intl.pluralLogic(
      count,
      locale: localeName,
      other: '$count files sent',
      one: '$count file sent',
    );
    return '$_temp0';
  }

  @override
  String receivedCount(int count) {
    String _temp0 = intl.Intl.pluralLogic(
      count,
      locale: localeName,
      other: '$count files received',
      one: '$count file received',
    );
    return '$_temp0';
  }

  @override
  String sizeSummary(String count, String size) {
    return '$count, $size';
  }

  @override
  String get cancelTransfer => 'Cancel';

  @override
  String get done => 'Done';

  @override
  String doneSent(String summary, String name) {
    return '$summary to $name. Every file was verified.';
  }

  @override
  String doneReceived(String summary, String name) {
    return '$summary from $name. Every file was verified.';
  }

  @override
  String heldNote(int count) {
    String _temp0 = intl.Intl.pluralLogic(
      count,
      locale: localeName,
      other:
          '$count items look like programs and are waiting for you in History.',
      one: 'One item looks like a program and is waiting for you in History.',
    );
    return '$_temp0';
  }

  @override
  String heldNoteSent(int count) {
    String _temp0 = intl.Intl.pluralLogic(
      count,
      locale: localeName,
      other:
          '$count items look like programs and are waiting for a decision on the other device.',
      one:
          'One item looks like a program and is waiting for a decision on the other device.',
    );
    return '$_temp0';
  }

  @override
  String get sendMore => 'Send more';

  @override
  String get close => 'Close';

  @override
  String get resume => 'Continue';

  @override
  String get showInFolder => 'Show in folder';

  @override
  String progressOf(String done, String total) {
    return '$done of $total';
  }

  @override
  String perSecond(String size) {
    return '$size/s';
  }

  @override
  String sendingTo(String name) {
    return 'Sending to $name';
  }

  @override
  String receivingFrom(String name) {
    return 'Receiving from $name';
  }

  @override
  String get waitingAnswer => 'Waiting for the other device to accept';

  @override
  String get encryptedLine => 'Directly over your local network, encrypted';

  @override
  String incomingTitle(String name) {
    return '$name is sending files';
  }

  @override
  String get decline => 'Decline';

  @override
  String get accept => 'Accept';

  @override
  String programWarning(String name) {
    return 'This includes a program: $name.';
  }

  @override
  String get programAdvice => 'Accept only if you sent it yourself.';

  @override
  String get receiveFrom => 'Receiving from this device';

  @override
  String get receiving => 'Receiving files';

  @override
  String get modeAsk => 'Always ask';

  @override
  String get modeAskSub => 'Every transfer waits for your answer';

  @override
  String get modeSmart => 'Automatically, except programs';

  @override
  String get modeSmartSub => 'You are asked only when programs are included';

  @override
  String get modeAuto => 'Automatically';

  @override
  String get modeAutoSub => 'Everything is saved without asking';

  @override
  String get modeRefuse => 'Don\'t accept';

  @override
  String get modeRefuseSub => 'This device can\'t send files here';

  @override
  String get modeDefault => 'As in Settings';

  @override
  String deviceCode(String code) {
    return 'Device code $code';
  }

  @override
  String get removeDevice => 'Disconnect device';

  @override
  String removeTitle(String name) {
    return 'Disconnect $name?';
  }

  @override
  String get removeBody =>
      'It will no longer be able to exchange files with your other devices. Anything it already received stays on it.';

  @override
  String get removeAction => 'Disconnect';

  @override
  String get cancelAction => 'Cancel';

  @override
  String get back => 'Back';

  @override
  String get pairIntro =>
      'Open Zerohop on the other device. Link it once and it will see all your devices.';

  @override
  String get showCode => 'Show a code here';

  @override
  String get showCodeSub => 'Six digits to type on the other device';

  @override
  String get enterCode => 'Enter a code';

  @override
  String get enterCodeSub => 'If one is already showing on the other device';

  @override
  String get codeTitle => 'Linking code';

  @override
  String roomLine(String room, int seconds) {
    return 'Room $room. The code works once and expires in $seconds s.';
  }

  @override
  String get waitingForDevice => 'Waiting for the other device';

  @override
  String get enterTitle => 'Enter the code';

  @override
  String get enterLabel => 'Six digits from the other device\'s screen';

  @override
  String get roomsTitle => 'Found on this network';

  @override
  String get roomsSearching => 'Looking for a device showing a code';

  @override
  String room(String label) {
    return 'Room $label';
  }

  @override
  String get manualAddress => 'Type the address instead';

  @override
  String get addressLabel => 'Address, for example 192.168.1.20:53319';

  @override
  String get connecting => 'Connecting';

  @override
  String get compareTitle => 'Check the number';

  @override
  String get compareSub => 'It must be the same on both screens';

  @override
  String get otherDevice => 'Other device';

  @override
  String get numbersMatch => 'The numbers match';

  @override
  String get numbersDiffer => 'They don\'t match';

  @override
  String get finishing => 'Finishing the link';

  @override
  String pairedToast(String name) {
    return '$name added';
  }

  @override
  String get historyEmpty => 'Files you send and receive will appear here.';

  @override
  String toName(String name, String size) {
    return 'To $name, $size';
  }

  @override
  String fromName(String name, String size) {
    return 'From $name, $size';
  }

  @override
  String get historyDeclined => 'Transfer declined';

  @override
  String get historyCancelled => 'Transfer cancelled';

  @override
  String get historyInterrupted => 'Transfer interrupted';

  @override
  String get heldSection => 'Waiting for your decision';

  @override
  String get heldBody =>
      'These look like one kind of file but contain programs. If you do nothing, they are deleted after 24 hours.';

  @override
  String get letIn => 'Keep';

  @override
  String get delete => 'Delete';

  @override
  String heldFrom(String name, String size) {
    return 'From $name, $size';
  }

  @override
  String get unfinishedSection => 'Not finished';

  @override
  String get forget => 'Forget';

  @override
  String get clearHistory => 'Clear history';

  @override
  String get visible => 'Visible on the network';

  @override
  String get visibleSub => 'Your devices can find this one';

  @override
  String get folder => 'Folder for files';

  @override
  String get downloadsFolder => 'Downloads';

  @override
  String get filesAppFolder => 'Files › Zerohop';

  @override
  String get change => 'Change';

  @override
  String get open => 'Open';

  @override
  String get confirmNew => 'Approve new devices here';

  @override
  String get confirmNewSub =>
      'A device added through another one waits for your approval on this device';

  @override
  String get color => 'Color';

  @override
  String get cobalt => 'Cobalt';

  @override
  String get sunset => 'Sunset';

  @override
  String get mint => 'Mint';

  @override
  String get appearance => 'Appearance';

  @override
  String get themeSystem => 'System';

  @override
  String get themeLight => 'Light';

  @override
  String get themeDark => 'Dark';

  @override
  String get language => 'Language';

  @override
  String get langSystem => 'System';

  @override
  String get thisDevice => 'This device';

  @override
  String get startOver => 'Start over';

  @override
  String get startOverTitle => 'Start over on this device?';

  @override
  String get startOverBody =>
      'This device will leave your other devices and forget them. Files already received stay where they are.';

  @override
  String get removedBanner =>
      'This device was disconnected from your other devices.';

  @override
  String get folderProblem =>
      'The chosen folder is unavailable, so files are going to the default folder.';

  @override
  String newDeviceTitle(String name) {
    return 'New device: $name';
  }

  @override
  String get newDeviceBody =>
      'It was added through another of your devices. Allow it to send files here?';

  @override
  String get allow => 'Allow';

  @override
  String get errUnreachable =>
      'Can\'t reach this device. Check that Zerohop is open and both devices are on the same network.';

  @override
  String get errConnectionLost =>
      'The connection dropped. You can continue the transfer.';

  @override
  String get errDeclined => 'The other device declined the transfer.';

  @override
  String get errNoSpace => 'The other device doesn\'t have enough space.';

  @override
  String get errBusy => 'The other device is busy. Try again in a minute.';

  @override
  String get errInvalidName =>
      'One of the names can\'t be saved on the other device.';

  @override
  String get errCancelled => 'Transfer cancelled.';

  @override
  String get errCancelledByPeer =>
      'The transfer was cancelled on the other device.';

  @override
  String get errIntegrity => 'A file was damaged on the way. Send it again.';

  @override
  String get errSourceChanged =>
      'A file changed while it was being sent. Send it again.';

  @override
  String get errSourceUnreadable => 'A file couldn\'t be read.';

  @override
  String get errNothingToSend => 'There is nothing to send here.';

  @override
  String get errRemoved =>
      'This device was disconnected from your other devices.';

  @override
  String get errPeerRemoved => 'That device is no longer one of yours.';

  @override
  String get errCannotResume =>
      'This can no longer be continued. Send the files again.';

  @override
  String get errProtocol =>
      'The other device replied incorrectly. Update Zerohop on both devices.';

  @override
  String get errStorage => 'Files couldn\'t be written to the disk.';

  @override
  String get errPeerStorage => 'The other device couldn\'t save the files.';

  @override
  String get errAwaiting => 'Approve this device first.';

  @override
  String get errInUse => 'Zerohop is already open.';

  @override
  String get errWrongCode =>
      'That code didn\'t work. Show a new code and try again.';

  @override
  String get errTimeout => 'Time ran out. Show a new code.';

  @override
  String get errPairDeclined => 'The link was declined.';

  @override
  String get errPairUnreachable =>
      'Couldn\'t connect. Check that both devices are on the same network.';

  @override
  String get errPairFailed => 'Linking failed. Try again.';

  @override
  String get errAlreadyLinked =>
      'This device is already linked to others. Show a code here and enter it on the new device.';

  @override
  String get errBadAddress => 'That address doesn\'t look right.';

  @override
  String get errBadName => 'Enter a shorter name.';

  @override
  String get errFolder => 'That folder can\'t be used.';

  @override
  String get errGeneric => 'Something went wrong. Try again.';

  @override
  String get unitB => 'B';

  @override
  String get unitKB => 'KB';

  @override
  String get unitMB => 'MB';

  @override
  String get unitGB => 'GB';

  @override
  String get unitTB => 'TB';

  @override
  String get clipboard => 'Shared clipboard';

  @override
  String get clipboardSub =>
      'Copy on one device and paste on another. Passwords from password managers stay where they are.';

  @override
  String get clipboardOnDevice => 'What you copy here can be pasted there';

  @override
  String get clipboardOffDevice => 'Off for this device';

  @override
  String get clipboardOff => 'Off';

  @override
  String get clipboardOneTap => 'Send with one tap';

  @override
  String get clipboardSend => 'Send';

  @override
  String get clipboardEnable => 'Turn on';

  @override
  String get clipboardOptions => 'Options';

  @override
  String get clipTurnOnFirst =>
      'Turn on the shared clipboard in Settings first.';

  @override
  String get clipSendTo => 'Send to this device';

  @override
  String get clipReceiveFrom => 'Receive from this device';

  @override
  String get clipText => 'Text and links';

  @override
  String get clipImages => 'Pictures';

  @override
  String get clipAllow => 'Allow';

  @override
  String get clipAllowTitle => 'macOS permission';

  @override
  String get clipNeedsAllow =>
      'Let Zerohop read the clipboard: choose Allow in macOS settings';

  @override
  String get clipSent => 'Clipboard sent to nearby devices';

  @override
  String clipArrived(String name) {
    return 'Clipboard received: $name';
  }

  @override
  String get clipNoDevices => 'No nearby device can take the clipboard';

  @override
  String get clipFailed => 'Could not send the clipboard';

  @override
  String get clipEmpty => 'Copy some text or a picture first';

  @override
  String get clipSecret => 'This was copied as secret and is not sent';

  @override
  String get clipTooLarge => 'This is too large for the clipboard';

  @override
  String get clipSame => 'This already came from another device';

  @override
  String get clipDenied => 'The system did not allow reading the clipboard';

  @override
  String get clipTileTitle => 'Button in quick settings';

  @override
  String get clipTileSub =>
      'Send what you copied from quick settings or the notification, without opening the app.';

  @override
  String get clipTileAdd => 'Add';

  @override
  String get clipTileManual =>
      'Add the Send clipboard tile to quick settings by hand';

  @override
  String get clipMenuTitle => '“To Zerohop” entry';

  @override
  String get clipMenuSub =>
      'Adds it to the text selection menu, and text from Share goes straight to the clipboard. Any app on screen can use it too, so turn it on only if you trust your apps.';

  @override
  String get errKeyUnavailable =>
      'The system did not give access to this device\'s key. Allow access and try again.';

  @override
  String get errKeyLocked =>
      'This device\'s key can no longer be opened. To keep using the app, start over and link your devices again.';

  @override
  String get keyRetry => 'Try again';

  @override
  String get keyStartOver => 'Start over';

  @override
  String get shareNoDevices => 'Add a device first';

  @override
  String get shareTitle => 'Send to which device?';

  @override
  String shareMore(int count) {
    return 'and $count more';
  }

  @override
  String get shareChoose => 'Choose a device';

  @override
  String get shareAwaiting => 'Waiting for your approval';

  @override
  String get shareNobodyNearby =>
      'None of your devices is nearby right now. Open Zerohop on the one you are sending to and it will become available.';
}
