import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../l10n/app_localizations.dart';
import '../app_state.dart';
import '../format.dart';
import '../model.dart';
import '../theme.dart';
import 'icons.dart';
import 'widgets.dart';

/// Files handed over by another app or by Send to: what they are, and which
/// device they go to. Choosing a device sends nothing; the Send button does.
class ShareDialog extends StatelessWidget {
  const ShareDialog({
    super.key,
    required this.state,
    required this.snapshot,
    required this.items,
  });

  final AppState state;
  final Snapshot snapshot;
  final List<Picked> items;

  /// How many of the files are named before the rest are only counted.
  static const _named = 3;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    final total = items.fold<int>(0, (sum, p) => sum + (p.size ?? 0));
    final files = items.fold<int>(0, (sum, p) => sum + p.files);
    final devices = [
      ...snapshot.devices.where(AppState.canReceive),
      ...snapshot.devices.where((d) => !AppState.canReceive(d)),
    ];
    final chosen = snapshot.deviceById(state.sharingTo);
    final ready = chosen != null && AppState.canReceive(chosen);
    final nobody = !devices.any(AppState.canReceive);
    final height = MediaQuery.sizeOf(context).height;
    return CallbackShortcuts(
      bindings: {
        const SingleActivator(LogicalKeyboardKey.escape): state.cancelShared,
      },
      child: Focus(
        autofocus: true,
        child: ColoredBox(
          color: const Color(0x8C080A10),
          child: Center(
            child: Padding(
              padding: const EdgeInsets.all(16),
              child: Semantics(
                scopesRoute: true,
                explicitChildNodes: true,
                child: Container(
                  constraints: BoxConstraints(
                    maxWidth: 460,
                    maxHeight: height - 32,
                  ),
                  padding: const EdgeInsets.fromLTRB(24, 24, 24, 20),
                  decoration: BoxDecoration(
                    color: z.surface,
                    borderRadius: BorderRadius.circular(28),
                  ),
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      Semantics(
                        header: true,
                        namesRoute: true,
                        child: Text(
                          t.shareTitle,
                          style: zText(
                            22,
                            weight: 700,
                            letterSpacing: -0.01,
                            color: z.ink,
                          ),
                        ),
                      ),
                      const SizedBox(height: 4),
                      Text(
                        t.sizeSummary(t.fileCount(files), formatSize(t, total)),
                        style: zText(15, color: z.muted),
                      ),
                      const SizedBox(height: 14),
                      _Files(items: items, named: _named),
                      const SizedBox(height: 18),
                      Flexible(
                        child: SingleChildScrollView(
                          child: Column(
                            crossAxisAlignment: CrossAxisAlignment.stretch,
                            children: [
                              for (final device in devices)
                                Padding(
                                  padding: const EdgeInsets.only(bottom: 8),
                                  child: _Target(
                                    device: device,
                                    chosen: ready && device.id == chosen.id,
                                    onTap: AppState.canReceive(device)
                                        ? () =>
                                              state.chooseShareTarget(device.id)
                                        : null,
                                  ),
                                ),
                              if (nobody)
                                Padding(
                                  padding: const EdgeInsets.only(top: 2),
                                  child: Text(
                                    t.shareNobodyNearby,
                                    style: zText(
                                      14,
                                      color: z.muted,
                                      height: 1.35,
                                    ),
                                  ),
                                ),
                            ],
                          ),
                        ),
                      ),
                      const SizedBox(height: 12),
                      Row(
                        children: [
                          ZButton(
                            label: t.cancelAction,
                            onPressed: state.cancelShared,
                          ),
                          const SizedBox(width: 8),
                          Expanded(
                            child: ZButton(
                              label: ready
                                  ? t.sendTo(chosen.name)
                                  : t.shareChoose,
                              kind: ZButtonKind.primary,
                              expand: true,
                              onPressed: ready ? state.sendShared : null,
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
        ),
      ),
    );
  }
}

/// The first few files by name, and how many more there are.
class _Files extends StatelessWidget {
  const _Files({required this.items, required this.named});

  final List<Picked> items;
  final int named;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    final shown = items.take(named).toList();
    final rest = items.length - shown.length;
    return Container(
      padding: const EdgeInsets.fromLTRB(14, 10, 14, 10),
      decoration: BoxDecoration(
        color: z.surface2,
        borderRadius: BorderRadius.circular(16),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final item in shown)
            SizedBox(
              height: 30,
              child: Row(
                children: [
                  ZIcon(
                    ZIcons.forFile(item.name, isDirectory: item.isDirectory),
                    size: 18,
                    color: z.muted,
                  ),
                  const SizedBox(width: 10),
                  Expanded(
                    child: Text(
                      item.name,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: zText(14, weight: 500, color: z.ink),
                    ),
                  ),
                  const SizedBox(width: 10),
                  Text(
                    item.size == null
                        ? t.folderKind
                        : formatSize(t, item.size!),
                    style: zText(14, color: z.muted, tabular: true),
                  ),
                ],
              ),
            ),
          if (rest > 0)
            SizedBox(
              height: 26,
              child: Padding(
                padding: const EdgeInsets.only(left: 28),
                child: Align(
                  alignment: Alignment.centerLeft,
                  child: Text(
                    t.shareMore(rest),
                    style: zText(14, color: z.muted),
                  ),
                ),
              ),
            ),
        ],
      ),
    );
  }
}

/// One of the owner's devices. Those that cannot take files now are shown
/// faded, so the owner knows why they are missing from the choice.
class _Target extends StatelessWidget {
  const _Target({
    required this.device,
    required this.chosen,
    required this.onTap,
  });

  final Device device;
  final bool chosen;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    final radius = BorderRadius.circular(16);
    final String status;
    if (device.awaitingConfirmation) {
      status = t.shareAwaiting;
    } else {
      status = device.nearby ? t.nearby : t.offline;
    }
    return Semantics(
      inMutuallyExclusiveGroup: true,
      checked: chosen,
      enabled: onTap != null,
      child: Opacity(
        opacity: onTap == null ? 0.5 : 1,
        child: Material(
          color: chosen ? z.soft : Colors.transparent,
          shape: RoundedRectangleBorder(
            borderRadius: radius,
            side: BorderSide(
              color: chosen ? z.accent : z.line,
              width: chosen ? 2 : 1,
            ),
          ),
          child: InkWell(
            onTap: onTap,
            borderRadius: radius,
            child: Padding(
              padding: EdgeInsets.all(chosen ? 9 : 10),
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
                            Expanded(
                              child: Text(
                                status,
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis,
                                style: zText(13, color: z.muted),
                              ),
                            ),
                          ],
                        ),
                      ],
                    ),
                  ),
                  const SizedBox(width: 12),
                  _Mark(on: chosen),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// The round mark at the end of a device row.
class _Mark extends StatelessWidget {
  const _Mark({required this.on});

  final bool on;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    return Container(
      width: 24,
      height: 24,
      decoration: BoxDecoration(
        shape: BoxShape.circle,
        color: on ? z.accent : Colors.transparent,
        border: on ? null : Border.all(color: z.line, width: 2),
      ),
      child: on
          ? Center(
              child: ZIcon(ZIcons.check, size: 15, stroke: 2.6, color: z.on),
            )
          : null,
    );
  }
}
