import Foundation
import Network

/// Finding Zerohop's devices through the system's own Bonjour service, and
/// announcing this one. Whatever arrives here is only a hint about which
/// address to try: the core decides who is really there.
///
/// Pairing rooms are announced and looked for by the interface, since
/// pairing happens only while its window is open.
final class Bonjour {
  private static let type = "_zerohop._tcp"

  private var browser: NWBrowser?
  private var found: [String: Found] = [:]
  private var advert: DNSServiceRef?
  private var advertKey: String?

  /// A device seen on the network, and what is known about it so far.
  private final class Found {
    let instance: String
    var tag: String?
    var port = 0
    var addresses: [String] = []
    /// The lookup under way, if any.
    var lookup: DNSServiceRef?

    init(instance: String) { self.instance = instance }
  }

  // MARK: Looking for devices

  func browse() {
    guard browser == nil else { return }
    let browser = NWBrowser(
      for: .bonjourWithTXTRecord(type: Self.type, domain: "local."), using: .tcp)
    browser.browseResultsChangedHandler = { [weak self] _, changes in
      self?.changed(changes)
    }
    // Looking stops for good when it fails, such as while the network
    // changes, so it starts again a little later.
    browser.stateUpdateHandler = { [weak self, weak browser] state in
      guard case .failed = state, let self, let browser, self.browser === browser else { return }
      browser.cancel()
      self.browser = nil
      // Whatever is still there is found again, and looked up afresh.
      self.found.values.forEach(self.stop)
      self.found = [:]
      DispatchQueue.main.asyncAfter(deadline: .now() + 5) { self.browse() }
    }
    browser.start(queue: .main)
    self.browser = browser
  }

  private func changed(_ changes: Set<NWBrowser.Result.Change>) {
    for change in changes {
      switch change {
      case .added(let result):
        guard let instance = Self.instance(result), found[instance] == nil else { continue }
        let device = Found(instance: instance)
        device.tag = Self.tag(result)
        found[instance] = device
        look(up: device)
      case .changed(_, let result, _):
        guard let instance = Self.instance(result), let device = found[instance] else { continue }
        device.tag = Self.tag(result)
        if device.addresses.isEmpty { look(up: device) } else { report(device) }
      case .removed(let result):
        guard let instance = Self.instance(result), let device = found.removeValue(forKey: instance)
        else { continue }
        stop(device)
        Core.send("discovery_lost", ["instance": instance])
      default:
        break
      }
    }
  }

  private static func instance(_ result: NWBrowser.Result) -> String? {
    guard case .service(let name, _, _, _) = result.endpoint else { return nil }
    return name
  }

  private static func tag(_ result: NWBrowser.Result) -> String? {
    guard case .bonjour(let record) = result.metadata else { return nil }
    return record["t"]
  }

  private func report(_ device: Found) {
    guard !device.addresses.isEmpty else { return }
    var params: [String: Any] = [
      "instance": device.instance, "addresses": device.addresses, "port": device.port,
    ]
    if let tag = device.tag { params["tag"] = tag }
    Core.send("discovery_seen", params)
  }

  /// Asks for the device's port and then its addresses. Both answers arrive
  /// on the main queue, where everything here lives.
  private func look(up device: Found) {
    stop(device)
    var ref: DNSServiceRef?
    let context = Unmanaged.passUnretained(device).toOpaque()
    guard
      DNSServiceResolve(
        &ref, 0, 0, device.instance, Self.type, "local.", Self.resolved, context)
        == kDNSServiceErr_NoError, let ref
    else { return }
    device.lookup = ref
    if DNSServiceSetDispatchQueue(ref, .main) != kDNSServiceErr_NoError { stop(device) }
  }

  private func stop(_ device: Found) {
    if let lookup = device.lookup { DNSServiceRefDeallocate(lookup) }
    device.lookup = nil
  }

  private static let resolved: DNSServiceResolveReply = {
    _, _, _, error, _, host, port, _, _, context in
    guard let context else { return }
    let device = Unmanaged<Found>.fromOpaque(context).takeUnretainedValue()
    let bonjour = Service.shared.bonjour
    guard bonjour.found[device.instance] === device else { return }
    bonjour.stop(device)
    guard error == kDNSServiceErr_NoError, let host else { return }
    device.port = Int(UInt16(bigEndian: port))
    device.addresses = []
    var ref: DNSServiceRef?
    guard
      DNSServiceGetAddrInfo(
        &ref, 0, 0, DNSServiceProtocol(kDNSServiceProtocol_IPv4 | kDNSServiceProtocol_IPv6), host,
        Bonjour.addressed, context) == kDNSServiceErr_NoError, let ref
    else { return }
    device.lookup = ref
    if DNSServiceSetDispatchQueue(ref, .main) != kDNSServiceErr_NoError { bonjour.stop(device) }
  }

  private static let addressed: DNSServiceGetAddrInfoReply = {
    _, flags, _, error, _, address, _, context in
    guard let context else { return }
    let device = Unmanaged<Found>.fromOpaque(context).takeUnretainedValue()
    let bonjour = Service.shared.bonjour
    guard bonjour.found[device.instance] === device else { return }
    if error == kDNSServiceErr_NoError, let address, let text = Bonjour.numeric(address),
      !device.addresses.contains(text)
    {
      device.addresses.append(text)
    }
    guard flags & DNSServiceFlags(kDNSServiceFlagsMoreComing) == 0 else { return }
    bonjour.stop(device)
    bonjour.report(device)
  }

  /// The address as text. The system's copy lasts only for the callback.
  private static func numeric(_ address: UnsafePointer<sockaddr>) -> String? {
    var host = [CChar](repeating: 0, count: Int(NI_MAXHOST))
    let result = getnameinfo(
      address, socklen_t(address.pointee.sa_len), &host, socklen_t(host.count), nil, 0,
      NI_NUMERICHOST)
    return result == 0 ? String(cString: host) : nil
  }

  // MARK: Announcing this device

  /// Announces this device, or stops, to match what the core wants. The
  /// announcement holds a random name and a rotating tag, never the
  /// device's name.
  func advertise(_ wanted: [String: Any]?) {
    let instance = wanted?["instance"] as? String
    let tag = wanted?["tag"] as? String
    let port = wanted?["port"] as? Int
    let key = instance.flatMap { i in tag.flatMap { t in port.map { "\(i):\(t):\($0)" } } }
    guard key != advertKey else { return }
    advertKey = key
    if let advert { DNSServiceRefDeallocate(advert) }
    advert = nil
    guard let instance, let tag, let port, let portNumber = UInt16(exactly: port) else { return }
    let txt = Self.txt(["v": "1", "t": tag])
    var ref: DNSServiceRef?
    let error = txt.withUnsafeBytes { bytes in
      DNSServiceRegister(
        &ref, 0, 0, instance, Self.type, "local.", nil, portNumber.bigEndian,
        UInt16(bytes.count), bytes.baseAddress, nil, nil)
    }
    guard error == kDNSServiceErr_NoError, let ref else { return }
    if DNSServiceSetDispatchQueue(ref, .main) != kDNSServiceErr_NoError {
      DNSServiceRefDeallocate(ref)
      return
    }
    advert = ref
  }

  /// A TXT record: each entry is its length followed by "key=value".
  private static func txt(_ entries: [String: String]) -> Data {
    var record = Data()
    for (key, value) in entries.sorted(by: { $0.key < $1.key }) {
      let entry = Data("\(key)=\(value)".utf8)
      guard entry.count <= 255 else { continue }
      record.append(UInt8(entry.count))
      record.append(entry)
    }
    return record
  }
}
