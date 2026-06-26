import 'package:flutter/material.dart';

/// App logo: solid blue shield on black rounded square, matching the launcher icon.
class ScreenGuardLogo extends StatelessWidget {
  final double size;
  const ScreenGuardLogo({super.key, this.size = 72});

  @override
  Widget build(BuildContext context) {
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        color: Colors.black,
        borderRadius: BorderRadius.circular(size * 0.22),
      ),
      padding: EdgeInsets.all(size * 0.14),
      child: CustomPaint(painter: _ShieldPainter()),
    );
  }
}

class _ShieldPainter extends CustomPainter {
  @override
  void paint(Canvas canvas, Size size) {
    final s = size.width / 24.0;
    final paint = Paint()
      ..color = const Color(0xFF1A73E8)
      ..style = PaintingStyle.stroke
      ..strokeWidth = size.width * 0.07
      ..strokeJoin = StrokeJoin.round;

    // Material Design "shield" filled path (24×24 viewBox)
    // M12 1L3 5v6c0 5.55 3.84 10.74 9 12 5.16-1.26 9-6.45 9-12V5l-9-4z
    final path = Path()
      ..moveTo(12 * s, 1 * s)
      ..lineTo(3 * s, 5 * s)
      ..relativeLineTo(0, 6 * s)
      ..relativeCubicTo(0, 5.55 * s, 3.84 * s, 10.74 * s, 9 * s, 12 * s)
      ..relativeCubicTo(5.16 * s, -1.26 * s, 9 * s, -6.45 * s, 9 * s, -12 * s)
      ..lineTo(21 * s, 5 * s)
      ..relativeLineTo(-9 * s, -4 * s)
      ..close();

    canvas.drawPath(path, paint);
  }

  @override
  bool shouldRepaint(_ShieldPainter _) => false;
}
