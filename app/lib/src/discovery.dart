// Finding devices through the operating system's own Bonjour or DNS-SD
// service. Whatever arrives here is only a hint about which address to try:
// the core decides who is really there.

import 'package:bonsoir/bonsoir.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import 'bridge/core.dart';
import 'model.dart';

const _serviceType = '_zerohop._tcp';
const _pairingType = '_zerohop-pair._tcp';

class Discovery {
  Discovery(this._core);

  final Core _core;
  final rooms = ValueNotifier<List<Room>>(const []);

  BonsoirBroadcast? _advert;
  String? _advertKey;
  BonsoirBroadcast? _room;
  String? _roomKey;
  BonsoirDiscovery? _browser;
  BonsoirDiscovery? _roomBrowser;

  Future<void> startBrowsing() async {
    if (_browser != null) return;
    final browser = BonsoirDiscovery(type: _serviceType, printLogs: false);
    _browser = browser;
    await browser.initialize();
    browser.eventStream?.listen((event) {
      switch (event) {
        case BonsoirDiscoveryServiceFoundEvent():
          _resolve(event.service, browser);
        case BonsoirDiscoveryServiceResolvedEvent():
          _report(event.service);
        case BonsoirDiscoveryServiceUpdatedEvent():
          _report(event.service);
        case BonsoirDiscoveryServiceLostEvent():
          _core.call('discovery_lost', {'instance': event.service.name});
        default:
          break;
      }
    });
    await browser.start();
  }

  /// A service can vanish between being found and being looked up.
  Future<void> _resolve(
    BonsoirService service,
    BonsoirDiscovery browser,
  ) async {
    try {
      await service.resolve(browser.serviceResolver);
    } on PlatformException {
      // It has gone, and it will be reported again if it comes back.
    }
  }

  void _report(BonsoirService service) {
    if (service.hostAddresses.isEmpty) return;
    _core.call('discovery_seen', {
      'instance': service.name,
      'addresses': service.hostAddresses,
      'port': service.port,
      'tag': service.attributes['t'],
    });
  }

  /// Announces this device, or stops, to match what the core wants.
  Future<void> applyAdvert(Map<String, dynamic>? advert) async {
    final key = advert == null
        ? null
        : '${advert['instance']}:${advert['tag']}';
    if (key == _advertKey) return;
    _advertKey = key;
    await _advert?.stop();
    _advert = null;
    if (advert == null) return;
    final broadcast = BonsoirBroadcast(
      printLogs: false,
      service: BonsoirService(
        name: advert['instance'] as String,
        type: _serviceType,
        port: advert['port'] as int,
        attributes: {'v': '1', 't': advert['tag'] as String},
      ),
    );
    _advert = broadcast;
    await broadcast.initialize();
    await broadcast.start();
  }

  /// Announces a pairing room only while one is open.
  Future<void> applyRoom(Pairing? pairing) async {
    final open =
        pairing != null && pairing.isHost && pairing.phase == 'waiting';
    final key = open ? '${pairing.room}:${pairing.port}' : null;
    if (key == _roomKey) return;
    _roomKey = key;
    await _room?.stop();
    _room = null;
    if (!open) return;
    final broadcast = BonsoirBroadcast(
      printLogs: false,
      service: BonsoirService(
        name: 'room-${pairing.room}',
        type: _pairingType,
        port: pairing.port!,
        attributes: {'v': '1', 'r': pairing.room!},
      ),
    );
    _room = broadcast;
    await broadcast.initialize();
    await broadcast.start();
  }

  /// Looks for open pairing rooms while the owner is about to enter a code.
  Future<void> browseRooms(bool active) async {
    if (!active) {
      await _roomBrowser?.stop();
      _roomBrowser = null;
      rooms.value = const [];
      return;
    }
    if (_roomBrowser != null) return;
    final browser = BonsoirDiscovery(type: _pairingType, printLogs: false);
    _roomBrowser = browser;
    await browser.initialize();
    browser.eventStream?.listen((event) {
      switch (event) {
        case BonsoirDiscoveryServiceFoundEvent():
          _resolve(event.service, browser);
        case BonsoirDiscoveryServiceResolvedEvent():
          _addRoom(event.service);
        case BonsoirDiscoveryServiceUpdatedEvent():
          _addRoom(event.service);
        case BonsoirDiscoveryServiceLostEvent():
          rooms.value = rooms.value
              .where((r) => r.instance != event.service.name)
              .toList();
        default:
          break;
      }
    });
    await browser.start();
  }

  void _addRoom(BonsoirService service) {
    final label = service.attributes['r'];
    if (label == null || service.hostAddresses.isEmpty) return;
    final room = Room(
      instance: service.name,
      label: label,
      addresses: service.hostAddresses,
      port: service.port,
    );
    rooms.value = [
      ...rooms.value.where((r) => r.instance != room.instance),
      room,
    ];
  }

  /// The core renews what it announces when the network changes.
  void forgetRooms() {
    rooms.value = const [];
  }
}
