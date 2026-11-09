import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../l10n/app_localizations.dart';
import '../app_state.dart';
import '../model.dart';
import '../theme.dart';
import 'icons.dart';
import 'onboarding.dart';
import 'widgets.dart';

class _PageFrame extends StatelessWidget {
  const _PageFrame({
    required this.wide,
    required this.title,
    required this.onBack,
    required this.child,
  });

  final bool wide;
  final String title;
  final VoidCallback onBack;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final t = L10n.of(context);
    return Padding(
      padding: EdgeInsets.fromLTRB(20, wide ? 32 : 8, 20, 20),
      child: Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 520),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              BackBar(title: title, label: t.back, onBack: onBack),
              const SizedBox(height: 16),
              Expanded(child: child),
            ],
          ),
        ),
      ),
    );
  }
}

class PairChoicePage extends StatelessWidget {
  const PairChoicePage({super.key, required this.state, required this.wide});

  final AppState state;
  final bool wide;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    Widget option(String icon, String title, String sub, VoidCallback onTap) {
      return InkWell(
        onTap: onTap,
        child: Padding(
          padding: const EdgeInsets.all(14),
          child: Row(
            children: [
              IconTile(icon: icon, size: 48, radius: 14),
              const SizedBox(width: 14),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(title, style: zText(17, weight: 600, color: z.ink)),
                    Text(sub, style: zText(14, color: z.muted)),
                  ],
                ),
              ),
            ],
          ),
        ),
      );
    }

    return _PageFrame(
      wide: wide,
      title: t.addDevice,
      onBack: () => state.open(Screen.home),
      child: SingleChildScrollView(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(t.pairIntro, style: zText(16, color: z.muted, height: 1.4)),
            const SizedBox(height: 16),
            ZCard(
              child: Column(
                children: [
                  option(
                    ZIcons.qr,
                    t.showCode,
                    t.showCodeSub,
                    state.startHosting,
                  ),
                  const ZDivider(),
                  option(
                    ZIcons.hash,
                    t.enterCode,
                    t.enterCodeSub,
                    state.startEntering,
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class PairShowPage extends StatefulWidget {
  const PairShowPage({
    super.key,
    required this.state,
    required this.pairing,
    required this.wide,
  });

  final AppState state;
  final Pairing? pairing;
  final bool wide;

  @override
  State<PairShowPage> createState() => _PairShowPageState();
}

class _PairShowPageState extends State<PairShowPage> {
  Timer? _timer;

  @override
  void initState() {
    super.initState();
    _timer = Timer.periodic(const Duration(seconds: 1), (_) {
      if (mounted) setState(() {});
    });
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    final pairing = widget.pairing;
    final code = pairing?.code ?? '';
    final seconds = pairing == null
        ? 0
        : ((pairing.deadlineMs - DateTime.now().millisecondsSinceEpoch) / 1000)
              .ceil()
              .clamp(0, 120);
    return _PageFrame(
      wide: widget.wide,
      title: t.codeTitle,
      onBack: widget.state.cancelPairing,
      child: Center(
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Semantics(
                label: code,
                excludeSemantics: true,
                child: FittedBox(
                  fit: BoxFit.scaleDown,
                  child: Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      for (var i = 0; i < code.length; i++)
                        Padding(
                          padding: EdgeInsets.only(
                            left: i == 0 ? 0 : (i == 3 ? 16 : 8),
                          ),
                          child: Container(
                            width: 52,
                            height: 64,
                            alignment: Alignment.center,
                            decoration: BoxDecoration(
                              color: z.surface,
                              borderRadius: BorderRadius.circular(14),
                            ),
                            child: Text(
                              code[i],
                              style: zText(
                                30,
                                weight: 700,
                                color: z.ink,
                                tabular: true,
                              ),
                            ),
                          ),
                        ),
                    ],
                  ),
                ),
              ),
              const SizedBox(height: 18),
              if (pairing != null)
                Text(
                  t.roomLine(pairing.room ?? '', seconds),
                  textAlign: TextAlign.center,
                  style: zText(15, color: z.muted, tabular: true),
                ),
              const SizedBox(height: 18),
              Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  SizedBox(
                    width: 16,
                    height: 16,
                    child: CircularProgressIndicator(
                      strokeWidth: 2,
                      color: z.accent,
                    ),
                  ),
                  const SizedBox(width: 10),
                  Text(
                    t.waitingForDevice,
                    style: zText(15, weight: 500, color: z.ink),
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

class PairEnterPage extends StatefulWidget {
  const PairEnterPage({super.key, required this.state, required this.wide});

  final AppState state;
  final bool wide;

  @override
  State<PairEnterPage> createState() => _PairEnterPageState();
}

class _PairEnterPageState extends State<PairEnterPage> {
  final _code = TextEditingController();
  final _address = TextEditingController();
  String? _room;
  bool _manual = false;

  @override
  void initState() {
    super.initState();
    _code.addListener(_refresh);
    _address.addListener(_refresh);
  }

  void _refresh() => setState(() {});

  @override
  void dispose() {
    _code.dispose();
    _address.dispose();
    super.dispose();
  }

  String? _target(List<Room> rooms) {
    if (_manual) {
      final typed = _address.text.trim();
      return typed.isEmpty ? null : typed;
    }
    final selected = rooms.where((r) => r.instance == _room).firstOrNull;
    final room = selected ?? (rooms.length == 1 ? rooms.first : null);
    return room?.address;
  }

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    return _PageFrame(
      wide: widget.wide,
      title: t.enterTitle,
      onBack: () => widget.state.open(Screen.pair),
      child: ValueListenableBuilder<List<Room>>(
        valueListenable: widget.state.discovery.rooms,
        builder: (context, rooms, _) {
          final target = _target(rooms);
          final ready = _code.text.length == 6 && target != null;
          return Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Expanded(
                child: SingleChildScrollView(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      const SizedBox(height: 12),
                      Text(t.enterLabel, style: zText(15, color: z.muted)),
                      const SizedBox(height: 10),
                      TextField(
                        controller: _code,
                        autofocus: true,
                        keyboardType: TextInputType.number,
                        textAlign: TextAlign.center,
                        inputFormatters: [
                          FilteringTextInputFormatter.digitsOnly,
                          LengthLimitingTextInputFormatter(6),
                        ],
                        style: zText(
                          36,
                          weight: 700,
                          letterSpacing: 0.25,
                          color: z.ink,
                          tabular: true,
                        ),
                        decoration: InputDecoration(
                          filled: true,
                          fillColor: z.surface,
                          contentPadding: const EdgeInsets.symmetric(
                            vertical: 18,
                          ),
                          border: OutlineInputBorder(
                            borderRadius: BorderRadius.circular(18),
                            borderSide: BorderSide.none,
                          ),
                          focusedBorder: OutlineInputBorder(
                            borderRadius: BorderRadius.circular(18),
                            borderSide: BorderSide(color: z.accent, width: 2),
                          ),
                        ),
                      ),
                      const SizedBox(height: 20),
                      if (!_manual) ...[
                        Text(
                          t.roomsTitle,
                          style: zText(15, weight: 600, color: z.ink),
                        ),
                        const SizedBox(height: 8),
                        if (rooms.isEmpty)
                          Row(
                            children: [
                              SizedBox(
                                width: 16,
                                height: 16,
                                child: CircularProgressIndicator(
                                  strokeWidth: 2,
                                  color: z.accent,
                                ),
                              ),
                              const SizedBox(width: 10),
                              Expanded(
                                child: Text(
                                  t.roomsSearching,
                                  style: zText(15, color: z.muted),
                                ),
                              ),
                            ],
                          )
                        else
                          Wrap(
                            spacing: 8,
                            runSpacing: 8,
                            children: [
                              for (final room in rooms)
                                ZButton(
                                  label: t.room(room.label),
                                  height: 44,
                                  kind:
                                      (_room == room.instance ||
                                          (rooms.length == 1 && _room == null))
                                      ? ZButtonKind.primary
                                      : ZButtonKind.secondary,
                                  onPressed: () =>
                                      setState(() => _room = room.instance),
                                ),
                            ],
                          ),
                        const SizedBox(height: 12),
                        Align(
                          alignment: Alignment.centerLeft,
                          child: TextButton(
                            onPressed: () => setState(() => _manual = true),
                            child: Text(
                              t.manualAddress,
                              style: zText(14, weight: 600, color: z.accent),
                            ),
                          ),
                        ),
                      ] else
                        ZTextField(
                          controller: _address,
                          label: t.addressLabel,
                          autofocus: false,
                        ),
                    ],
                  ),
                ),
              ),
              const SizedBox(height: 12),
              ZButton(
                label: t.continueAction,
                kind: ZButtonKind.primary,
                height: 56,
                expand: true,
                onPressed: ready
                    ? () => widget.state.joinWith(target, _code.text)
                    : null,
              ),
            ],
          );
        },
      ),
    );
  }
}

class ConnectingPage extends StatelessWidget {
  const ConnectingPage({super.key});

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          SizedBox(
            width: 32,
            height: 32,
            child: CircularProgressIndicator(strokeWidth: 3, color: z.accent),
          ),
          const SizedBox(height: 16),
          Text(t.connecting, style: zText(17, weight: 500, color: z.ink)),
        ],
      ),
    );
  }
}

class ComparePage extends StatelessWidget {
  const ComparePage({super.key, required this.state, required this.pairing});

  final AppState state;
  final Pairing pairing;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    final number = pairing.comparison ?? '';
    final grouped = number.length == 6
        ? '${number.substring(0, 3)} ${number.substring(3)}'
        : number;
    final finishing = pairing.phase == 'finishing';
    return Center(
      child: SingleChildScrollView(
        padding: const EdgeInsets.all(20),
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 480),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Text(
                t.compareTitle,
                textAlign: TextAlign.center,
                style: zText(
                  34,
                  weight: 700,
                  height: 1.15,
                  letterSpacing: -0.025,
                  color: z.ink,
                ),
              ),
              const SizedBox(height: 14),
              Text(
                t.compareSub,
                textAlign: TextAlign.center,
                style: zText(16, color: z.muted),
              ),
              const SizedBox(height: 8),
              FittedBox(
                fit: BoxFit.scaleDown,
                child: Text(
                  grouped,
                  style: zText(
                    72,
                    weight: 700,
                    letterSpacing: 0.02,
                    color: z.ink,
                    tabular: true,
                  ),
                ),
              ),
              const SizedBox(height: 8),
              ZCard(
                radius: 18,
                padding: const EdgeInsets.symmetric(
                  horizontal: 16,
                  vertical: 12,
                ),
                child: Row(
                  children: [
                    IconCircle(
                      icon: ZIcons.forPlatform(pairing.peerPlatform ?? 'other'),
                      size: 44,
                      iconSize: 22,
                    ),
                    const SizedBox(width: 12),
                    Expanded(
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Text(t.otherDevice, style: zText(14, color: z.muted)),
                          Text(
                            pairing.peerName ?? '',
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: zText(16, weight: 600, color: z.ink),
                          ),
                        ],
                      ),
                    ),
                    Text(
                      pairing.peerShortId ?? '',
                      style: zText(13, color: z.muted, tabular: true),
                    ),
                  ],
                ),
              ),
              const SizedBox(height: 22),
              if (finishing)
                Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    SizedBox(
                      width: 16,
                      height: 16,
                      child: CircularProgressIndicator(
                        strokeWidth: 2,
                        color: z.accent,
                      ),
                    ),
                    const SizedBox(width: 10),
                    Text(
                      t.finishing,
                      style: zText(15, weight: 500, color: z.ink),
                    ),
                  ],
                )
              else
                Wrap(
                  spacing: 10,
                  runSpacing: 10,
                  alignment: WrapAlignment.center,
                  children: [
                    ZButton(
                      label: t.numbersDiffer,
                      onPressed: () => state.answerPairing(false),
                    ),
                    ZButton(
                      label: t.numbersMatch,
                      kind: ZButtonKind.primary,
                      onPressed: () => state.answerPairing(true),
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
