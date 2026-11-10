import 'dart:io';
import 'dart:math' as math;
import 'dart:ui' show PathMetric;

import 'package:desktop_drop/desktop_drop.dart';
import 'package:flutter/material.dart';

import '../../l10n/app_localizations.dart';
import '../app_state.dart';
import '../format.dart';
import '../model.dart';
import '../platform.dart';
import '../theme.dart';
import 'icons.dart';
import 'widgets.dart';

bool get _hasDragAndDrop =>
    Platform.isMacOS || Platform.isWindows || Platform.isLinux;

List<DropdownMenuItem<String>> _modeItems(L10n t, BuildContext context) {
  DropdownMenuItem<String> item(String value, String label) => DropdownMenuItem(
    value: value,
    child: Text(label, overflow: TextOverflow.ellipsis),
  );
  return [
    item('default', t.modeDefault),
    item('ask', t.modeAsk),
    item('smart', t.modeSmart),
    item('auto', t.modeAuto),
    item('refuse', t.modeRefuse),
  ];
}

class DevicePage extends StatelessWidget {
  const DevicePage({
    super.key,
    required this.state,
    required this.snapshot,
    required this.device,
    required this.wide,
  });

  final AppState state;
  final Snapshot snapshot;
  final Device device;
  final bool wide;

  @override
  Widget build(BuildContext context) {
    final transfer = snapshot.transfers
        .where((x) => x.peer == device.id)
        .firstOrNull;
    final panel = _Panel(
      state: state,
      snapshot: snapshot,
      device: device,
      transfer: transfer,
      wide: wide,
    );
    final page = wide
        ? _WideLayout(state: state, device: device, panel: panel)
        : _NarrowLayout(
            state: state,
            device: device,
            panel: panel,
            transfer: transfer,
          );
    if (!_hasDragAndDrop) return page;
    return DropTarget(
      onDragEntered: (_) => state.setDragging(true),
      onDragExited: (_) => state.setDragging(false),
      onDragDone: (details) {
        state.setDragging(false);
        state.addPaths(device.id, details.files.map((f) => f.path));
      },
      child: page,
    );
  }
}

class _WideLayout extends StatelessWidget {
  const _WideLayout({
    required this.state,
    required this.device,
    required this.panel,
  });

  final AppState state;
  final Device device;
  final Widget panel;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    return LayoutBuilder(
      builder: (context, constraints) {
        final horizontal = (constraints.maxWidth * 0.04).clamp(20.0, 56.0);
        final clipboard = state.platform.clipboard != ClipboardSupport.none;
        // Leaves room for everything under the panel without scrolling.
        final below = clipboard ? 384.0 : 296.0;
        final panelHeight = math.max(380.0, constraints.maxHeight - below);
        return SingleChildScrollView(
          padding: EdgeInsets.symmetric(horizontal: horizontal, vertical: 40),
          child: Center(
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 920),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Row(
                    children: [
                      Expanded(
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            PageTitle(device.name),
                            const SizedBox(height: 4),
                            Row(
                              children: [
                                StatusDot(on: device.nearby),
                                const SizedBox(width: 7),
                                Text(
                                  device.nearby ? t.nearby : t.offline,
                                  style: zText(15, color: z.muted),
                                ),
                              ],
                            ),
                          ],
                        ),
                      ),
                    ],
                  ),
                  const SizedBox(height: 24),
                  SizedBox(height: panelHeight, child: panel),
                  const SizedBox(height: 24),
                  Wrap(
                    spacing: 16,
                    runSpacing: 16,
                    children: [
                      SizedBox(
                        width: math.max(
                          280,
                          math.min(constraints.maxWidth - horizontal * 2, 920) -
                              280,
                        ),
                        child: ZCard(
                          padding: const EdgeInsets.fromLTRB(18, 16, 18, 16),
                          child: Column(
                            crossAxisAlignment: CrossAxisAlignment.stretch,
                            children: [
                              Text(
                                t.receiveFrom,
                                style: zText(15, weight: 600, color: z.ink),
                              ),
                              const SizedBox(height: 8),
                              ZSelect<String>(
                                value: device.mode ?? 'default',
                                items: _modeItems(t, context),
                                onChanged: (mode) => state.setDeviceMode(
                                  device.id,
                                  mode == 'default' ? null : mode,
                                ),
                              ),
                            ],
                          ),
                        ),
                      ),
                      SizedBox(
                        width: 264,
                        child: ZCard(
                          padding: const EdgeInsets.fromLTRB(18, 16, 18, 16),
                          child: Column(
                            crossAxisAlignment: CrossAxisAlignment.stretch,
                            children: [
                              Text(
                                t.deviceCode(device.shortId),
                                style: zText(
                                  15,
                                  weight: 600,
                                  color: z.ink,
                                  tabular: true,
                                ),
                              ),
                              const SizedBox(height: 8),
                              ZButton(
                                label: t.removeDevice,
                                kind: ZButtonKind.dangerSoft,
                                height: 44,
                                onPressed: () =>
                                    state.open(Screen.removeDevice),
                              ),
                            ],
                          ),
                        ),
                      ),
                      if (clipboard)
                        SizedBox(
                          width: math.min(
                            constraints.maxWidth - horizontal * 2,
                            920,
                          ),
                          child: _DeviceClipboardCard(
                            state: state,
                            device: device,
                          ),
                        ),
                    ],
                  ),
                ],
              ),
            ),
          ),
        );
      },
    );
  }
}

class _NarrowLayout extends StatelessWidget {
  const _NarrowLayout({
    required this.state,
    required this.device,
    required this.panel,
    required this.transfer,
  });

  final AppState state;
  final Device device;
  final Widget panel;
  final Transfer? transfer;

  @override
  Widget build(BuildContext context) {
    final t = L10n.of(context);
    if (transfer != null) return panel;
    return Padding(
      padding: const EdgeInsets.fromLTRB(20, 8, 20, 20),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          BackBar(
            title: t.sendTo(device.name),
            label: t.back,
            onBack: () => state.open(Screen.home),
            trailing: IconButtonZ(
              icon: ZIcons.dots,
              label: t.settings,
              stroke: 3,
              onPressed: () => state.open(Screen.deviceSettings),
            ),
          ),
          const SizedBox(height: 16),
          Expanded(child: panel),
        ],
      ),
    );
  }
}

class _Panel extends StatelessWidget {
  const _Panel({
    required this.state,
    required this.snapshot,
    required this.device,
    required this.transfer,
    required this.wide,
  });

  final AppState state;
  final Snapshot snapshot;
  final Device device;
  final Transfer? transfer;
  final bool wide;

  @override
  Widget build(BuildContext context) {
    final active = transfer;
    if (active != null) {
      return TransferPanel(
        state: state,
        transfer: active,
        device: device,
        fullBleed: !wide,
      );
    }
    final ended = state.ended[device.id];
    if (ended != null) {
      return ended.isDone
          ? _DonePanel(state: state, ended: ended, device: device, wide: wide)
          : _FailedPanel(
              state: state,
              snapshot: snapshot,
              ended: ended,
              device: device,
            );
    }
    final picked = state.picked[device.id] ?? const <Picked>[];
    if (picked.isNotEmpty) {
      return _ChosenPanel(
        state: state,
        device: device,
        picked: picked,
        wide: wide,
      );
    }
    return _DropPanel(state: state, device: device, wide: wide);
  }
}

class _DropPanel extends StatelessWidget {
  const _DropPanel({
    required this.state,
    required this.device,
    required this.wide,
  });

  final AppState state;
  final Device device;
  final bool wide;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    final dragging = state.dragging;
    return AnimatedContainer(
      duration: const Duration(milliseconds: 120),
      decoration: BoxDecoration(
        color: dragging ? z.soft : z.surface,
        borderRadius: BorderRadius.circular(28),
        border: Border.all(color: dragging ? z.accent : z.line, width: 2),
      ),
      padding: const EdgeInsets.all(24),
      child: Center(
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              const IconCircle(icon: ZIcons.upload, size: 88, iconSize: 38),
              const SizedBox(height: 16),
              Text(
                _hasDragAndDrop ? t.dropTitle : t.chooseFiles,
                textAlign: TextAlign.center,
                style: zText(
                  26,
                  weight: 700,
                  letterSpacing: -0.02,
                  color: z.ink,
                ),
              ),
              const SizedBox(height: 16),
              Text(
                device.nearby
                    ? t.dropHint(device.name)
                    : t.dropHintOffline(device.name),
                textAlign: TextAlign.center,
                style: zText(15, color: z.muted),
              ),
              const SizedBox(height: 16),
              Wrap(
                spacing: 8,
                runSpacing: 8,
                alignment: WrapAlignment.center,
                children: [
                  ZButton(
                    label: t.chooseFiles,
                    kind: ZButtonKind.primary,
                    onPressed: () => state.chooseFiles(device.id),
                  ),
                  if (state.platform.canPickPhotos)
                    _PhotosButton(state: state, device: device),
                  if (state.platform.canPickFolders)
                    ZButton(
                      label: t.chooseFolder,
                      onPressed: () => state.chooseFolder(device.id),
                    ),
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// Picks from the photo library, which the Files app does not show.
class _PhotosButton extends StatelessWidget {
  const _PhotosButton({
    required this.state,
    required this.device,
    this.expand = false,
  });

  final AppState state;
  final Device device;
  final bool expand;

  @override
  Widget build(BuildContext context) {
    final t = L10n.of(context);
    final loading = state.loadingPhotos;
    return ZButton(
      label: loading ? t.photosLoading : t.choosePhotos,
      expand: expand,
      onPressed: loading ? null : () => state.choosePhotos(device.id),
    );
  }
}

class _ChosenPanel extends StatelessWidget {
  const _ChosenPanel({
    required this.state,
    required this.device,
    required this.picked,
    required this.wide,
  });

  final AppState state;
  final Device device;
  final List<Picked> picked;
  final bool wide;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    final total = picked.fold<int>(0, (sum, p) => sum + (p.size ?? 0));
    final fileCount = picked.fold<int>(0, (sum, p) => sum + p.files);
    final summary = t.sizeSummary(t.fileCount(fileCount), formatSize(t, total));
    final canSend = device.nearby && !device.awaitingConfirmation;
    return ZCard(
      radius: 28,
      padding: const EdgeInsets.fromLTRB(24, 16, 24, 24),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Expanded(
            child: ListView.separated(
              itemCount: picked.length,
              separatorBuilder: (_, _) => const ZDivider(),
              itemBuilder: (context, index) {
                final item = picked[index];
                final size = item.size;
                return SizedBox(
                  height: 64,
                  child: Row(
                    children: [
                      IconTile(
                        icon: ZIcons.forFile(
                          item.name,
                          isDirectory: item.isDirectory,
                        ),
                      ),
                      const SizedBox(width: 12),
                      Expanded(
                        child: Text(
                          item.name,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: zText(15, weight: 500, color: z.ink),
                        ),
                      ),
                      const SizedBox(width: 12),
                      Text(
                        size == null ? t.folderKind : formatSize(t, size),
                        style: zText(15, color: z.muted),
                      ),
                      IconButtonZ(
                        icon: ZIcons.x,
                        label: t.removeItem(item.name),
                        iconSize: 18,
                        color: z.muted,
                        onPressed: () => state.removePicked(device.id, item),
                      ),
                    ],
                  ),
                );
              },
            ),
          ),
          const SizedBox(height: 16),
          if (wide)
            Wrap(
              alignment: WrapAlignment.spaceBetween,
              crossAxisAlignment: WrapCrossAlignment.center,
              spacing: 12,
              runSpacing: 10,
              children: [
                Text(summary, style: zText(15, color: z.muted)),
                Wrap(
                  spacing: 8,
                  runSpacing: 8,
                  children: [
                    ZButton(
                      label: t.addFiles,
                      onPressed: () => state.chooseFiles(device.id),
                    ),
                    if (state.platform.canPickPhotos)
                      _PhotosButton(state: state, device: device),
                    ZButton(
                      label: t.clear,
                      onPressed: () => state.clearPicked(device.id),
                    ),
                    ZButton(
                      label: t.send,
                      kind: ZButtonKind.primary,
                      onPressed: canSend ? () => state.send(device.id) : null,
                    ),
                  ],
                ),
              ],
            )
          else ...[
            Text(summary, style: zText(15, color: z.muted)),
            const SizedBox(height: 10),
            Row(
              children: [
                Expanded(
                  child: ZButton(
                    // Next to the photos button the long label has no room.
                    label: state.platform.canPickPhotos
                        ? t.addFilesShort
                        : t.addFiles,
                    expand: true,
                    onPressed: () => state.chooseFiles(device.id),
                  ),
                ),
                if (state.platform.canPickPhotos) ...[
                  const SizedBox(width: 8),
                  Expanded(
                    child: _PhotosButton(
                      state: state,
                      device: device,
                      expand: true,
                    ),
                  ),
                ],
                const SizedBox(width: 8),
                ZButton(
                  label: t.clear,
                  onPressed: () => state.clearPicked(device.id),
                ),
              ],
            ),
            const SizedBox(height: 10),
            ZButton(
              label: t.send,
              kind: ZButtonKind.primary,
              height: 56,
              expand: true,
              onPressed: canSend ? () => state.send(device.id) : null,
            ),
          ],
        ],
      ),
    );
  }
}

class TransferPanel extends StatelessWidget {
  const TransferPanel({
    super.key,
    required this.state,
    required this.transfer,
    required this.device,
    required this.fullBleed,
  });

  final AppState state;
  final Transfer transfer;
  final Device device;
  final bool fullBleed;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    final percent = (transfer.fraction * 100).floor();
    final waiting = transfer.done == 0 && transfer.isSending;
    final title = transfer.isSending
        ? t.sendingTo(device.name)
        : t.receivingFrom(device.name);
    final detail = waiting
        ? t.waitingAnswer
        : t.progressOf(
            formatSize(t, transfer.done),
            formatSize(t, transfer.totalBytes),
          );
    final speed = waiting || transfer.speed <= 0
        ? null
        : t.perSecond(formatSize(t, transfer.speed.round()));
    const white = Colors.white;
    final content = Column(
      mainAxisAlignment: MainAxisAlignment.center,
      children: [
        Text(
          title,
          textAlign: TextAlign.center,
          style: zText(18, weight: 600, color: white),
        ),
        const SizedBox(height: 12),
        SizedBox(
          width: 360,
          height: 130,
          child: CustomPaint(painter: _ArcPainter(transfer.fraction)),
        ),
        const SizedBox(height: 12),
        Semantics(
          value: '$percent%',
          child: Text(
            '$percent%',
            style: zText(
              fullBleed ? 88 : 72,
              weight: 700,
              height: 1,
              letterSpacing: -0.04,
              color: white,
              tabular: true,
            ),
          ),
        ),
        const SizedBox(height: 8),
        Text(
          detail,
          textAlign: TextAlign.center,
          style: zText(16, color: white, tabular: true),
        ),
        if (speed != null) ...[
          const SizedBox(height: 2),
          Text(
            speed,
            style: zText(
              14,
              color: white.withValues(alpha: 0.85),
              tabular: true,
            ),
          ),
        ],
        const SizedBox(height: 18),
        ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 480),
          child: ClipRRect(
            borderRadius: BorderRadius.circular(4),
            child: SizedBox(
              height: 8,
              child: Stack(
                children: [
                  Container(color: white.withValues(alpha: 0.28)),
                  FractionallySizedBox(
                    widthFactor: transfer.fraction,
                    child: Container(color: white),
                  ),
                ],
              ),
            ),
          ),
        ),
        const SizedBox(height: 14),
        Text(
          t.encryptedLine,
          textAlign: TextAlign.center,
          style: zText(14, color: white),
        ),
        const SizedBox(height: 20),
        ZButton(
          label: t.cancelTransfer,
          kind: ZButtonKind.onGradient,
          height: fullBleed ? 56 : 48,
          expand: fullBleed,
          onPressed: () => state.cancelTransfer(transfer.id),
        ),
      ],
    );
    return Container(
      padding: EdgeInsets.all(fullBleed ? 24 : 28),
      decoration: BoxDecoration(
        gradient: z.gradient,
        borderRadius: fullBleed ? null : BorderRadius.circular(28),
      ),
      child: Center(
        child: fullBleed
            ? SingleChildScrollView(child: content)
            // In a short window everything shrinks together, so that the
            // button to cancel is never cut off.
            : FittedBox(
                fit: BoxFit.scaleDown,
                child: SizedBox(width: 560, child: content),
              ),
      ),
    );
  }
}

class _ArcPainter extends CustomPainter {
  _ArcPainter(this.fraction);

  final double fraction;

  @override
  void paint(Canvas canvas, Size size) {
    final path = Path()
      ..moveTo(40, 104)
      ..cubicTo(110, 8, 250, 8, 320, 104);
    final line = Paint()
      ..style = PaintingStyle.stroke
      ..strokeCap = StrokeCap.round
      ..strokeWidth = 6
      ..color = Colors.white.withValues(alpha: 0.3);
    canvas.drawPath(path, line);
    final PathMetric metric = path.computeMetrics().first;
    final travelled = metric.length * fraction;
    if (travelled > 0) {
      canvas.drawPath(
        metric.extractPath(0, travelled),
        line..color = Colors.white,
      );
    }
    final dot = Paint()..color = Colors.white;
    final position = metric.getTangentForOffset(travelled)?.position;
    if (position != null) canvas.drawCircle(position, 10, dot);
    canvas.drawCircle(const Offset(40, 104), 16, dot);
    canvas.drawCircle(const Offset(320, 104), 16, dot);
  }

  @override
  bool shouldRepaint(_ArcPainter old) => old.fraction != fraction;
}

class _DonePanel extends StatelessWidget {
  const _DonePanel({
    required this.state,
    required this.ended,
    required this.device,
    required this.wide,
  });

  final AppState state;
  final Ended ended;
  final Device device;
  final bool wide;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    final summary = t.sizeSummary(
      t.fileCount(ended.files),
      formatSize(t, ended.bytes),
    );
    final text = ended.isSent
        ? t.doneSent(summary, device.name)
        : t.doneReceived(summary, device.name);
    return ZCard(
      radius: 28,
      padding: const EdgeInsets.all(24),
      child: Center(
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              const IconCircle(
                icon: ZIcons.check,
                size: 88,
                iconSize: 42,
                stroke: 2.4,
              ),
              const SizedBox(height: 12),
              Text(
                t.done,
                style: zText(
                  26,
                  weight: 700,
                  letterSpacing: -0.02,
                  color: z.ink,
                ),
              ),
              const SizedBox(height: 12),
              Text(
                text,
                textAlign: TextAlign.center,
                style: zText(15, color: z.muted, height: 1.4),
              ),
              if (ended.held > 0) ...[
                const SizedBox(height: 10),
                Text(
                  ended.isSent
                      ? t.heldNoteSent(ended.held)
                      : t.heldNote(ended.held),
                  textAlign: TextAlign.center,
                  style: zText(15, weight: 500, color: z.danger),
                ),
              ],
              const SizedBox(height: 16),
              Wrap(
                spacing: 8,
                runSpacing: 8,
                alignment: WrapAlignment.center,
                children: [
                  if (!ended.isSent)
                    ZButton(label: t.showInFolder, onPressed: state.showFolder),
                  if (ended.held > 0 && !ended.isSent)
                    ZButton(
                      label: t.history,
                      onPressed: () => state.open(Screen.history),
                    ),
                  ZButton(
                    label: t.sendMore,
                    kind: ended.isSent
                        ? ZButtonKind.secondary
                        : ZButtonKind.primary,
                    onPressed: () => state.dismissEnded(device.id),
                  ),
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _FailedPanel extends StatelessWidget {
  const _FailedPanel({
    required this.state,
    required this.snapshot,
    required this.ended,
    required this.device,
  });

  final AppState state;
  final Snapshot snapshot;
  final Ended ended;
  final Device device;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    final resumable =
        ended.canResume && snapshot.unfinished.any((u) => u.id == ended.id);
    return ZCard(
      radius: 28,
      padding: const EdgeInsets.all(24),
      child: Center(
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Container(
                width: 88,
                height: 88,
                alignment: Alignment.center,
                decoration: BoxDecoration(
                  color: z.dangerSoft,
                  shape: BoxShape.circle,
                ),
                child: ZIcon(ZIcons.warn, size: 38, color: z.danger),
              ),
              const SizedBox(height: 16),
              ConstrainedBox(
                constraints: const BoxConstraints(maxWidth: 440),
                child: Text(
                  describeError(t, ended.outcome),
                  textAlign: TextAlign.center,
                  style: zText(18, weight: 600, height: 1.35, color: z.ink),
                ),
              ),
              const SizedBox(height: 18),
              Wrap(
                spacing: 8,
                runSpacing: 8,
                alignment: WrapAlignment.center,
                children: [
                  ZButton(
                    label: t.close,
                    onPressed: () => state.dismissEnded(device.id),
                  ),
                  if (resumable)
                    ZButton(
                      label: t.resume,
                      kind: ZButtonKind.primary,
                      onPressed: device.nearby
                          ? () => state.resume(ended.id)
                          : null,
                    ),
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class DeviceSettingsPage extends StatelessWidget {
  const DeviceSettingsPage({
    super.key,
    required this.state,
    required this.snapshot,
    required this.device,
  });

  final AppState state;
  final Snapshot snapshot;
  final Device device;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    Widget option(String? value, String label, String sub) {
      final selected = device.mode == value;
      return InkWell(
        onTap: () => state.setDeviceMode(device.id, value),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
          child: Row(
            children: [
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(label, style: zText(16, weight: 600, color: z.ink)),
                    if (sub.isNotEmpty)
                      Text(sub, style: zText(14, color: z.muted)),
                  ],
                ),
              ),
              if (selected)
                ZIcon(ZIcons.check, size: 22, stroke: 2.6, color: z.accent),
            ],
          ),
        ),
      );
    }

    return ListView(
      padding: const EdgeInsets.fromLTRB(20, 8, 20, 20),
      children: [
        BackBar(
          title: device.name,
          subtitle: t.deviceCode(device.shortId),
          label: t.back,
          onBack: () => state.open(Screen.device),
        ),
        const SizedBox(height: 20),
        Text(t.receiving, style: zText(17, weight: 600, color: z.ink)),
        const SizedBox(height: 10),
        ZCard(
          child: Column(
            children: [
              option(null, t.modeDefault, ''),
              const ZDivider(),
              option('ask', t.modeAsk, t.modeAskSub),
              const ZDivider(),
              option('smart', t.modeSmart, t.modeSmartSub),
              const ZDivider(),
              option('auto', t.modeAuto, t.modeAutoSub),
              const ZDivider(),
              option('refuse', t.modeRefuse, t.modeRefuseSub),
            ],
          ),
        ),
        if (state.platform.clipboard != ClipboardSupport.none) ...[
          const SizedBox(height: 20),
          Text(t.clipboard, style: zText(17, weight: 600, color: z.ink)),
          if (!snapshot.clipboard) ...[
            const SizedBox(height: 6),
            Text(t.clipTurnOnFirst, style: zText(14, color: z.muted)),
          ],
          const SizedBox(height: 10),
          _ClipboardRules(state: state, snapshot: snapshot, device: device),
        ],
        const SizedBox(height: 20),
        ZButton(
          label: t.removeDevice,
          kind: ZButtonKind.dangerSoft,
          height: 52,
          expand: true,
          onPressed: () => state.open(Screen.removeDevice),
        ),
      ],
    );
  }
}

/// The switches for what the shared clipboard does with one device.
class _ClipboardRules extends StatelessWidget {
  const _ClipboardRules({
    required this.state,
    required this.snapshot,
    required this.device,
  });

  final AppState state;
  final Snapshot snapshot;
  final Device device;

  @override
  Widget build(BuildContext context) {
    final t = L10n.of(context);
    final z = context.z;
    final rules = device.clipboard;
    Widget row(String key, String label, bool value) => ConstrainedBox(
      constraints: const BoxConstraints(minHeight: 54),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 8),
        child: Row(
          children: [
            Expanded(
              child: Text(label, style: zText(16, color: z.ink)),
            ),
            const SizedBox(width: 12),
            ZSwitch(
              value: value,
              label: label,
              onChanged: (next) =>
                  state.setClipboardRule(device.id, {key: next}),
            ),
          ],
        ),
      ),
    );
    final enabled = snapshot.clipboard;
    return Opacity(
      opacity: enabled ? 1 : 0.5,
      child: IgnorePointer(
        ignoring: !enabled,
        child: ZCard(
          child: Column(
            children: [
              row('send', t.clipSendTo, rules.send),
              const ZDivider(),
              row('receive', t.clipReceiveFrom, rules.receive),
              const ZDivider(),
              row('text', t.clipText, rules.text),
              const ZDivider(),
              row('images', t.clipImages, rules.images),
            ],
          ),
        ),
      ),
    );
  }
}

/// The shared clipboard for one device, on a wide screen.
class _DeviceClipboardCard extends StatelessWidget {
  const _DeviceClipboardCard({required this.state, required this.device});

  final AppState state;
  final Device device;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    final enabled = state.snapshot?.clipboard ?? false;
    final both = device.clipboard.bothWays;
    final allowing = state.clipboardNeedsAllowing;
    final status = !enabled
        ? t.clipTurnOnFirst
        : allowing
        ? t.clipNeedsAllow
        : both
        ? t.clipboardOnDevice
        : t.clipboardOffDevice;
    return ZCard(
      padding: const EdgeInsets.fromLTRB(18, 14, 18, 14),
      child: Row(
        children: [
          const IconTile(icon: ZIcons.clip, size: 44, radius: 14),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(t.clipboard, style: zText(15, weight: 600, color: z.ink)),
                Text(status, style: zText(14, color: z.muted)),
              ],
            ),
          ),
          const SizedBox(width: 12),
          if (!enabled)
            ZButton(
              label: t.clipboardEnable,
              height: 44,
              onPressed: () => state.open(Screen.settings),
            )
          else ...[
            if (allowing)
              ZButton(
                label: t.clipAllow,
                height: 44,
                onPressed: state.platform.openClipboardSettings,
              )
            else
              ZButton(
                label: t.clipboardOptions,
                height: 44,
                onPressed: () {
                  state.selected = device.id;
                  state.open(Screen.deviceSettings);
                },
              ),
            const SizedBox(width: 10),
            ZSwitch(
              value: both,
              label: t.clipboard,
              onChanged: (next) => state.setClipboardRule(device.id, {
                'send': next,
                'receive': next,
              }),
            ),
          ],
        ],
      ),
    );
  }
}

/// The shared clipboard on the home page of a phone or a narrow window.
class _HomeClipboardCard extends StatelessWidget {
  const _HomeClipboardCard({required this.state, required this.snapshot});

  final AppState state;
  final Snapshot snapshot;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    final enabled = snapshot.clipboard;
    final oneTap = state.platform.clipboard == ClipboardSupport.oneTap;
    final allowing = state.clipboardNeedsAllowing;
    final status = !enabled
        ? t.clipboardOff
        : allowing
        ? t.clipNeedsAllow
        : oneTap
        ? t.clipboardOneTap
        : t.clipboardOnDevice;
    return Material(
      color: z.surface,
      borderRadius: BorderRadius.circular(20),
      child: Padding(
        padding: const EdgeInsets.fromLTRB(14, 12, 12, 12),
        child: Row(
          children: [
            const IconTile(icon: ZIcons.clip, size: 44, radius: 14),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    t.clipboard,
                    style: zText(16, weight: 600, color: z.ink),
                  ),
                  Text(status, style: zText(14, color: z.muted)),
                ],
              ),
            ),
            const SizedBox(width: 12),
            if (!enabled)
              ZButton(
                label: t.clipboardEnable,
                height: 44,
                onPressed: () => state.open(Screen.settings),
              )
            else if (allowing)
              ZButton(
                label: t.clipAllow,
                height: 44,
                onPressed: state.platform.openClipboardSettings,
              )
            else if (oneTap)
              ZButton(
                label: t.clipboardSend,
                kind: ZButtonKind.primary,
                height: 44,
                onPressed: state.sendClipboard,
              ),
          ],
        ),
      ),
    );
  }
}

class RemoveDevicePage extends StatelessWidget {
  const RemoveDevicePage({
    super.key,
    required this.state,
    required this.device,
  });

  final AppState state;
  final Device device;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    return Center(
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 480),
        child: Padding(
          padding: const EdgeInsets.all(20),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text(
                t.removeTitle(device.name),
                style: zText(
                  28,
                  weight: 700,
                  height: 1.2,
                  letterSpacing: -0.02,
                  color: z.ink,
                ),
              ),
              const SizedBox(height: 14),
              Text(t.removeBody, style: zText(16, color: z.muted, height: 1.4)),
              const SizedBox(height: 24),
              ZButton(
                label: t.removeAction,
                kind: ZButtonKind.danger,
                height: 56,
                expand: true,
                onPressed: () => state.removeDevice(device.id),
              ),
              const SizedBox(height: 10),
              ZButton(
                label: t.cancelAction,
                height: 56,
                expand: true,
                onPressed: () => state.open(Screen.device),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class EmptyHome extends StatelessWidget {
  const EmptyHome({super.key, required this.state});

  final AppState state;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    return Center(
      child: SingleChildScrollView(
        padding: const EdgeInsets.all(24),
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 460),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              const IconCircle(
                icon: ZIcons.plus,
                size: 88,
                iconSize: 38,
                stroke: 2.2,
              ),
              const SizedBox(height: 20),
              Text(
                t.emptyTitle,
                textAlign: TextAlign.center,
                style: zText(
                  26,
                  weight: 700,
                  letterSpacing: -0.02,
                  color: z.ink,
                ),
              ),
              const SizedBox(height: 10),
              Text(
                t.emptyBody,
                textAlign: TextAlign.center,
                style: zText(16, color: z.muted, height: 1.4),
              ),
              const SizedBox(height: 20),
              ZButton(
                label: t.addDevice,
                kind: ZButtonKind.primary,
                onPressed: () => state.open(Screen.pair),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class HomePage extends StatelessWidget {
  const HomePage({super.key, required this.state, required this.snapshot});

  final AppState state;
  final Snapshot snapshot;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    if (snapshot.devices.isEmpty) return EmptyHome(state: state);
    return ListView(
      padding: const EdgeInsets.fromLTRB(20, 16, 20, 20),
      children: [
        SizedBox(
          height: 48,
          child: Row(
            children: [
              const Logo(size: 34),
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
        const SizedBox(height: 20),
        PageTitle(t.myDevices),
        const SizedBox(height: 8),
        Text(t.homeHint, style: zText(16, color: z.muted)),
        const SizedBox(height: 20),
        GridView.count(
          crossAxisCount: 2,
          shrinkWrap: true,
          physics: const NeverScrollableScrollPhysics(),
          mainAxisSpacing: 12,
          crossAxisSpacing: 12,
          childAspectRatio: 1.06,
          children: [
            for (final device in snapshot.devices)
              _DeviceTile(state: state, device: device),
            _AddTile(label: t.add, onTap: () => state.open(Screen.pair)),
          ],
        ),
        if (state.platform.clipboard != ClipboardSupport.none) ...[
          const SizedBox(height: 12),
          _HomeClipboardCard(state: state, snapshot: snapshot),
        ],
      ],
    );
  }
}

class _DeviceTile extends StatelessWidget {
  const _DeviceTile({required this.state, required this.device});

  final AppState state;
  final Device device;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    return Material(
      color: z.surface,
      borderRadius: BorderRadius.circular(24),
      child: InkWell(
        onTap: () => state.openDevice(device.id),
        borderRadius: BorderRadius.circular(24),
        child: Padding(
          padding: const EdgeInsets.all(16),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisAlignment: MainAxisAlignment.spaceBetween,
            children: [
              IconCircle(
                icon: ZIcons.forPlatform(device.platform),
                size: 60,
                iconSize: 28,
                lit: device.nearby,
              ),
              Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    device.name,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: zText(17, weight: 600, height: 1.2, color: z.ink),
                  ),
                  const SizedBox(height: 4),
                  Row(
                    children: [
                      StatusDot(on: device.nearby),
                      const SizedBox(width: 6),
                      Text(
                        device.nearby ? t.nearby : t.offline,
                        style: zText(14, color: z.muted),
                      ),
                    ],
                  ),
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _AddTile extends StatelessWidget {
  const _AddTile({required this.label, required this.onTap});

  final String label;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    return InkWell(
      onTap: onTap,
      borderRadius: BorderRadius.circular(24),
      child: Container(
        decoration: BoxDecoration(
          borderRadius: BorderRadius.circular(24),
          border: Border.all(color: z.line, width: 1.5),
        ),
        child: Column(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            Container(
              width: 52,
              height: 52,
              alignment: Alignment.center,
              decoration: BoxDecoration(color: z.soft, shape: BoxShape.circle),
              child: ZIcon(ZIcons.plus, size: 26, stroke: 2.2, color: z.accent),
            ),
            const SizedBox(height: 10),
            Text(label, style: zText(16, weight: 600, color: z.accent)),
          ],
        ),
      ),
    );
  }
}
