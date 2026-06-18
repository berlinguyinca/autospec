// skills/autospec-harmonize/scripts/lib/color.mjs
//
// Pure color-math helpers for autospec-harmonize (hex ↔ HSL, luminance,
// chromaticity, circular hue distance). No I/O — safe to import anywhere.

/** Parse "#rrggbb" → [r, g, b] in [0,255]. */
export function hexToRgb(hex) {
  const h = hex.replace('#', '');
  return [
    parseInt(h.slice(0, 2), 16),
    parseInt(h.slice(2, 4), 16),
    parseInt(h.slice(4, 6), 16),
  ];
}

/** [r,g,b] in [0,255] → [h,s,l] where h∈[0,360), s∈[0,1], l∈[0,1]. */
export function rgbToHsl(r, g, b) {
  r /= 255; g /= 255; b /= 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  if (max === min) return [0, 0, l]; // achromatic
  const d = max - min;
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
  let h;
  switch (max) {
    case r: h = ((g - b) / d + (g < b ? 6 : 0)) / 6; break;
    case g: h = ((b - r) / d + 2) / 6; break;
    default: h = ((r - g) / d + 4) / 6; break;
  }
  return [h * 360, s, l];
}

/** Return [h, s, l] for a "#rrggbb" hex string. */
export function hexToHsl(hex) {
  return rgbToHsl(...hexToRgb(hex));
}

/** Luminance proxy: l component from HSL (0=dark, 1=light). */
export function luminance(hex) {
  return hexToHsl(hex)[2];
}

/** Is this hex "chromatic" (saturation >= 0.15)? */
export function isChromatic(hex) {
  const [, s] = hexToHsl(hex);
  return s >= 0.15;
}

/** Circular hue distance in degrees. */
export function hueDist(a, b) {
  const d = Math.abs(a - b) % 360;
  return d > 180 ? 360 - d : d;
}

// ---------------------------------------------------------------------------
// WCAG contrast helpers (shared by design-variants and design-generalize)
// ---------------------------------------------------------------------------

/** sRGB channel [0,255] → linear light value. */
export function linearize(c) {
  const s = c / 255;
  return s <= 0.04045 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
}

/** WCAG relative luminance of a "#rrggbb" hex. */
export function wcagLuminance(hex) {
  const [r, g, b] = hexToRgb(hex);
  return 0.2126 * linearize(r) + 0.7152 * linearize(g) + 0.0722 * linearize(b);
}

/** WCAG contrast ratio between two hex colors. */
export function contrastRatio(hexA, hexB) {
  const L1 = wcagLuminance(hexA);
  const L2 = wcagLuminance(hexB);
  const lighter = Math.max(L1, L2);
  const darker  = Math.min(L1, L2);
  return (lighter + 0.05) / (darker + 0.05);
}

/** Pick the background entry: the one tagged role:"bg", else the lightest hex. */
export function pickBackground(palette) {
  if (!palette.length) return null;
  const tagged = palette.find(e => e.role === 'bg');
  if (tagged) return tagged;
  return palette.reduce((a, b) => (wcagLuminance(b.hex) > wcagLuminance(a.hex) ? b : a));
}

/**
 * Readability metric: the minimum WCAG contrast of each foreground color
 * against the background. Contrast is inherently foreground-vs-background, so
 * this — not min-pairwise-across-all-colors — is what "high contrast" must
 * improve. The high-contrast transform pushes foregrounds away from the bg
 * luminance, which monotonically raises this value (no fudge floor needed).
 */
export function minForegroundContrast(palette) {
  const bg = pickBackground(palette);
  if (!bg) return 1;
  const fg = palette.filter(e => e !== bg);
  if (!fg.length) return 1;
  let min = Infinity;
  for (const e of fg) {
    const r = contrastRatio(e.hex, bg.hex);
    if (r < min) min = r;
  }
  return min === Infinity ? 1 : min;
}
