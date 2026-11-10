import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../l10n/app_localizations.dart';
import '../app_state.dart';
import '../format.dart';
import '../model.dart';
import '../theme.dart';
import 'device_page.dart';
import 'history_page.dart';
import 'icons.dart';
import 'onboarding.dart';
import 'pair_pages.dart';
import 'settings_page.dart';
import 'share_dialog.dart';
import 'widgets.dart';

const wideBreakpoint = 760.0;

class AppShell extends StatelessWidget {
  const AppShell({super.key, required this.state});

  final AppState state;

  @override
  Widget build(BuildContext context) {
    final snapshot = state.snapshot;
    final t = L10n.of(context);
    Widget body;
    if (state.fatal != null) {
      body = _Fatal(state: state, kind: state.fatal!);
    } else if (snapshot == null) {
      body = const SizedBox.shrink();
    } else if (snapshot.name == null) {
      body = Onboarding(state: state);
    } else {
      body = LayoutBuilder(
        builder: (context, constraints) {
          final wide = constraints.maxWidth >= wideBreakpoint;
          return wide
              ? _Wide(state: state, snapshot: snapshot)
              : _Narrow(state: state, snapshot: snapshot);
        },
      );
    }
    final offer = snapshot?.offers.firstOrNull;
    final sharing = state.sharing;
    final dark = Theme.of(context).brightness == Brightness.dark;
    return AnnotatedRegion<SystemUiOverlayStyle>(
      value: dark ? SystemUiOverlayStyle.light : SystemUiOverlayStyle.dark,
      child: Scaffold(
        body: Stack(
          children: [
            Positioned.fill(child: body),
            if (sharing != null && snapshot != null && state.fatal == null)
              Positioned.fill(
                child: ShareDialog(
                  state: state,
                  snapshot: snapshot,
                  items: sharing,
                ),
              ),
            if (offer != null)
              Positioned.fill(
                child: OfferDialog(state: state, offer: offer),
              ),
            if (state.toast != null) _Toast(text: state.toast!(t)),
          ],
        ),
      ),
    );
  }
}

/// Something stopped the app from starting, and what can be done about it.
class _Fatal extends StatelessWidget {
  const _Fatal({required this.state, required this.kind});

  final AppState state;
  final String kind;

  @override
  Widget build(BuildContext context) {
    final t = L10n.of(context);
    return Center(
      child: SingleChildScrollView(
        padding: const EdgeInsets.all(32),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(
              describeError(t, kind),
              textAlign: TextAlign.center,
              style: zText(17, color: context.z.ink),
            ),
            if (kind == 'key_unavailable') ...[
              const SizedBox(height: 20),
              ZButton(
                label: t.keyRetry,
                kind: ZButtonKind.primary,
                onPressed: state.retryStartup,
              ),
            ],
            if (kind == 'key_locked') ...[
              const SizedBox(height: 20),
              ZButton(
                label: t.keyStartOver,
                kind: ZButtonKind.dangerSoft,
                onPressed: state.startOverAfterLostKey,
              ),
            ],
          ],
        ),
      ),
    );
  }
}

class _Toast extends StatelessWidget {
  const _Toast({required this.text});

  final String text;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    return Positioned(
      left: 20,
      right: 20,
      bottom: 24,
      child: Center(
        child: Semantics(
          liveRegion: true,
          child: Container(
            constraints: const BoxConstraints(maxWidth: 520),
            padding: const EdgeInsets.symmetric(horizontal: 18, vertical: 12),
            decoration: BoxDecoration(
              color: z.ink,
              borderRadius: BorderRadius.circular(14),
            ),
            child: Text(
              text,
              textAlign: TextAlign.center,
              style: zText(15, weight: 500, color: z.bg),
            ),
          ),
        ),
      ),
    );
  }
}

/// What to show in the main area, for any window size.
Widget mainContent(AppState state, Snapshot snapshot, {required bool wide}) {
  final pairing = snapshot.pairing;
  if (pairing != null &&
      (pairing.phase == 'compare' || pairing.phase == 'finishing')) {
    return ComparePage(state: state, pairing: pairing);
  }
  if (pairing != null && pairing.phase == 'connecting') {
    return const ConnectingPage();
  }
  switch (state.screen) {
    case Screen.history:
      return HistoryPage(state: state, snapshot: snapshot, wide: wide);
    case Screen.settings:
      return SettingsPage(state: state, snapshot: snapshot, wide: wide);
    case Screen.pair:
      return PairChoicePage(state: state, wide: wide);
    case Screen.pairShow:
      return PairShowPage(state: state, pairing: pairing, wide: wide);
    case Screen.pairEnter:
      return PairEnterPage(state: state, wide: wide);
    case Screen.removeDevice:
      final device = snapshot.deviceById(state.selected);
      if (device != null) return RemoveDevicePage(state: state, device: device);
    case Screen.deviceSettings:
      final device = snapshot.deviceById(state.selected);
      if (device != null) {
        return DeviceSettingsPage(
          state: state,
          snapshot: snapshot,
          device: device,
        );
      }
    case Screen.device:
      final device = snapshot.deviceById(state.selected);
      if (device != null) {
        return DevicePage(
          state: state,
          snapshot: snapshot,
          device: device,
          wide: wide,
        );
      }
    case Screen.home:
      break;
  }
  if (wide) {
    final first =
        snapshot.deviceById(state.selected) ?? snapshot.devices.firstOrNull;
    if (first != null) {
      return DevicePage(
        state: state,
        snapshot: snapshot,
        device: first,
        wide: true,
      );
    }
    return EmptyHome(state: state);
  }
  return HomePage(state: state, snapshot: snapshot);
}

class _Wide extends StatelessWidget {
  const _Wide({required this.state, required this.snapshot});

  final AppState state;
  final Snapshot snapshot;

  @override
  Widget build(BuildContext context) {
    return Row(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _Sidebar(state: state, snapshot: snapshot),
        Expanded(
          child: Column(
            children: [
              Notices(
                state: state,
                snapshot: snapshot,
                padding: const EdgeInsets.fromLTRB(32, 20, 32, 0),
              ),
              Expanded(child: mainContent(state, snapshot, wide: true)),
            ],
          ),
        ),
      ],
    );
  }
}

class _Sidebar extends StatelessWidget {
  const _Sidebar({required this.state, required this.snapshot});

  final AppState state;
  final Snapshot snapshot;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    final deviceScreens = {
      Screen.home,
      Screen.device,
      Screen.deviceSettings,
      Screen.removeDevice,
    };
    final current = deviceScreens.contains(state.screen)
        ? (state.selected ?? snapshot.devices.firstOrNull?.id)
        : null;
    return Container(
      width: 288,
      padding: const EdgeInsets.fromLTRB(14, 20, 14, 20),
      decoration: BoxDecoration(
        color: z.surface,
        border: Border(right: BorderSide(color: z.line)),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 8),
            child: Row(
              children: [
                const Logo(),
                const SizedBox(width: 10),
                Text(
                  'Zerohop',
                  style: zText(
                    20,
                    weight: 700,
                    letterSpacing: -0.02,
                    color: z.ink,
                  ),
                ),
              ],
            ),
          ),
          const SizedBox(height: 24),
          Padding(
            padding: const EdgeInsets.fromLTRB(8, 0, 8, 6),
            child: Text(
              t.myDevices,
              style: zText(14, weight: 600, color: z.muted),
            ),
          ),
          Expanded(
            child: ListView(
              padding: EdgeInsets.zero,
              children: [
                for (final device in snapshot.devices)
                  _DeviceRow(
                    device: device,
                    selected: device.id == current,
                    onTap: () => state.openDevice(device.id),
                  ),
                _AddRow(
                  label: t.addDevice,
                  onTap: () => state.open(Screen.pair),
                ),
              ],
            ),
          ),
          _NavRow(
            icon: ZIcons.clock,
            label: t.history,
            selected: state.screen == Screen.history,
            badge: snapshot.held.isNotEmpty,
            onTap: () => state.open(Screen.history),
          ),
          const SizedBox(height: 2),
          _NavRow(
            icon: ZIcons.sliders,
            label: t.settings,
            selected: state.screen == Screen.settings,
            onTap: () => state.open(Screen.settings),
          ),
        ],
      ),
    );
  }
}

class _DeviceRow extends StatelessWidget {
  const _DeviceRow({
    required this.device,
    required this.selected,
    required this.onTap,
  });

  final Device device;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    return Material(
      color: selected ? z.soft : Colors.transparent,
      borderRadius: BorderRadius.circular(14),
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(14),
        child: Padding(
          padding: const EdgeInsets.all(8),
          child: Row(
            children: [
              IconCircle(
                icon: ZIcons.forPlatform(device.platform),
                size: 42,
                iconSize: 21,
                lit: device.nearby,
              ),
              const SizedBox(width: 12),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      device.name,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: zText(15, weight: 600, color: z.ink),
                    ),
                    Row(
                      children: [
                        StatusDot(on: device.nearby, size: 7),
                        const SizedBox(width: 6),
                        Text(
                          device.nearby ? t.nearby : t.offline,
                          style: zText(13, color: z.muted),
                        ),
                      ],
                    ),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _AddRow extends StatelessWidget {
  const _AddRow({required this.label, required this.onTap});

  final String label;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    return InkWell(
      onTap: onTap,
      borderRadius: BorderRadius.circular(14),
      child: Padding(
        padding: const EdgeInsets.all(8),
        child: Row(
          children: [
            Container(
              width: 42,
              height: 42,
              alignment: Alignment.center,
              decoration: BoxDecoration(color: z.soft, shape: BoxShape.circle),
              child: ZIcon(ZIcons.plus, size: 21, stroke: 2.2, color: z.accent),
            ),
            const SizedBox(width: 12),
            Expanded(
              child: Text(
                label,
                style: zText(15, weight: 600, color: z.accent),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _NavRow extends StatelessWidget {
  const _NavRow({
    required this.icon,
    required this.label,
    required this.selected,
    required this.onTap,
    this.badge = false,
  });

  final String icon;
  final String label;
  final bool selected;
  final bool badge;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final color = selected ? z.ink : z.muted;
    return Material(
      color: selected ? z.soft : Colors.transparent,
      borderRadius: BorderRadius.circular(12),
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(12),
        child: SizedBox(
          height: 44,
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 10),
            child: Row(
              children: [
                ZIcon(icon, size: 21, color: color),
                const SizedBox(width: 12),
                Expanded(
                  child: Text(
                    label,
                    style: zText(15, weight: 600, color: color),
                  ),
                ),
                if (badge)
                  Container(
                    width: 8,
                    height: 8,
                    decoration: BoxDecoration(
                      color: z.danger,
                      shape: BoxShape.circle,
                    ),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class _Narrow extends StatelessWidget {
  const _Narrow({required this.state, required this.snapshot});

  final AppState state;
  final Snapshot snapshot;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    final inPairing =
        snapshot.pairing != null && snapshot.pairing!.phase != 'waiting';
    final tabs =
        !inPairing &&
        {Screen.home, Screen.history, Screen.settings}.contains(state.screen);
    return PopScope(
      canPop: false,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop && !state.goBack()) SystemNavigator.pop();
      },
      child: Column(
        children: [
          Expanded(
            child: SafeArea(
              bottom: !tabs,
              child: Column(
                children: [
                  Notices(
                    state: state,
                    snapshot: snapshot,
                    padding: const EdgeInsets.fromLTRB(20, 12, 20, 0),
                  ),
                  Expanded(child: mainContent(state, snapshot, wide: false)),
                ],
              ),
            ),
          ),
          if (tabs)
            Container(
              decoration: BoxDecoration(
                color: z.surface,
                border: Border(top: BorderSide(color: z.line)),
              ),
              child: SafeArea(
                top: false,
                child: Row(
                  children: [
                    _Tab(
                      icon: ZIcons.grid,
                      label: t.devices,
                      selected: state.screen == Screen.home,
                      onTap: () => state.open(Screen.home),
                    ),
                    _Tab(
                      icon: ZIcons.clock,
                      label: t.history,
                      selected: state.screen == Screen.history,
                      onTap: () => state.open(Screen.history),
                    ),
                    _Tab(
                      icon: ZIcons.sliders,
                      label: t.settings,
                      selected: state.screen == Screen.settings,
                      onTap: () => state.open(Screen.settings),
                    ),
                  ],
                ),
              ),
            ),
        ],
      ),
    );
  }
}

class _Tab extends StatelessWidget {
  const _Tab({
    required this.icon,
    required this.label,
    required this.selected,
    required this.onTap,
  });

  final String icon;
  final String label;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final color = selected ? z.accent : z.muted;
    return Expanded(
      child: InkWell(
        onTap: onTap,
        child: SizedBox(
          height: 64,
          child: Column(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              ZIcon(icon, size: 24, color: color),
              const SizedBox(height: 3),
              Text(label, style: zText(12, weight: 600, color: color)),
            ],
          ),
        ),
      ),
    );
  }
}

/// Things the owner should know about before anything else.
class Notices extends StatelessWidget {
  const Notices({
    super.key,
    required this.state,
    required this.snapshot,
    required this.padding,
  });

  final AppState state;
  final Snapshot snapshot;
  final EdgeInsetsGeometry padding;

  @override
  Widget build(BuildContext context) {
    final t = L10n.of(context);
    final awaiting = snapshot.devices
        .where((d) => d.awaitingConfirmation)
        .toList();
    final removed = snapshot.me?.removed ?? false;
    final folderProblem = snapshot.problem == 'folder_unavailable';
    if (awaiting.isEmpty && !removed && !folderProblem) {
      return const SizedBox.shrink();
    }
    return Padding(
      padding: padding,
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 920),
        child: Column(
          children:
              [
                    if (removed)
                      _Notice(
                        text: t.removedBanner,
                        actions: [
                          ZButton(
                            label: t.startOver,
                            onPressed: state.startOver,
                            kind: ZButtonKind.primary,
                            height: 44,
                          ),
                        ],
                      ),
                    if (folderProblem)
                      _Notice(text: t.folderProblem, actions: const []),
                    for (final device in awaiting)
                      _Notice(
                        title: t.newDeviceTitle(device.name),
                        text: t.newDeviceBody,
                        actions: [
                          ZButton(
                            label: t.removeAction,
                            onPressed: () =>
                                state.confirmDevice(device.id, false),
                            kind: ZButtonKind.dangerSoft,
                            height: 44,
                          ),
                          ZButton(
                            label: t.allow,
                            onPressed: () =>
                                state.confirmDevice(device.id, true),
                            kind: ZButtonKind.primary,
                            height: 44,
                          ),
                        ],
                      ),
                  ]
                  .map(
                    (w) => Padding(
                      padding: const EdgeInsets.only(bottom: 8),
                      child: w,
                    ),
                  )
                  .toList(),
        ),
      ),
    );
  }
}

class _Notice extends StatelessWidget {
  const _Notice({this.title, required this.text, required this.actions});

  final String? title;
  final String text;
  final List<Widget> actions;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    return ZCard(
      padding: const EdgeInsets.all(14),
      child: Wrap(
        alignment: WrapAlignment.spaceBetween,
        crossAxisAlignment: WrapCrossAlignment.center,
        spacing: 12,
        runSpacing: 10,
        children: [
          ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 520),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisSize: MainAxisSize.min,
              children: [
                if (title != null)
                  Text(title!, style: zText(15, weight: 600, color: z.ink)),
                Text(text, style: zText(14, color: z.muted, height: 1.35)),
              ],
            ),
          ),
          Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              for (final action in actions)
                Padding(padding: const EdgeInsets.only(left: 8), child: action),
            ],
          ),
        ],
      ),
    );
  }
}

class OfferDialog extends StatelessWidget {
  const OfferDialog({super.key, required this.state, required this.offer});

  final AppState state;
  final Offer offer;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    final device = state.snapshot?.deviceById(offer.peer);
    return ColoredBox(
      color: const Color(0x8C080A10),
      child: Center(
        child: Padding(
          padding: const EdgeInsets.all(20),
          child: Semantics(
            scopesRoute: true,
            explicitChildNodes: true,
            child: Container(
              constraints: const BoxConstraints(maxWidth: 440),
              padding: const EdgeInsets.all(28),
              decoration: BoxDecoration(
                color: z.surface,
                borderRadius: BorderRadius.circular(28),
              ),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  IconCircle(
                    icon: ZIcons.forPlatform(device?.platform ?? 'other'),
                    size: 72,
                    iconSize: 32,
                  ),
                  const SizedBox(height: 14),
                  Text(
                    t.incomingTitle(offer.peerName),
                    textAlign: TextAlign.center,
                    style: zText(
                      22,
                      weight: 700,
                      letterSpacing: -0.01,
                      color: z.ink,
                    ),
                  ),
                  const SizedBox(height: 14),
                  Text(
                    t.sizeSummary(
                      t.fileCount(offer.files),
                      formatSize(t, offer.totalBytes),
                    ),
                    style: zText(15, color: z.muted),
                  ),
                  if (offer.hasProgram) ...[
                    const SizedBox(height: 14),
                    Container(
                      padding: const EdgeInsets.all(14),
                      decoration: BoxDecoration(
                        color: z.dangerSoft,
                        borderRadius: BorderRadius.circular(14),
                      ),
                      child: Row(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          ZIcon(
                            ZIcons.warn,
                            size: 20,
                            stroke: 2,
                            color: z.danger,
                          ),
                          const SizedBox(width: 10),
                          Expanded(
                            child: Text.rich(
                              TextSpan(
                                children: [
                                  TextSpan(
                                    text:
                                        '${t.programWarning(offer.firstProgram ?? '')} ',
                                    style: zText(15, weight: 600, color: z.ink),
                                  ),
                                  TextSpan(
                                    text: t.programAdvice,
                                    style: zText(15, color: z.ink),
                                  ),
                                ],
                              ),
                            ),
                          ),
                        ],
                      ),
                    ),
                  ],
                  const SizedBox(height: 18),
                  Row(
                    children: [
                      Expanded(
                        child: ZButton(
                          label: t.decline,
                          expand: true,
                          onPressed: () => state.answerOffer(offer.id, false),
                        ),
                      ),
                      const SizedBox(width: 8),
                      Expanded(
                        child: ZButton(
                          label: t.accept,
                          expand: true,
                          kind: ZButtonKind.primary,
                          onPressed: () => state.answerOffer(offer.id, true),
                        ),
                      ),
                    ],
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
