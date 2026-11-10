import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_localizations/flutter_localizations.dart';
import 'package:intl/intl.dart' as intl;

import 'app_localizations_en.dart';
import 'app_localizations_ru.dart';

// ignore_for_file: type=lint

/// Callers can lookup localized strings with an instance of L10n
/// returned by `L10n.of(context)`.
///
/// Applications need to include `L10n.delegate()` in their app's
/// `localizationDelegates` list, and the locales they support in the app's
/// `supportedLocales` list. For example:
///
/// ```dart
/// import 'l10n/app_localizations.dart';
///
/// return MaterialApp(
///   localizationsDelegates: L10n.localizationsDelegates,
///   supportedLocales: L10n.supportedLocales,
///   home: MyApplicationHome(),
/// );
/// ```
///
/// ## Update pubspec.yaml
///
/// Please make sure to update your pubspec.yaml to include the following
/// packages:
///
/// ```yaml
/// dependencies:
///   # Internationalization support.
///   flutter_localizations:
///     sdk: flutter
///   intl: any # Use the pinned version from flutter_localizations
///
///   # Rest of dependencies
/// ```
///
/// ## iOS Applications
///
/// iOS applications define key application metadata, including supported
/// locales, in an Info.plist file that is built into the application bundle.
/// To configure the locales supported by your app, you’ll need to edit this
/// file.
///
/// First, open your project’s ios/Runner.xcworkspace Xcode workspace file.
/// Then, in the Project Navigator, open the Info.plist file under the Runner
/// project’s Runner folder.
///
/// Next, select the Information Property List item, select Add Item from the
/// Editor menu, then select Localizations from the pop-up menu.
///
/// Select and expand the newly-created Localizations item then, for each
/// locale your application supports, add a new item and select the locale
/// you wish to add from the pop-up menu in the Value field. This list should
/// be consistent with the languages listed in the L10n.supportedLocales
/// property.
abstract class L10n {
  L10n(String locale)
    : localeName = intl.Intl.canonicalizedLocale(locale.toString());

  final String localeName;

  static L10n of(BuildContext context) {
    return Localizations.of<L10n>(context, L10n)!;
  }

  static const LocalizationsDelegate<L10n> delegate = _L10nDelegate();

  /// A list of this localizations delegate along with the default localizations
  /// delegates.
  ///
  /// Returns a list of localizations delegates containing this delegate along with
  /// GlobalMaterialLocalizations.delegate, GlobalCupertinoLocalizations.delegate,
  /// and GlobalWidgetsLocalizations.delegate.
  ///
  /// Additional delegates can be added by appending to this list in
  /// MaterialApp. This list does not have to be used at all if a custom list
  /// of delegates is preferred or required.
  static const List<LocalizationsDelegate<dynamic>> localizationsDelegates =
      <LocalizationsDelegate<dynamic>>[
        delegate,
        GlobalMaterialLocalizations.delegate,
        GlobalCupertinoLocalizations.delegate,
        GlobalWidgetsLocalizations.delegate,
      ];

  /// A list of this localizations delegate's supported locales.
  static const List<Locale> supportedLocales = <Locale>[
    Locale('en'),
    Locale('ru'),
  ];

  /// No description provided for @onboardTitle.
  ///
  /// In en, this message translates to:
  /// **'What should this device be called?'**
  String get onboardTitle;

  /// No description provided for @onboardHint.
  ///
  /// In en, this message translates to:
  /// **'Only your own devices will see this name.'**
  String get onboardHint;

  /// No description provided for @nameLabel.
  ///
  /// In en, this message translates to:
  /// **'Device name'**
  String get nameLabel;

  /// No description provided for @continueAction.
  ///
  /// In en, this message translates to:
  /// **'Continue'**
  String get continueAction;

  /// No description provided for @myDevices.
  ///
  /// In en, this message translates to:
  /// **'My devices'**
  String get myDevices;

  /// No description provided for @addDevice.
  ///
  /// In en, this message translates to:
  /// **'Add device'**
  String get addDevice;

  /// No description provided for @add.
  ///
  /// In en, this message translates to:
  /// **'Add'**
  String get add;

  /// No description provided for @history.
  ///
  /// In en, this message translates to:
  /// **'History'**
  String get history;

  /// No description provided for @settings.
  ///
  /// In en, this message translates to:
  /// **'Settings'**
  String get settings;

  /// No description provided for @devices.
  ///
  /// In en, this message translates to:
  /// **'Devices'**
  String get devices;

  /// No description provided for @nearby.
  ///
  /// In en, this message translates to:
  /// **'Nearby'**
  String get nearby;

  /// No description provided for @offline.
  ///
  /// In en, this message translates to:
  /// **'Offline'**
  String get offline;

  /// No description provided for @homeHint.
  ///
  /// In en, this message translates to:
  /// **'Pick a device to send files to it'**
  String get homeHint;

  /// No description provided for @emptyTitle.
  ///
  /// In en, this message translates to:
  /// **'This is your only device so far'**
  String get emptyTitle;

  /// No description provided for @emptyBody.
  ///
  /// In en, this message translates to:
  /// **'Install Zerohop on another device and link it once. After that, files go directly between them, without the internet.'**
  String get emptyBody;

  /// No description provided for @dropTitle.
  ///
  /// In en, this message translates to:
  /// **'Drop files here'**
  String get dropTitle;

  /// No description provided for @dropHint.
  ///
  /// In en, this message translates to:
  /// **'Files go directly to {name}, without the internet'**
  String dropHint(String name);

  /// No description provided for @dropHintOffline.
  ///
  /// In en, this message translates to:
  /// **'{name} is not on this network right now'**
  String dropHintOffline(String name);

  /// No description provided for @chooseFiles.
  ///
  /// In en, this message translates to:
  /// **'Choose files'**
  String get chooseFiles;

  /// No description provided for @chooseFolder.
  ///
  /// In en, this message translates to:
  /// **'Choose folder'**
  String get chooseFolder;

  /// No description provided for @addFiles.
  ///
  /// In en, this message translates to:
  /// **'Add files'**
  String get addFiles;

  /// No description provided for @addFilesShort.
  ///
  /// In en, this message translates to:
  /// **'Files'**
  String get addFilesShort;

  /// No description provided for @choosePhotos.
  ///
  /// In en, this message translates to:
  /// **'Photos'**
  String get choosePhotos;

  /// No description provided for @photosLoading.
  ///
  /// In en, this message translates to:
  /// **'Loading…'**
  String get photosLoading;

  /// No description provided for @photosFailed.
  ///
  /// In en, this message translates to:
  /// **'Some photos or videos could not be loaded and were not added'**
  String get photosFailed;

  /// No description provided for @clear.
  ///
  /// In en, this message translates to:
  /// **'Clear'**
  String get clear;

  /// No description provided for @send.
  ///
  /// In en, this message translates to:
  /// **'Send'**
  String get send;

  /// No description provided for @sendTo.
  ///
  /// In en, this message translates to:
  /// **'Send to {name}'**
  String sendTo(String name);

  /// No description provided for @removeItem.
  ///
  /// In en, this message translates to:
  /// **'Remove {name}'**
  String removeItem(String name);

  /// No description provided for @folderKind.
  ///
  /// In en, this message translates to:
  /// **'Folder'**
  String get folderKind;

  /// No description provided for @fileCount.
  ///
  /// In en, this message translates to:
  /// **'{count, plural, =1{{count} file} other{{count} files}}'**
  String fileCount(int count);

  /// No description provided for @sentCount.
  ///
  /// In en, this message translates to:
  /// **'{count, plural, =1{{count} file sent} other{{count} files sent}}'**
  String sentCount(int count);

  /// No description provided for @receivedCount.
  ///
  /// In en, this message translates to:
  /// **'{count, plural, =1{{count} file received} other{{count} files received}}'**
  String receivedCount(int count);

  /// No description provided for @sizeSummary.
  ///
  /// In en, this message translates to:
  /// **'{count}, {size}'**
  String sizeSummary(String count, String size);

  /// No description provided for @cancelTransfer.
  ///
  /// In en, this message translates to:
  /// **'Cancel'**
  String get cancelTransfer;

  /// No description provided for @done.
  ///
  /// In en, this message translates to:
  /// **'Done'**
  String get done;

  /// No description provided for @doneSent.
  ///
  /// In en, this message translates to:
  /// **'{summary} to {name}. Every file was verified.'**
  String doneSent(String summary, String name);

  /// No description provided for @doneReceived.
  ///
  /// In en, this message translates to:
  /// **'{summary} from {name}. Every file was verified.'**
  String doneReceived(String summary, String name);

  /// No description provided for @heldNote.
  ///
  /// In en, this message translates to:
  /// **'{count, plural, =1{One item looks like a program and is waiting for you in History.} other{{count} items look like programs and are waiting for you in History.}}'**
  String heldNote(int count);

  /// No description provided for @heldNoteSent.
  ///
  /// In en, this message translates to:
  /// **'{count, plural, =1{One item looks like a program and is waiting for a decision on the other device.} other{{count} items look like programs and are waiting for a decision on the other device.}}'**
  String heldNoteSent(int count);

  /// No description provided for @sendMore.
  ///
  /// In en, this message translates to:
  /// **'Send more'**
  String get sendMore;

  /// No description provided for @close.
  ///
  /// In en, this message translates to:
  /// **'Close'**
  String get close;

  /// No description provided for @resume.
  ///
  /// In en, this message translates to:
  /// **'Continue'**
  String get resume;

  /// No description provided for @showInFolder.
  ///
  /// In en, this message translates to:
  /// **'Show in folder'**
  String get showInFolder;

  /// No description provided for @progressOf.
  ///
  /// In en, this message translates to:
  /// **'{done} of {total}'**
  String progressOf(String done, String total);

  /// No description provided for @perSecond.
  ///
  /// In en, this message translates to:
  /// **'{size}/s'**
  String perSecond(String size);

  /// No description provided for @sendingTo.
  ///
  /// In en, this message translates to:
  /// **'Sending to {name}'**
  String sendingTo(String name);

  /// No description provided for @receivingFrom.
  ///
  /// In en, this message translates to:
  /// **'Receiving from {name}'**
  String receivingFrom(String name);

  /// No description provided for @waitingAnswer.
  ///
  /// In en, this message translates to:
  /// **'Waiting for the other device to accept'**
  String get waitingAnswer;

  /// No description provided for @encryptedLine.
  ///
  /// In en, this message translates to:
  /// **'Directly over your local network, encrypted'**
  String get encryptedLine;

  /// No description provided for @incomingTitle.
  ///
  /// In en, this message translates to:
  /// **'{name} is sending files'**
  String incomingTitle(String name);

  /// No description provided for @decline.
  ///
  /// In en, this message translates to:
  /// **'Decline'**
  String get decline;

  /// No description provided for @accept.
  ///
  /// In en, this message translates to:
  /// **'Accept'**
  String get accept;

  /// No description provided for @programWarning.
  ///
  /// In en, this message translates to:
  /// **'This includes a program: {name}.'**
  String programWarning(String name);

  /// No description provided for @programAdvice.
  ///
  /// In en, this message translates to:
  /// **'Accept only if you sent it yourself.'**
  String get programAdvice;

  /// No description provided for @receiveFrom.
  ///
  /// In en, this message translates to:
  /// **'Receiving from this device'**
  String get receiveFrom;

  /// No description provided for @receiving.
  ///
  /// In en, this message translates to:
  /// **'Receiving files'**
  String get receiving;

  /// No description provided for @modeAsk.
  ///
  /// In en, this message translates to:
  /// **'Always ask'**
  String get modeAsk;

  /// No description provided for @modeAskSub.
  ///
  /// In en, this message translates to:
  /// **'Every transfer waits for your answer'**
  String get modeAskSub;

  /// No description provided for @modeSmart.
  ///
  /// In en, this message translates to:
  /// **'Automatically, except programs'**
  String get modeSmart;

  /// No description provided for @modeSmartSub.
  ///
  /// In en, this message translates to:
  /// **'You are asked only when programs are included'**
  String get modeSmartSub;

  /// No description provided for @modeAuto.
  ///
  /// In en, this message translates to:
  /// **'Automatically'**
  String get modeAuto;

  /// No description provided for @modeAutoSub.
  ///
  /// In en, this message translates to:
  /// **'Everything is saved without asking'**
  String get modeAutoSub;

  /// No description provided for @modeRefuse.
  ///
  /// In en, this message translates to:
  /// **'Don\'t accept'**
  String get modeRefuse;

  /// No description provided for @modeRefuseSub.
  ///
  /// In en, this message translates to:
  /// **'This device can\'t send files here'**
  String get modeRefuseSub;

  /// No description provided for @modeDefault.
  ///
  /// In en, this message translates to:
  /// **'As in Settings'**
  String get modeDefault;

  /// No description provided for @deviceCode.
  ///
  /// In en, this message translates to:
  /// **'Device code {code}'**
  String deviceCode(String code);

  /// No description provided for @removeDevice.
  ///
  /// In en, this message translates to:
  /// **'Disconnect device'**
  String get removeDevice;

  /// No description provided for @removeTitle.
  ///
  /// In en, this message translates to:
  /// **'Disconnect {name}?'**
  String removeTitle(String name);

  /// No description provided for @removeBody.
  ///
  /// In en, this message translates to:
  /// **'It will no longer be able to exchange files with your other devices. Anything it already received stays on it.'**
  String get removeBody;

  /// No description provided for @removeAction.
  ///
  /// In en, this message translates to:
  /// **'Disconnect'**
  String get removeAction;

  /// No description provided for @cancelAction.
  ///
  /// In en, this message translates to:
  /// **'Cancel'**
  String get cancelAction;

  /// No description provided for @back.
  ///
  /// In en, this message translates to:
  /// **'Back'**
  String get back;

  /// No description provided for @pairIntro.
  ///
  /// In en, this message translates to:
  /// **'Open Zerohop on the other device. Link it once and it will see all your devices.'**
  String get pairIntro;

  /// No description provided for @showCode.
  ///
  /// In en, this message translates to:
  /// **'Show a code here'**
  String get showCode;

  /// No description provided for @showCodeSub.
  ///
  /// In en, this message translates to:
  /// **'Six digits to type on the other device'**
  String get showCodeSub;

  /// No description provided for @enterCode.
  ///
  /// In en, this message translates to:
  /// **'Enter a code'**
  String get enterCode;

  /// No description provided for @enterCodeSub.
  ///
  /// In en, this message translates to:
  /// **'If one is already showing on the other device'**
  String get enterCodeSub;

  /// No description provided for @codeTitle.
  ///
  /// In en, this message translates to:
  /// **'Linking code'**
  String get codeTitle;

  /// No description provided for @roomLine.
  ///
  /// In en, this message translates to:
  /// **'Room {room}. The code works once and expires in {seconds} s.'**
  String roomLine(String room, int seconds);

  /// No description provided for @waitingForDevice.
  ///
  /// In en, this message translates to:
  /// **'Waiting for the other device'**
  String get waitingForDevice;

  /// No description provided for @enterTitle.
  ///
  /// In en, this message translates to:
  /// **'Enter the code'**
  String get enterTitle;

  /// No description provided for @enterLabel.
  ///
  /// In en, this message translates to:
  /// **'Six digits from the other device\'s screen'**
  String get enterLabel;

  /// No description provided for @roomsTitle.
  ///
  /// In en, this message translates to:
  /// **'Found on this network'**
  String get roomsTitle;

  /// No description provided for @roomsSearching.
  ///
  /// In en, this message translates to:
  /// **'Looking for a device showing a code'**
  String get roomsSearching;

  /// No description provided for @room.
  ///
  /// In en, this message translates to:
  /// **'Room {label}'**
  String room(String label);

  /// No description provided for @manualAddress.
  ///
  /// In en, this message translates to:
  /// **'Type the address instead'**
  String get manualAddress;

  /// No description provided for @addressLabel.
  ///
  /// In en, this message translates to:
  /// **'Address, for example 192.168.1.20:53319'**
  String get addressLabel;

  /// No description provided for @connecting.
  ///
  /// In en, this message translates to:
  /// **'Connecting'**
  String get connecting;

  /// No description provided for @compareTitle.
  ///
  /// In en, this message translates to:
  /// **'Check the number'**
  String get compareTitle;

  /// No description provided for @compareSub.
  ///
  /// In en, this message translates to:
  /// **'It must be the same on both screens'**
  String get compareSub;

  /// No description provided for @otherDevice.
  ///
  /// In en, this message translates to:
  /// **'Other device'**
  String get otherDevice;

  /// No description provided for @numbersMatch.
  ///
  /// In en, this message translates to:
  /// **'The numbers match'**
  String get numbersMatch;

  /// No description provided for @numbersDiffer.
  ///
  /// In en, this message translates to:
  /// **'They don\'t match'**
  String get numbersDiffer;

  /// No description provided for @finishing.
  ///
  /// In en, this message translates to:
  /// **'Finishing the link'**
  String get finishing;

  /// No description provided for @pairedToast.
  ///
  /// In en, this message translates to:
  /// **'{name} added'**
  String pairedToast(String name);

  /// No description provided for @historyEmpty.
  ///
  /// In en, this message translates to:
  /// **'Files you send and receive will appear here.'**
  String get historyEmpty;

  /// No description provided for @toName.
  ///
  /// In en, this message translates to:
  /// **'To {name}, {size}'**
  String toName(String name, String size);

  /// No description provided for @fromName.
  ///
  /// In en, this message translates to:
  /// **'From {name}, {size}'**
  String fromName(String name, String size);

  /// No description provided for @historyDeclined.
  ///
  /// In en, this message translates to:
  /// **'Transfer declined'**
  String get historyDeclined;

  /// No description provided for @historyCancelled.
  ///
  /// In en, this message translates to:
  /// **'Transfer cancelled'**
  String get historyCancelled;

  /// No description provided for @historyInterrupted.
  ///
  /// In en, this message translates to:
  /// **'Transfer interrupted'**
  String get historyInterrupted;

  /// No description provided for @heldSection.
  ///
  /// In en, this message translates to:
  /// **'Waiting for your decision'**
  String get heldSection;

  /// No description provided for @heldBody.
  ///
  /// In en, this message translates to:
  /// **'These look like one kind of file but contain programs. If you do nothing, they are deleted after 24 hours.'**
  String get heldBody;

  /// No description provided for @letIn.
  ///
  /// In en, this message translates to:
  /// **'Keep'**
  String get letIn;

  /// No description provided for @delete.
  ///
  /// In en, this message translates to:
  /// **'Delete'**
  String get delete;

  /// No description provided for @heldFrom.
  ///
  /// In en, this message translates to:
  /// **'From {name}, {size}'**
  String heldFrom(String name, String size);

  /// No description provided for @unfinishedSection.
  ///
  /// In en, this message translates to:
  /// **'Not finished'**
  String get unfinishedSection;

  /// No description provided for @forget.
  ///
  /// In en, this message translates to:
  /// **'Forget'**
  String get forget;

  /// No description provided for @clearHistory.
  ///
  /// In en, this message translates to:
  /// **'Clear history'**
  String get clearHistory;

  /// No description provided for @visible.
  ///
  /// In en, this message translates to:
  /// **'Visible on the network'**
  String get visible;

  /// No description provided for @visibleSub.
  ///
  /// In en, this message translates to:
  /// **'Your devices can find this one'**
  String get visibleSub;

  /// No description provided for @folder.
  ///
  /// In en, this message translates to:
  /// **'Folder for files'**
  String get folder;

  /// No description provided for @downloadsFolder.
  ///
  /// In en, this message translates to:
  /// **'Downloads'**
  String get downloadsFolder;

  /// No description provided for @filesAppFolder.
  ///
  /// In en, this message translates to:
  /// **'Files › Zerohop'**
  String get filesAppFolder;

  /// No description provided for @change.
  ///
  /// In en, this message translates to:
  /// **'Change'**
  String get change;

  /// No description provided for @open.
  ///
  /// In en, this message translates to:
  /// **'Open'**
  String get open;

  /// No description provided for @confirmNew.
  ///
  /// In en, this message translates to:
  /// **'Approve new devices here'**
  String get confirmNew;

  /// No description provided for @confirmNewSub.
  ///
  /// In en, this message translates to:
  /// **'A device added through another one waits for your approval on this device'**
  String get confirmNewSub;

  /// No description provided for @color.
  ///
  /// In en, this message translates to:
  /// **'Color'**
  String get color;

  /// No description provided for @cobalt.
  ///
  /// In en, this message translates to:
  /// **'Cobalt'**
  String get cobalt;

  /// No description provided for @sunset.
  ///
  /// In en, this message translates to:
  /// **'Sunset'**
  String get sunset;

  /// No description provided for @mint.
  ///
  /// In en, this message translates to:
  /// **'Mint'**
  String get mint;

  /// No description provided for @appearance.
  ///
  /// In en, this message translates to:
  /// **'Appearance'**
  String get appearance;

  /// No description provided for @themeSystem.
  ///
  /// In en, this message translates to:
  /// **'System'**
  String get themeSystem;

  /// No description provided for @themeLight.
  ///
  /// In en, this message translates to:
  /// **'Light'**
  String get themeLight;

  /// No description provided for @themeDark.
  ///
  /// In en, this message translates to:
  /// **'Dark'**
  String get themeDark;

  /// No description provided for @language.
  ///
  /// In en, this message translates to:
  /// **'Language'**
  String get language;

  /// No description provided for @langSystem.
  ///
  /// In en, this message translates to:
  /// **'System'**
  String get langSystem;

  /// No description provided for @thisDevice.
  ///
  /// In en, this message translates to:
  /// **'This device'**
  String get thisDevice;

  /// No description provided for @startOver.
  ///
  /// In en, this message translates to:
  /// **'Start over'**
  String get startOver;

  /// No description provided for @startOverTitle.
  ///
  /// In en, this message translates to:
  /// **'Start over on this device?'**
  String get startOverTitle;

  /// No description provided for @startOverBody.
  ///
  /// In en, this message translates to:
  /// **'This device will leave your other devices and forget them. Files already received stay where they are.'**
  String get startOverBody;

  /// No description provided for @removedBanner.
  ///
  /// In en, this message translates to:
  /// **'This device was disconnected from your other devices.'**
  String get removedBanner;

  /// No description provided for @folderProblem.
  ///
  /// In en, this message translates to:
  /// **'The chosen folder is unavailable, so files are going to the default folder.'**
  String get folderProblem;

  /// No description provided for @newDeviceTitle.
  ///
  /// In en, this message translates to:
  /// **'New device: {name}'**
  String newDeviceTitle(String name);

  /// No description provided for @newDeviceBody.
  ///
  /// In en, this message translates to:
  /// **'It was added through another of your devices. Allow it to send files here?'**
  String get newDeviceBody;

  /// No description provided for @allow.
  ///
  /// In en, this message translates to:
  /// **'Allow'**
  String get allow;

  /// No description provided for @errUnreachable.
  ///
  /// In en, this message translates to:
  /// **'Can\'t reach this device. Check that Zerohop is open and both devices are on the same network.'**
  String get errUnreachable;

  /// No description provided for @errConnectionLost.
  ///
  /// In en, this message translates to:
  /// **'The connection dropped. You can continue the transfer.'**
  String get errConnectionLost;

  /// No description provided for @errDeclined.
  ///
  /// In en, this message translates to:
  /// **'The other device declined the transfer.'**
  String get errDeclined;

  /// No description provided for @errNoSpace.
  ///
  /// In en, this message translates to:
  /// **'The other device doesn\'t have enough space.'**
  String get errNoSpace;

  /// No description provided for @errBusy.
  ///
  /// In en, this message translates to:
  /// **'The other device is busy. Try again in a minute.'**
  String get errBusy;

  /// No description provided for @errInvalidName.
  ///
  /// In en, this message translates to:
  /// **'One of the names can\'t be saved on the other device.'**
  String get errInvalidName;

  /// No description provided for @errCancelled.
  ///
  /// In en, this message translates to:
  /// **'Transfer cancelled.'**
  String get errCancelled;

  /// No description provided for @errCancelledByPeer.
  ///
  /// In en, this message translates to:
  /// **'The transfer was cancelled on the other device.'**
  String get errCancelledByPeer;

  /// No description provided for @errIntegrity.
  ///
  /// In en, this message translates to:
  /// **'A file was damaged on the way. Send it again.'**
  String get errIntegrity;

  /// No description provided for @errSourceChanged.
  ///
  /// In en, this message translates to:
  /// **'A file changed while it was being sent. Send it again.'**
  String get errSourceChanged;

  /// No description provided for @errSourceUnreadable.
  ///
  /// In en, this message translates to:
  /// **'A file couldn\'t be read.'**
  String get errSourceUnreadable;

  /// No description provided for @errNothingToSend.
  ///
  /// In en, this message translates to:
  /// **'There is nothing to send here.'**
  String get errNothingToSend;

  /// No description provided for @errRemoved.
  ///
  /// In en, this message translates to:
  /// **'This device was disconnected from your other devices.'**
  String get errRemoved;

  /// No description provided for @errPeerRemoved.
  ///
  /// In en, this message translates to:
  /// **'That device is no longer one of yours.'**
  String get errPeerRemoved;

  /// No description provided for @errCannotResume.
  ///
  /// In en, this message translates to:
  /// **'This can no longer be continued. Send the files again.'**
  String get errCannotResume;

  /// No description provided for @errProtocol.
  ///
  /// In en, this message translates to:
  /// **'The other device replied incorrectly. Update Zerohop on both devices.'**
  String get errProtocol;

  /// No description provided for @errStorage.
  ///
  /// In en, this message translates to:
  /// **'Files couldn\'t be written to the disk.'**
  String get errStorage;

  /// No description provided for @errPeerStorage.
  ///
  /// In en, this message translates to:
  /// **'The other device couldn\'t save the files.'**
  String get errPeerStorage;

  /// No description provided for @errAwaiting.
  ///
  /// In en, this message translates to:
  /// **'Approve this device first.'**
  String get errAwaiting;

  /// No description provided for @errInUse.
  ///
  /// In en, this message translates to:
  /// **'Zerohop is already open.'**
  String get errInUse;

  /// No description provided for @errWrongCode.
  ///
  /// In en, this message translates to:
  /// **'That code didn\'t work. Show a new code and try again.'**
  String get errWrongCode;

  /// No description provided for @errTimeout.
  ///
  /// In en, this message translates to:
  /// **'Time ran out. Show a new code.'**
  String get errTimeout;

  /// No description provided for @errPairDeclined.
  ///
  /// In en, this message translates to:
  /// **'The link was declined.'**
  String get errPairDeclined;

  /// No description provided for @errPairUnreachable.
  ///
  /// In en, this message translates to:
  /// **'Couldn\'t connect. Check that both devices are on the same network.'**
  String get errPairUnreachable;

  /// No description provided for @errPairFailed.
  ///
  /// In en, this message translates to:
  /// **'Linking failed. Try again.'**
  String get errPairFailed;

  /// No description provided for @errAlreadyLinked.
  ///
  /// In en, this message translates to:
  /// **'This device is already linked to others. Show a code here and enter it on the new device.'**
  String get errAlreadyLinked;

  /// No description provided for @errBadAddress.
  ///
  /// In en, this message translates to:
  /// **'That address doesn\'t look right.'**
  String get errBadAddress;

  /// No description provided for @errBadName.
  ///
  /// In en, this message translates to:
  /// **'Enter a shorter name.'**
  String get errBadName;

  /// No description provided for @errFolder.
  ///
  /// In en, this message translates to:
  /// **'That folder can\'t be used.'**
  String get errFolder;

  /// No description provided for @errGeneric.
  ///
  /// In en, this message translates to:
  /// **'Something went wrong. Try again.'**
  String get errGeneric;

  /// No description provided for @unitB.
  ///
  /// In en, this message translates to:
  /// **'B'**
  String get unitB;

  /// No description provided for @unitKB.
  ///
  /// In en, this message translates to:
  /// **'KB'**
  String get unitKB;

  /// No description provided for @unitMB.
  ///
  /// In en, this message translates to:
  /// **'MB'**
  String get unitMB;

  /// No description provided for @unitGB.
  ///
  /// In en, this message translates to:
  /// **'GB'**
  String get unitGB;

  /// No description provided for @unitTB.
  ///
  /// In en, this message translates to:
  /// **'TB'**
  String get unitTB;

  /// No description provided for @clipboard.
  ///
  /// In en, this message translates to:
  /// **'Shared clipboard'**
  String get clipboard;

  /// No description provided for @clipboardSub.
  ///
  /// In en, this message translates to:
  /// **'Copy on one device and paste on another. Passwords from password managers stay where they are.'**
  String get clipboardSub;

  /// No description provided for @clipboardOnDevice.
  ///
  /// In en, this message translates to:
  /// **'What you copy here can be pasted there'**
  String get clipboardOnDevice;

  /// No description provided for @clipboardOffDevice.
  ///
  /// In en, this message translates to:
  /// **'Off for this device'**
  String get clipboardOffDevice;

  /// No description provided for @clipboardOff.
  ///
  /// In en, this message translates to:
  /// **'Off'**
  String get clipboardOff;

  /// No description provided for @clipboardOneTap.
  ///
  /// In en, this message translates to:
  /// **'Send with one tap'**
  String get clipboardOneTap;

  /// No description provided for @clipboardSend.
  ///
  /// In en, this message translates to:
  /// **'Send'**
  String get clipboardSend;

  /// No description provided for @clipboardEnable.
  ///
  /// In en, this message translates to:
  /// **'Turn on'**
  String get clipboardEnable;

  /// No description provided for @clipboardOptions.
  ///
  /// In en, this message translates to:
  /// **'Options'**
  String get clipboardOptions;

  /// No description provided for @clipTurnOnFirst.
  ///
  /// In en, this message translates to:
  /// **'Turn on the shared clipboard in Settings first.'**
  String get clipTurnOnFirst;

  /// No description provided for @clipSendTo.
  ///
  /// In en, this message translates to:
  /// **'Send to this device'**
  String get clipSendTo;

  /// No description provided for @clipReceiveFrom.
  ///
  /// In en, this message translates to:
  /// **'Receive from this device'**
  String get clipReceiveFrom;

  /// No description provided for @clipText.
  ///
  /// In en, this message translates to:
  /// **'Text and links'**
  String get clipText;

  /// No description provided for @clipImages.
  ///
  /// In en, this message translates to:
  /// **'Pictures'**
  String get clipImages;

  /// No description provided for @clipAllow.
  ///
  /// In en, this message translates to:
  /// **'Allow'**
  String get clipAllow;

  /// No description provided for @clipAllowTitle.
  ///
  /// In en, this message translates to:
  /// **'macOS permission'**
  String get clipAllowTitle;

  /// No description provided for @clipNeedsAllow.
  ///
  /// In en, this message translates to:
  /// **'Let Zerohop read the clipboard: choose Allow in macOS settings'**
  String get clipNeedsAllow;

  /// No description provided for @clipSent.
  ///
  /// In en, this message translates to:
  /// **'Clipboard sent to nearby devices'**
  String get clipSent;

  /// No description provided for @clipArrived.
  ///
  /// In en, this message translates to:
  /// **'Clipboard received: {name}'**
  String clipArrived(String name);

  /// No description provided for @clipNoDevices.
  ///
  /// In en, this message translates to:
  /// **'No nearby device can take the clipboard'**
  String get clipNoDevices;

  /// No description provided for @clipFailed.
  ///
  /// In en, this message translates to:
  /// **'Could not send the clipboard'**
  String get clipFailed;

  /// No description provided for @clipEmpty.
  ///
  /// In en, this message translates to:
  /// **'Copy some text or a picture first'**
  String get clipEmpty;

  /// No description provided for @clipSecret.
  ///
  /// In en, this message translates to:
  /// **'This was copied as secret and is not sent'**
  String get clipSecret;

  /// No description provided for @clipTooLarge.
  ///
  /// In en, this message translates to:
  /// **'This is too large for the clipboard'**
  String get clipTooLarge;

  /// No description provided for @clipSame.
  ///
  /// In en, this message translates to:
  /// **'This already came from another device'**
  String get clipSame;

  /// No description provided for @clipDenied.
  ///
  /// In en, this message translates to:
  /// **'The system did not allow reading the clipboard'**
  String get clipDenied;

  /// No description provided for @clipTileTitle.
  ///
  /// In en, this message translates to:
  /// **'Button in quick settings'**
  String get clipTileTitle;

  /// No description provided for @clipTileSub.
  ///
  /// In en, this message translates to:
  /// **'Send what you copied from quick settings or the notification, without opening the app.'**
  String get clipTileSub;

  /// No description provided for @clipTileAdd.
  ///
  /// In en, this message translates to:
  /// **'Add'**
  String get clipTileAdd;

  /// No description provided for @clipTileManual.
  ///
  /// In en, this message translates to:
  /// **'Add the Send clipboard tile to quick settings by hand'**
  String get clipTileManual;

  /// No description provided for @clipMenuTitle.
  ///
  /// In en, this message translates to:
  /// **'“To Zerohop” entry'**
  String get clipMenuTitle;

  /// No description provided for @clipMenuSub.
  ///
  /// In en, this message translates to:
  /// **'Adds it to the text selection menu, and text from Share goes straight to the clipboard. Any app on screen can use it too, so turn it on only if you trust your apps.'**
  String get clipMenuSub;

  /// No description provided for @errKeyUnavailable.
  ///
  /// In en, this message translates to:
  /// **'The system did not give access to this device\'s key. Allow access and try again.'**
  String get errKeyUnavailable;

  /// No description provided for @errKeyLocked.
  ///
  /// In en, this message translates to:
  /// **'This device\'s key can no longer be opened. To keep using the app, start over and link your devices again.'**
  String get errKeyLocked;

  /// No description provided for @keyRetry.
  ///
  /// In en, this message translates to:
  /// **'Try again'**
  String get keyRetry;

  /// No description provided for @keyStartOver.
  ///
  /// In en, this message translates to:
  /// **'Start over'**
  String get keyStartOver;

  /// No description provided for @shareNoDevices.
  ///
  /// In en, this message translates to:
  /// **'Add a device first'**
  String get shareNoDevices;

  /// No description provided for @shareChooseDevice.
  ///
  /// In en, this message translates to:
  /// **'Choose a device for these files'**
  String get shareChooseDevice;
}

class _L10nDelegate extends LocalizationsDelegate<L10n> {
  const _L10nDelegate();

  @override
  Future<L10n> load(Locale locale) {
    return SynchronousFuture<L10n>(lookupL10n(locale));
  }

  @override
  bool isSupported(Locale locale) =>
      <String>['en', 'ru'].contains(locale.languageCode);

  @override
  bool shouldReload(_L10nDelegate old) => false;
}

L10n lookupL10n(Locale locale) {
  // Lookup logic when only language code is specified.
  switch (locale.languageCode) {
    case 'en':
      return L10nEn();
    case 'ru':
      return L10nRu();
  }

  throw FlutterError(
    'L10n.delegate failed to load unsupported locale "$locale". This is likely '
    'an issue with the localizations generation tool. Please file an issue '
    'on GitHub with a reproducible sample app and the gen-l10n configuration '
    'that was used.',
  );
}
