export interface RgbColor {
  r: number;
  g: number;
  b: number;
}

export interface HsvColor {
  h: number;
  s: number;
  v: number;
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(maximum, Math.max(minimum, Number.isFinite(value) ? value : minimum));
}

export function parseHexColor(value: string): RgbColor | null {
  const match = value.trim().match(/^#?([0-9a-f]{3}|[0-9a-f]{6})$/i);
  if (!match) return null;
  const digits = match[1].length === 3
    ? [...match[1]].map((digit) => digit + digit).join('')
    : match[1];
  const number = Number.parseInt(digits, 16);
  return { r: number >> 16, g: (number >> 8) & 255, b: number & 255 };
}

export function rgbToHex({ r, g, b }: RgbColor): string {
  const byte = (value: number) => Math.round(clamp(value, 0, 255)).toString(16).padStart(2, '0');
  return `#${byte(r)}${byte(g)}${byte(b)}`;
}

export function rgbToHsv({ r, g, b }: RgbColor): HsvColor {
  const red = clamp(r, 0, 255) / 255;
  const green = clamp(g, 0, 255) / 255;
  const blue = clamp(b, 0, 255) / 255;
  const maximum = Math.max(red, green, blue);
  const minimum = Math.min(red, green, blue);
  const delta = maximum - minimum;
  let hue = 0;
  if (delta !== 0) {
    if (maximum === red) hue = ((green - blue) / delta) % 6;
    else if (maximum === green) hue = (blue - red) / delta + 2;
    else hue = (red - green) / delta + 4;
    hue *= 60;
    if (hue < 0) hue += 360;
  }
  return {
    h: hue,
    s: maximum === 0 ? 0 : (delta / maximum) * 100,
    v: maximum * 100,
  };
}

export function hsvToRgb({ h, s, v }: HsvColor): RgbColor {
  const hue = ((Number.isFinite(h) ? h : 0) % 360 + 360) % 360;
  const saturation = clamp(s, 0, 100) / 100;
  const value = clamp(v, 0, 100) / 100;
  const chroma = value * saturation;
  const part = hue / 60;
  const x = chroma * (1 - Math.abs((part % 2) - 1));
  let red = 0;
  let green = 0;
  let blue = 0;
  if (part < 1) [red, green] = [chroma, x];
  else if (part < 2) [red, green] = [x, chroma];
  else if (part < 3) [green, blue] = [chroma, x];
  else if (part < 4) [green, blue] = [x, chroma];
  else if (part < 5) [red, blue] = [x, chroma];
  else [red, blue] = [chroma, x];
  const match = value - chroma;
  return {
    r: Math.round((red + match) * 255),
    g: Math.round((green + match) * 255),
    b: Math.round((blue + match) * 255),
  };
}
