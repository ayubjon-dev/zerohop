import 'package:intl/intl.dart';

import '../l10n/app_localizations.dart';

String formatSize(L10n t, int bytes) {
  final units = [t.unitB, t.unitKB, t.unitMB, t.unitGB, t.unitTB];
  var value = bytes.toDouble();
  var unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  if (unit == 0) return '$bytes ${units[0]}';
  final number = NumberFormat('0.0', t.localeName).format(value);
  return '$number ${units[unit]}';
}

/// A folder path as the owner thinks of it, without the sandbox's internals.
String friendlyFolder(L10n t, String path) {
  final downloads = RegExp(r'^.*/(?:Data/)?Downloads(?=/|$)');
  final shown = path.replaceFirst(downloads, t.downloadsFolder);
  if (shown != path) return shown;
  final home = RegExp(r'^/Users/[^/]+');
  return path.replaceFirst(home, '~');
}

String formatMoment(L10n t, int millis) {
  final moment = DateTime.fromMillisecondsSinceEpoch(millis);
  return DateFormat.MMMd(t.localeName).add_Hm().format(moment);
}

/// Explains how a transfer or request ended, in the owner's words.
String describeError(L10n t, String kind) {
  switch (kind) {
    case 'key_unavailable':
      return t.errKeyUnavailable;
    case 'key_locked':
      return t.errKeyLocked;
    case 'unreachable':
    case 'offline':
      return t.errUnreachable;
    case 'connection_lost':
      return t.errConnectionLost;
    case 'declined':
      return t.errDeclined;
    case 'no_space':
      return t.errNoSpace;
    case 'busy':
      return t.errBusy;
    case 'invalid_name':
      return t.errInvalidName;
    case 'cancelled':
      return t.errCancelled;
    case 'cancelled_by_peer':
      return t.errCancelledByPeer;
    case 'integrity':
      return t.errIntegrity;
    case 'source_changed':
      return t.errSourceChanged;
    case 'source_unreadable':
      return t.errSourceUnreadable;
    case 'nothing_to_send':
      return t.errNothingToSend;
    case 'removed':
      return t.errRemoved;
    case 'peer_removed':
      return t.errPeerRemoved;
    case 'cannot_resume':
      return t.errCannotResume;
    case 'protocol':
      return t.errProtocol;
    case 'storage':
      return t.errStorage;
    case 'peer_storage':
      return t.errPeerStorage;
    case 'awaiting_confirmation':
      return t.errAwaiting;
    case 'in_use':
      return t.errInUse;
    case 'already_in_circle':
      return t.errAlreadyLinked;
    case 'bad_address':
      return t.errBadAddress;
    case 'bad_name':
      return t.errBadName;
    case 'folder_unavailable':
      return t.errFolder;
    default:
      return t.errGeneric;
  }
}

/// Explains how a pairing attempt ended.
String describePairing(L10n t, String outcome) {
  switch (outcome) {
    case 'wrong_code':
      return t.errWrongCode;
    case 'timeout':
      return t.errTimeout;
    case 'declined':
      return t.errPairDeclined;
    case 'unreachable':
      return t.errPairUnreachable;
    default:
      return t.errPairFailed;
  }
}
