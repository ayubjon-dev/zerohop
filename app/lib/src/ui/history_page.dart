import 'package:flutter/material.dart';

import '../../l10n/app_localizations.dart';
import '../app_state.dart';
import '../format.dart';
import '../model.dart';
import '../theme.dart';
import 'icons.dart';
import 'widgets.dart';

EdgeInsets pagePadding(BuildContext context, bool wide) {
  if (!wide) return const EdgeInsets.fromLTRB(20, 16, 20, 20);
  final width = MediaQuery.sizeOf(context).width - 288;
  final horizontal = (width * 0.04).clamp(20.0, 56.0);
  return EdgeInsets.fromLTRB(horizontal, 40, horizontal, 40);
}

class HistoryPage extends StatelessWidget {
  const HistoryPage({
    super.key,
    required this.state,
    required this.snapshot,
    required this.wide,
  });

  final AppState state;
  final Snapshot snapshot;
  final bool wide;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    final history = snapshot.history;
    return SingleChildScrollView(
      padding: pagePadding(context, wide),
      child: Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 920),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              if (!wide) const SizedBox(height: 32),
              PageTitle(t.history),
              const SizedBox(height: 20),
              if (snapshot.held.isNotEmpty) ...[
                Text(
                  t.heldSection,
                  style: zText(17, weight: 600, color: z.ink),
                ),
                const SizedBox(height: 6),
                Text(t.heldBody, style: zText(14, color: z.muted, height: 1.4)),
                const SizedBox(height: 10),
                ZCard(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      for (final (index, item) in snapshot.held.indexed) ...[
                        if (index > 0) const ZDivider(),
                        _HeldRow(state: state, item: item),
                      ],
                    ],
                  ),
                ),
                const SizedBox(height: 24),
              ],
              if (snapshot.unfinished.isNotEmpty) ...[
                Text(
                  t.unfinishedSection,
                  style: zText(17, weight: 600, color: z.ink),
                ),
                const SizedBox(height: 10),
                ZCard(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      for (final (index, item)
                          in snapshot.unfinished.indexed) ...[
                        if (index > 0) const ZDivider(),
                        _UnfinishedRow(
                          state: state,
                          snapshot: snapshot,
                          item: item,
                        ),
                      ],
                    ],
                  ),
                ),
                const SizedBox(height: 24),
              ],
              if (history.isEmpty &&
                  snapshot.held.isEmpty &&
                  snapshot.unfinished.isEmpty)
                Padding(
                  padding: const EdgeInsets.only(top: 24),
                  child: Text(
                    t.historyEmpty,
                    textAlign: TextAlign.center,
                    style: zText(16, color: z.muted),
                  ),
                ),
              if (history.isNotEmpty)
                ZCard(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      for (final (index, item) in history.indexed) ...[
                        if (index > 0) const ZDivider(),
                        _HistoryRow(item: item),
                      ],
                    ],
                  ),
                ),
              if (history.isNotEmpty) ...[
                const SizedBox(height: 12),
                Align(
                  alignment: Alignment.centerLeft,
                  child: TextButton(
                    onPressed: state.clearHistory,
                    child: Text(
                      t.clearHistory,
                      style: zText(14, weight: 600, color: z.muted),
                    ),
                  ),
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

class _HistoryRow extends StatelessWidget {
  const _HistoryRow({required this.item});

  final HistoryItem item;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    final String title = switch (item.outcome) {
      'done' =>
        item.isSent ? t.sentCount(item.files) : t.receivedCount(item.files),
      'declined' => t.historyDeclined,
      'cancelled' || 'cancelled_by_peer' => t.historyCancelled,
      _ => t.historyInterrupted,
    };
    final size = formatSize(t, item.bytes);
    final sub = item.isSent
        ? t.toName(item.peerName, size)
        : t.fromName(item.peerName, size);
    final success = item.outcome == 'done';
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
      child: Row(
        children: [
          Container(
            width: 42,
            height: 42,
            alignment: Alignment.center,
            decoration: BoxDecoration(
              color: success ? z.soft : z.surface2,
              shape: BoxShape.circle,
            ),
            child: ZIcon(
              item.isSent ? ZIcons.up : ZIcons.down,
              size: 20,
              stroke: 2,
              color: success ? z.accent : z.muted,
            ),
          ),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(title, style: zText(15, weight: 600, color: z.ink)),
                Text(
                  sub,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: zText(14, color: z.muted),
                ),
              ],
            ),
          ),
          const SizedBox(width: 12),
          Text(
            formatMoment(t, item.atMs),
            style: zText(13, color: z.muted, tabular: true),
          ),
        ],
      ),
    );
  }
}

class _HeldRow extends StatelessWidget {
  const _HeldRow({required this.state, required this.item});

  final AppState state;
  final Held item;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
      child: Wrap(
        alignment: WrapAlignment.spaceBetween,
        crossAxisAlignment: WrapCrossAlignment.center,
        spacing: 12,
        runSpacing: 10,
        children: [
          ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 420),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Container(
                  width: 42,
                  height: 42,
                  alignment: Alignment.center,
                  decoration: BoxDecoration(
                    color: z.dangerSoft,
                    shape: BoxShape.circle,
                  ),
                  child: ZIcon(
                    ZIcons.warn,
                    size: 20,
                    stroke: 2,
                    color: z.danger,
                  ),
                ),
                const SizedBox(width: 12),
                Flexible(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        item.name,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: zText(15, weight: 600, color: z.ink),
                      ),
                      Text(
                        t.heldFrom(item.peerName, formatSize(t, item.bytes)),
                        style: zText(14, color: z.muted),
                      ),
                    ],
                  ),
                ),
              ],
            ),
          ),
          Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              ZButton(
                label: t.delete,
                height: 44,
                kind: ZButtonKind.dangerSoft,
                onPressed: () => state.discardHeld(item),
              ),
              const SizedBox(width: 8),
              ZButton(
                label: t.letIn,
                height: 44,
                onPressed: () => state.releaseHeld(item),
              ),
            ],
          ),
        ],
      ),
    );
  }
}

class _UnfinishedRow extends StatelessWidget {
  const _UnfinishedRow({
    required this.state,
    required this.snapshot,
    required this.item,
  });

  final AppState state;
  final Snapshot snapshot;
  final Unfinished item;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    final nearby = snapshot.deviceById(item.peer)?.nearby ?? false;
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
      child: Wrap(
        alignment: WrapAlignment.spaceBetween,
        crossAxisAlignment: WrapCrossAlignment.center,
        spacing: 12,
        runSpacing: 10,
        children: [
          ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 420),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  item.name,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: zText(15, weight: 600, color: z.ink),
                ),
                Text(
                  t.toName(item.peerName, formatSize(t, item.totalBytes)),
                  style: zText(14, color: z.muted),
                ),
              ],
            ),
          ),
          Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              ZButton(
                label: t.forget,
                height: 44,
                onPressed: () => state.forgetUnfinished(item.id),
              ),
              const SizedBox(width: 8),
              ZButton(
                label: t.resume,
                height: 44,
                kind: ZButtonKind.primary,
                onPressed: nearby ? () => state.resume(item.id) : null,
              ),
            ],
          ),
        ],
      ),
    );
  }
}
