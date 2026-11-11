import 'package:flutter/material.dart';

import '../../l10n/app_localizations.dart';
import '../app_state.dart';
import '../format.dart';
import '../model.dart';
import '../platform.dart';
import '../theme.dart';
import 'history_page.dart';
import 'widgets.dart';

class SettingsPage extends StatefulWidget {
  const SettingsPage({
    super.key,
    required this.state,
    required this.snapshot,
    required this.wide,
  });

  final AppState state;
  final Snapshot snapshot;
  final bool wide;

  @override
  State<SettingsPage> createState() => _SettingsPageState();
}

class _SettingsPageState extends State<SettingsPage> {
  bool _confirming = false;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    final state = widget.state;
    final snapshot = widget.snapshot;
    final palette = snapshot.ui['palette'] as String? ?? 'cobalt';
    final theme = snapshot.ui['theme'] as String? ?? 'system';
    final language = snapshot.ui['language'] as String? ?? 'system';
    Widget modeItem(String value, String label) =>
        Text(label, overflow: TextOverflow.ellipsis);
    return SingleChildScrollView(
      padding: pagePadding(context, widget.wide),
      child: Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 920),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              if (!widget.wide) const SizedBox(height: 32),
              PageTitle(t.settings),
              const SizedBox(height: 24),
              ZCard(
                padding: const EdgeInsets.symmetric(
                  horizontal: 18,
                  vertical: 4,
                ),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    _Row(
                      title: t.visible,
                      subtitle: t.visibleSub,
                      trailing: ZSwitch(
                        value: snapshot.visible,
                        label: t.visible,
                        onChanged: (v) => state.setSetting('visible', v),
                      ),
                    ),
                    const ZDivider(),
                    _Row(
                      title: t.confirmNew,
                      subtitle: t.confirmNewSub,
                      trailing: ZSwitch(
                        value: snapshot.confirmNew,
                        label: t.confirmNew,
                        onChanged: (v) => state.setSetting('confirm_new', v),
                      ),
                    ),
                    if (state.platform.clipboard != ClipboardSupport.none) ...[
                      const ZDivider(),
                      _Row(
                        title: t.clipboard,
                        subtitle: t.clipboardSub,
                        trailing: ZSwitch(
                          value: snapshot.clipboard,
                          label: t.clipboard,
                          onChanged: state.setClipboard,
                        ),
                      ),
                      if (snapshot.clipboard &&
                          state.platform.clipboard ==
                              ClipboardSupport.oneTap) ...[
                        const ZDivider(),
                        _Row(
                          title: t.clipTileTitle,
                          subtitle: t.clipTileSub,
                          trailing: ZButton(
                            label: t.clipTileAdd,
                            height: 44,
                            onPressed: state.addClipboardTile,
                          ),
                        ),
                        const ZDivider(),
                        _Row(
                          title: t.clipMenuTitle,
                          subtitle: t.clipMenuSub,
                          trailing: ZSwitch(
                            value: snapshot.ui['menu_entry'] == true,
                            label: t.clipMenuTitle,
                            onChanged: (v) => state.setUi('menu_entry', v),
                          ),
                        ),
                      ],
                      if (state.clipboardNeedsAllowing) ...[
                        const ZDivider(),
                        _Row(
                          title: t.clipAllowTitle,
                          subtitle: t.clipNeedsAllow,
                          trailing: ZButton(
                            label: t.clipAllow,
                            height: 44,
                            onPressed: state.platform.openClipboardSettings,
                          ),
                        ),
                      ],
                    ],
                    const ZDivider(),
                    _Row(
                      title: t.receiving,
                      subtitle: switch (snapshot.defaultMode) {
                        'ask' => t.modeAskSub,
                        'auto' => t.modeAutoSub,
                        _ => t.modeSmartSub,
                      },
                      trailing: SizedBox(
                        width: 260,
                        child: ZSelect<String>(
                          value: snapshot.defaultMode,
                          items: [
                            DropdownMenuItem(
                              value: 'ask',
                              child: modeItem('ask', t.modeAsk),
                            ),
                            DropdownMenuItem(
                              value: 'smart',
                              child: modeItem('smart', t.modeSmart),
                            ),
                            DropdownMenuItem(
                              value: 'auto',
                              child: modeItem('auto', t.modeAuto),
                            ),
                          ],
                          onChanged: (v) => state.setSetting('default_mode', v),
                        ),
                      ),
                    ),
                    const ZDivider(),
                    _Row(
                      title: t.folder,
                      subtitle: friendlyFolder(t, snapshot.folder),
                      trailing: Row(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          ZButton(
                            label: t.open,
                            height: 44,
                            onPressed: state.showFolder,
                          ),
                          if (state.platform.canPickFolders) ...[
                            const SizedBox(width: 8),
                            ZButton(
                              label: t.change,
                              height: 44,
                              onPressed: state.changeFolder,
                            ),
                          ],
                        ],
                      ),
                    ),
                  ],
                ),
              ),
              const SizedBox(height: 24),
              Wrap(
                spacing: 40,
                runSpacing: 24,
                children: [
                  Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        t.color,
                        style: zText(16, weight: 600, color: z.ink),
                      ),
                      const SizedBox(height: 12),
                      Row(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          for (final entry in {
                            'cobalt': t.cobalt,
                            'sunset': t.sunset,
                            'mint': t.mint,
                          }.entries)
                            Padding(
                              padding: const EdgeInsets.only(right: 20),
                              child: _Swatch(
                                id: entry.key,
                                label: entry.value,
                                selected: palette == entry.key,
                                onTap: () => state.setUi('palette', entry.key),
                              ),
                            ),
                        ],
                      ),
                    ],
                  ),
                  SizedBox(
                    width: 340,
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        Text(
                          t.appearance,
                          style: zText(16, weight: 600, color: z.ink),
                        ),
                        const SizedBox(height: 12),
                        _Segments(
                          value: theme,
                          options: {
                            'system': t.themeSystem,
                            'light': t.themeLight,
                            'dark': t.themeDark,
                          },
                          onChanged: (v) => state.setUi('theme', v),
                        ),
                      ],
                    ),
                  ),
                  SizedBox(
                    width: 340,
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        Text(
                          t.language,
                          style: zText(16, weight: 600, color: z.ink),
                        ),
                        const SizedBox(height: 12),
                        _Segments(
                          value: language,
                          options: {
                            'system': t.langSystem,
                            'ru': 'Русский',
                            'en': 'English',
                          },
                          onChanged: (v) =>
                              state.setUi('language', v == 'system' ? null : v),
                        ),
                      ],
                    ),
                  ),
                ],
              ),
              if (snapshot.me != null) ...[
                const SizedBox(height: 28),
                Text(t.thisDevice, style: zText(16, weight: 600, color: z.ink)),
                const SizedBox(height: 12),
                ZCard(
                  padding: const EdgeInsets.all(18),
                  child: _confirming
                      ? _confirmBox(context, state)
                      : _deviceBox(context, snapshot.me!),
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }

  Widget _deviceBox(BuildContext context, Me me) {
    final z = context.z;
    final t = L10n.of(context);
    return Wrap(
      alignment: WrapAlignment.spaceBetween,
      crossAxisAlignment: WrapCrossAlignment.center,
      spacing: 12,
      runSpacing: 10,
      children: [
        Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(me.name, style: zText(16, weight: 600, color: z.ink)),
            Text(
              t.deviceCode(me.shortId),
              style: zText(14, color: z.muted, tabular: true),
            ),
          ],
        ),
        ZButton(
          label: t.startOver,
          height: 44,
          kind: ZButtonKind.dangerSoft,
          onPressed: () => setState(() => _confirming = true),
        ),
      ],
    );
  }

  Widget _confirmBox(BuildContext context, AppState state) {
    final z = context.z;
    final t = L10n.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(t.startOverTitle, style: zText(17, weight: 600, color: z.ink)),
        const SizedBox(height: 6),
        Text(t.startOverBody, style: zText(15, color: z.muted, height: 1.4)),
        const SizedBox(height: 14),
        Wrap(
          spacing: 8,
          runSpacing: 8,
          children: [
            ZButton(
              label: t.cancelAction,
              height: 44,
              onPressed: () => setState(() => _confirming = false),
            ),
            ZButton(
              label: t.startOver,
              height: 44,
              kind: ZButtonKind.danger,
              onPressed: () {
                setState(() => _confirming = false);
                state.startOver(
                  title: t.ownerCheckTitle,
                  reason: t.ownerReasonStartOver,
                );
              },
            ),
          ],
        ),
      ],
    );
  }
}

class _Row extends StatelessWidget {
  const _Row({
    required this.title,
    required this.subtitle,
    required this.trailing,
  });

  final String title;
  final String subtitle;
  final Widget trailing;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    return ConstrainedBox(
      constraints: const BoxConstraints(minHeight: 68),
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: 10),
        child: LayoutBuilder(
          builder: (context, constraints) {
            final text = Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(title, style: zText(15, weight: 600, color: z.ink)),
                Text(subtitle, style: zText(14, color: z.muted, height: 1.35)),
              ],
            );
            if (constraints.maxWidth < 560 && trailing is! ZSwitch) {
              return Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [text, const SizedBox(height: 10), trailing],
              );
            }
            return Row(
              children: [
                Expanded(child: text),
                const SizedBox(width: 12),
                trailing,
              ],
            );
          },
        ),
      ),
    );
  }
}

class _Swatch extends StatelessWidget {
  const _Swatch({
    required this.id,
    required this.label,
    required this.selected,
    required this.onTap,
  });

  final String id;
  final String label;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final palette = palettes[id]!;
    return Semantics(
      selected: selected,
      button: true,
      label: label,
      excludeSemantics: true,
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(16),
        child: SizedBox(
          width: 72,
          child: Column(
            children: [
              const SizedBox(height: 5),
              Container(
                width: 44,
                height: 44,
                decoration: BoxDecoration(
                  shape: BoxShape.circle,
                  gradient: LinearGradient(
                    begin: Alignment.topLeft,
                    end: Alignment.bottomRight,
                    colors: [palette.ia, palette.ib],
                  ),
                  boxShadow: [
                    BoxShadow(color: z.bg, spreadRadius: 3),
                    BoxShadow(
                      color: selected ? z.ink : Colors.transparent,
                      spreadRadius: 5,
                    ),
                  ],
                ),
              ),
              const SizedBox(height: 12),
              Text(label, style: zText(14, weight: 500, color: z.ink)),
            ],
          ),
        ),
      ),
    );
  }
}

class _Segments extends StatelessWidget {
  const _Segments({
    required this.value,
    required this.options,
    required this.onChanged,
  });

  final String value;
  final Map<String, String> options;
  final ValueChanged<String> onChanged;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    return Container(
      padding: const EdgeInsets.all(4),
      decoration: BoxDecoration(
        color: z.surface2,
        borderRadius: BorderRadius.circular(14),
      ),
      child: Row(
        children: [
          for (final entry in options.entries)
            Expanded(
              child: Semantics(
                selected: entry.key == value,
                button: true,
                child: Material(
                  color: entry.key == value ? z.surface : Colors.transparent,
                  borderRadius: BorderRadius.circular(11),
                  child: InkWell(
                    onTap: () => onChanged(entry.key),
                    borderRadius: BorderRadius.circular(11),
                    child: SizedBox(
                      height: 44,
                      child: Center(
                        child: Text(
                          entry.value,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: zText(14, weight: 600, color: z.ink),
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            ),
        ],
      ),
    );
  }
}
