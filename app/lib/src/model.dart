// A readable view of the state the core publishes.

class Me {
  Me(Map<String, dynamic> raw)
    : id = raw['id'] as String,
      shortId = raw['short_id'] as String,
      name = raw['name'] as String,
      removed = raw['removed'] as bool? ?? false;

  final String id;
  final String shortId;
  final String name;
  final bool removed;
}

/// What the shared clipboard may do with one device.
class ClipRules {
  ClipRules(Map<String, dynamic>? raw)
    : send = raw?['send'] as bool? ?? true,
      receive = raw?['receive'] as bool? ?? true,
      text = raw?['text'] as bool? ?? true,
      images = raw?['images'] as bool? ?? true;

  final bool send;
  final bool receive;
  final bool text;
  final bool images;

  bool get bothWays => send && receive;
}

class Device {
  Device(Map<String, dynamic> raw)
    : id = raw['id'] as String,
      shortId = raw['short_id'] as String,
      name = raw['name'] as String,
      platform = raw['platform'] as String,
      nearby = raw['nearby'] as bool? ?? false,
      mode = raw['mode'] as String?,
      awaitingConfirmation = raw['awaiting_confirmation'] as bool? ?? false,
      clipboard = ClipRules(raw['clipboard'] as Map<String, dynamic>?);

  final String id;
  final String shortId;
  final String name;
  final String platform;
  final bool nearby;
  final String? mode;
  final bool awaitingConfirmation;
  final ClipRules clipboard;

  bool get isPhone => platform == 'ios' || platform == 'android';
}

class Pairing {
  Pairing(Map<String, dynamic> raw)
    : role = raw['role'] as String,
      phase = raw['phase'] as String,
      code = raw['code'] as String?,
      codeSpent = raw['code_spent'] as bool? ?? false,
      qr = raw['qr'] as String?,
      addresses = [
        for (final a in raw['addresses'] as List<dynamic>? ?? const [])
          a as String,
      ],
      room = raw['room'] as String?,
      port = raw['port'] as int?,
      comparison = raw['comparison'] as String?,
      peerName = raw['peer_name'] as String?,
      peerPlatform = raw['peer_platform'] as String?,
      peerShortId = raw['peer_short_id'] as String?,
      deadlineMs = raw['deadline_ms'] as int;

  final String role;
  final String phase;
  final String? code;

  /// Someone tried the code, so it no longer works; the QR code still does.
  final bool codeSpent;

  /// The QR payload the host shows, when it has an address to put in it.
  final String? qr;

  /// Where the host can be reached with the code, as address:port.
  final List<String> addresses;
  final String? room;
  final int? port;
  final String? comparison;
  final String? peerName;
  final String? peerPlatform;
  final String? peerShortId;
  final int deadlineMs;

  bool get isHost => role == 'host';
}

class Offer {
  Offer(Map<String, dynamic> raw)
    : id = raw['id'] as int,
      peer = raw['peer'] as String,
      peerName = raw['peer_name'] as String,
      files = raw['files'] as int,
      totalBytes = raw['total_bytes'] as int,
      hasProgram = raw['has_program'] as bool? ?? false,
      firstProgram = raw['first_program'] as String?;

  final int id;
  final String peer;
  final String peerName;
  final int files;
  final int totalBytes;
  final bool hasProgram;
  final String? firstProgram;
}

class Transfer {
  Transfer(Map<String, dynamic> raw)
    : id = raw['id'] as String,
      direction = raw['direction'] as String,
      peer = raw['peer'] as String,
      peerName = raw['peer_name'] as String,
      files = raw['files'] as int,
      totalBytes = raw['total_bytes'] as int,
      name = raw['name'] as String,
      done = raw['done'] as int,
      speed = (raw['speed'] as num?)?.toDouble() ?? 0;

  final String id;
  final String direction;
  final String peer;
  final String peerName;
  final int files;
  final int totalBytes;
  final String name;
  int done;
  double speed;

  bool get isSending => direction == 'sent';
  double get fraction =>
      totalBytes == 0 ? 0 : (done / totalBytes).clamp(0.0, 1.0);
}

class Unfinished {
  Unfinished(Map<String, dynamic> raw)
    : id = raw['id'] as String,
      peer = raw['peer'] as String,
      peerName = raw['peer_name'] as String,
      files = raw['files'] as int,
      totalBytes = raw['total_bytes'] as int,
      name = raw['name'] as String;

  final String id;
  final String peer;
  final String peerName;
  final int files;
  final int totalBytes;
  final String name;
}

class Held {
  Held(Map<String, dynamic> raw)
    : transfer = raw['transfer'] as String,
      peerName = raw['peer_name'] as String,
      name = raw['name'] as String,
      rawName = raw['raw_name'] as String,
      bytes = raw['bytes'] as int;

  final String transfer;
  final String peerName;
  final String name;
  final String rawName;
  final int bytes;
}

class HistoryItem {
  HistoryItem(Map<String, dynamic> raw)
    : id = raw['id'] as String,
      atMs = raw['at_ms'] as int,
      direction = raw['direction'] as String,
      peerName = raw['peer_name'] as String,
      files = raw['files'] as int,
      bytes = raw['bytes'] as int,
      name = raw['name'] as String,
      outcome = raw['outcome'] as String,
      held = raw['held'] as int? ?? 0;

  final String id;
  final int atMs;
  final String direction;
  final String peerName;
  final int files;
  final int bytes;
  final String name;
  final String outcome;
  final int held;

  bool get isSent => direction == 'sent';
}

class Snapshot {
  Snapshot(Map<String, dynamic> raw)
    : name = raw['name'] as String?,
      me = raw['me'] == null ? null : Me(raw['me'] as Map<String, dynamic>),
      devices = _list(raw['devices'], Device.new),
      defaultMode = raw['settings']['default_mode'] as String,
      folder = raw['settings']['folder'] as String,
      visible = raw['settings']['visible'] as bool,
      confirmNew = raw['settings']['confirm_new'] as bool,
      clipboard = raw['settings']['clipboard'] as bool? ?? false,
      ui = Map<String, dynamic>.from(raw['settings']['ui'] as Map),
      problem = raw['problem'] as String?,
      pairing = raw['pairing'] == null
          ? null
          : Pairing(raw['pairing'] as Map<String, dynamic>),
      offers = _list(raw['offers'], Offer.new),
      transfers = _list(raw['transfers'], Transfer.new),
      unfinished = _list(raw['unfinished'], Unfinished.new),
      held = _list(raw['held'], Held.new),
      history = _list(raw['history'], HistoryItem.new),
      advert = raw['advert'] as Map<String, dynamic>?;

  final String? name;
  final Me? me;
  final List<Device> devices;
  final String defaultMode;
  final String folder;
  final bool visible;
  final bool confirmNew;
  final bool clipboard;
  final Map<String, dynamic> ui;
  final String? problem;
  final Pairing? pairing;
  final List<Offer> offers;
  final List<Transfer> transfers;
  final List<Unfinished> unfinished;
  final List<Held> held;
  final List<HistoryItem> history;
  final Map<String, dynamic>? advert;

  Device? deviceById(String? id) {
    for (final device in devices) {
      if (device.id == id) return device;
    }
    return null;
  }

  Transfer? transferFor(String peer) {
    for (final transfer in transfers) {
      if (transfer.peer == peer) return transfer;
    }
    return null;
  }

  static List<T> _list<T>(Object? raw, T Function(Map<String, dynamic>) make) {
    return (raw as List<dynamic>? ?? const [])
        .map((e) => make(e as Map<String, dynamic>))
        .toList();
  }
}

/// How the last transfer with a device ended, until the owner moves on.
class Ended {
  Ended(Map<String, dynamic> raw)
    : id = raw['id'] as String,
      direction = raw['direction'] as String,
      peer = raw['peer'] as String,
      peerName = raw['peer_name'] as String,
      files = raw['files'] as int,
      bytes = raw['bytes'] as int,
      name = raw['name'] as String,
      outcome = raw['outcome'] as String,
      canResume = raw['can_resume'] as bool? ?? false,
      held = raw['held'] as int? ?? 0;

  final String id;
  final String direction;
  final String peer;
  final String peerName;
  final int files;
  final int bytes;
  final String name;
  final String outcome;
  final bool canResume;
  final int held;

  bool get isDone => outcome == 'done';
  bool get isSent => direction == 'sent';
}

class Picked {
  Picked({required this.path, required this.name, required this.isDirectory});

  final String path;
  final String name;
  final bool isDirectory;

  /// Null while a folder is still being measured.
  int? size;

  /// How many files this stands for, once known.
  int files = 1;
}

class Room {
  const Room({
    required this.instance,
    required this.label,
    required this.addresses,
    required this.port,
  });

  final String instance;
  final String label;
  final List<String> addresses;
  final int port;

  /// The address to try. On the same computer Bonjour also reports loopback,
  /// which the core refuses because it is not a local network.
  String? get address {
    final usable = addresses.where(
      (a) => !a.contains(':') && !a.startsWith('127.'),
    );
    return usable.isEmpty ? null : '${usable.first}:$port';
  }
}
