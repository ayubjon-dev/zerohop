import 'package:flutter/material.dart';

import '../theme.dart';
import 'icons.dart';

enum ZButtonKind { primary, secondary, danger, dangerSoft, onGradient }

class ZButton extends StatelessWidget {
  const ZButton({
    super.key,
    required this.label,
    required this.onPressed,
    this.kind = ZButtonKind.secondary,
    this.height = 48,
    this.expand = false,
  });

  final String label;
  final VoidCallback? onPressed;
  final ZButtonKind kind;
  final double height;
  final bool expand;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    final (Color background, Color foreground) = switch (kind) {
      ZButtonKind.primary => (z.accent, z.on),
      ZButtonKind.secondary => (z.surface2, z.ink),
      ZButtonKind.danger => (z.danger, z.bg),
      ZButtonKind.dangerSoft => (z.dangerSoft, z.danger),
      ZButtonKind.onGradient => (
        Colors.white.withValues(alpha: 0.2),
        Colors.white,
      ),
    };
    final enabled = onPressed != null;
    final radius = BorderRadius.circular(height >= 56 ? 18 : 14);
    final body = SizedBox(
      height: height,
      child: Padding(
        padding: EdgeInsets.symmetric(horizontal: expand ? 16 : 24),
        child: Center(
          // Without a factor the button would take all the width it is offered.
          widthFactor: 1,
          child: Text(
            label,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: zText(
              height >= 56 ? 17 : 15,
              weight: 600,
              color: foreground,
            ),
          ),
        ),
      ),
    );
    final button = Opacity(
      opacity: enabled ? 1 : 0.5,
      child: Material(
        color: background,
        borderRadius: radius,
        child: InkWell(onTap: onPressed, borderRadius: radius, child: body),
      ),
    );
    return expand ? SizedBox(width: double.infinity, child: button) : button;
  }
}

class ZCard extends StatelessWidget {
  const ZCard({super.key, required this.child, this.padding, this.radius = 20});

  final Widget child;
  final EdgeInsetsGeometry? padding;
  final double radius;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: padding,
      clipBehavior: Clip.antiAlias,
      decoration: BoxDecoration(
        color: context.z.surface,
        borderRadius: BorderRadius.circular(radius),
      ),
      child: child,
    );
  }
}

class ZDivider extends StatelessWidget {
  const ZDivider({super.key});

  @override
  Widget build(BuildContext context) =>
      Container(height: 1, color: context.z.line);
}

class ZSwitch extends StatelessWidget {
  const ZSwitch({
    super.key,
    required this.value,
    required this.onChanged,
    required this.label,
  });

  final bool value;
  final ValueChanged<bool> onChanged;
  final String label;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    return Semantics(
      toggled: value,
      label: label,
      child: InkWell(
        onTap: () => onChanged(!value),
        borderRadius: BorderRadius.circular(16),
        child: AnimatedContainer(
          duration: const Duration(milliseconds: 150),
          width: 52,
          height: 32,
          padding: const EdgeInsets.all(3),
          alignment: value ? Alignment.centerRight : Alignment.centerLeft,
          decoration: BoxDecoration(
            color: value ? z.accent : z.line,
            borderRadius: BorderRadius.circular(16),
          ),
          child: Container(
            width: 26,
            height: 26,
            decoration: const BoxDecoration(
              color: Colors.white,
              shape: BoxShape.circle,
            ),
          ),
        ),
      ),
    );
  }
}

class ZSelect<T> extends StatelessWidget {
  const ZSelect({
    super.key,
    required this.value,
    required this.items,
    required this.onChanged,
  });

  final T value;
  final List<DropdownMenuItem<T>> items;
  final ValueChanged<T> onChanged;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    return Container(
      height: 44,
      padding: const EdgeInsets.symmetric(horizontal: 12),
      decoration: BoxDecoration(
        color: z.surface2,
        borderRadius: BorderRadius.circular(12),
      ),
      child: DropdownButtonHideUnderline(
        child: DropdownButton<T>(
          value: value,
          items: items,
          isExpanded: true,
          dropdownColor: z.surface,
          borderRadius: BorderRadius.circular(12),
          iconEnabledColor: z.muted,
          style: zText(15, color: z.ink),
          onChanged: (next) {
            if (next != null) onChanged(next);
          },
        ),
      ),
    );
  }
}

class StatusDot extends StatelessWidget {
  const StatusDot({super.key, required this.on, this.size = 8});

  final bool on;
  final double size;

  @override
  Widget build(BuildContext context) {
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        shape: BoxShape.circle,
        color: on ? const Color(0xFF22B573) : context.z.muted,
      ),
    );
  }
}

/// A round badge: filled with the gradient when lit, quiet when not.
class IconCircle extends StatelessWidget {
  const IconCircle({
    super.key,
    required this.icon,
    required this.size,
    required this.iconSize,
    this.lit = true,
    this.stroke = 1.9,
  });

  final String icon;
  final double size;
  final double iconSize;
  final bool lit;
  final double stroke;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    return Container(
      width: size,
      height: size,
      alignment: Alignment.center,
      decoration: BoxDecoration(
        shape: BoxShape.circle,
        color: lit ? null : z.surface2,
        gradient: lit ? z.gradient : null,
      ),
      child: ZIcon(
        icon,
        size: iconSize,
        stroke: stroke,
        color: lit ? Colors.white : z.muted,
      ),
    );
  }
}

/// A rounded square holding an icon, for rows.
class IconTile extends StatelessWidget {
  const IconTile({
    super.key,
    required this.icon,
    this.size = 42,
    this.radius = 12,
  });

  final String icon;
  final double size;
  final double radius;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    return Container(
      width: size,
      height: size,
      alignment: Alignment.center,
      decoration: BoxDecoration(
        color: z.soft,
        borderRadius: BorderRadius.circular(radius),
      ),
      child: ZIcon(icon, size: size / 2, color: z.accent),
    );
  }
}

class Logo extends StatelessWidget {
  const Logo({super.key, this.size = 36});

  final double size;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(size * 0.28),
        gradient: LinearGradient(
          begin: Alignment.topLeft,
          end: Alignment.bottomRight,
          colors: [z.ia, z.ib],
        ),
      ),
      child: ZGlyph(size: size),
    );
  }
}

class IconButtonZ extends StatelessWidget {
  const IconButtonZ({
    super.key,
    required this.icon,
    required this.label,
    required this.onPressed,
    this.size = 44,
    this.iconSize = 22,
    this.stroke = 2.2,
    this.color,
  });

  final String icon;
  final String label;
  final VoidCallback onPressed;
  final double size;
  final double iconSize;
  final double stroke;
  final Color? color;

  @override
  Widget build(BuildContext context) {
    return Tooltip(
      message: label,
      child: Semantics(
        button: true,
        label: label,
        child: InkWell(
          onTap: onPressed,
          borderRadius: BorderRadius.circular(size / 2),
          child: SizedBox(
            width: size,
            height: size,
            child: Center(
              child: ZIcon(
                icon,
                size: iconSize,
                stroke: stroke,
                color: color ?? context.z.ink,
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class PageTitle extends StatelessWidget {
  const PageTitle(this.text, {super.key});

  final String text;

  @override
  Widget build(BuildContext context) {
    return Text(
      text,
      style: zText(
        34,
        weight: 700,
        height: 1.15,
        letterSpacing: -0.025,
        color: context.z.ink,
      ),
    );
  }
}

class BackBar extends StatelessWidget {
  const BackBar({
    super.key,
    required this.title,
    required this.label,
    required this.onBack,
    this.subtitle,
    this.trailing,
  });

  final String title;
  final String? subtitle;
  final String label;
  final VoidCallback onBack;
  final Widget? trailing;

  @override
  Widget build(BuildContext context) {
    final z = context.z;
    return SizedBox(
      height: 48,
      child: Row(
        children: [
          Transform.translate(
            offset: const Offset(-12, 0),
            child: IconButtonZ(
              icon: ZIcons.back,
              label: label,
              onPressed: onBack,
              iconSize: 24,
            ),
          ),
          Expanded(
            child: Column(
              mainAxisAlignment: MainAxisAlignment.center,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  title,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: zText(20, weight: 700, height: 1.2, color: z.ink),
                ),
                if (subtitle != null)
                  Text(
                    subtitle!,
                    style: zText(13, color: z.muted, tabular: true),
                  ),
              ],
            ),
          ),
          ?trailing,
        ],
      ),
    );
  }
}
