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
