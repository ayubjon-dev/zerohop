import 'package:flutter/material.dart';

import '../../l10n/app_localizations.dart';
import '../app_state.dart';
import '../theme.dart';
import 'widgets.dart';

class Onboarding extends StatefulWidget {
  const Onboarding({super.key, required this.state});

  final AppState state;

  @override
  State<Onboarding> createState() => _OnboardingState();
}

class _OnboardingState extends State<Onboarding> {
  late final TextEditingController _name;

  @override
  void initState() {
    super.initState();
    _name = TextEditingController()..addListener(() => setState(() {}));
    widget.state.platform.suggestedName().then((suggestion) {
      if (mounted && _name.text.isEmpty) _name.text = suggestion;
    });
  }

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  void _submit() {
    if (_name.text.trim().isNotEmpty) widget.state.setName(_name.text);
  }

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final t = L10n.of(context);
    return Center(
      child: SingleChildScrollView(
        padding: const EdgeInsets.all(24),
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 420),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              const Logo(size: 64),
              const SizedBox(height: 28),
              Text(
                t.onboardTitle,
                style: zText(
                  30,
                  weight: 700,
                  height: 1.15,
                  letterSpacing: -0.02,
                  color: z.ink,
                ),
              ),
              const SizedBox(height: 10),
              Text(t.onboardHint, style: zText(16, color: z.muted)),
              const SizedBox(height: 24),
              ZTextField(
                controller: _name,
                label: t.nameLabel,
                onSubmitted: (_) => _submit(),
                maxLength: 40,
              ),
              const SizedBox(height: 16),
              ZButton(
                label: t.continueAction,
                kind: ZButtonKind.primary,
                height: 56,
                expand: true,
                onPressed: _name.text.trim().isEmpty ? null : _submit,
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class ZTextField extends StatelessWidget {
  const ZTextField({
    super.key,
    required this.controller,
    required this.label,
    this.onSubmitted,
    this.maxLength,
    this.autofocus = true,
  });

  final TextEditingController controller;
  final String label;
  final ValueChanged<String>? onSubmitted;
  final int? maxLength;
  final bool autofocus;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    return TextField(
      controller: controller,
      autofocus: autofocus,
      maxLength: maxLength,
      onSubmitted: onSubmitted,
      style: zText(17, weight: 500, color: z.ink),
      decoration: InputDecoration(
        labelText: label,
        counterText: '',
        labelStyle: zText(15, color: z.muted),
        filled: true,
        fillColor: z.surface,
        contentPadding: const EdgeInsets.symmetric(
          horizontal: 16,
          vertical: 18,
        ),
        border: OutlineInputBorder(
          borderRadius: BorderRadius.circular(16),
          borderSide: BorderSide.none,
        ),
        focusedBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(16),
          borderSide: BorderSide(color: z.accent, width: 2),
        ),
      ),
    );
  }
}
