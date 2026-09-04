import { describe, expect, test } from 'bun:test';
import { hsvToRgb, parseHexColor, rgbToHex, rgbToHsv } from './paint-color.ts';

describe('paint color conversion', () => {
  test('parses short and full hex colors', () => {
    expect(parseHexColor('#4ecdc4')).toEqual({ r: 78, g: 205, b: 196 });
    expect(parseHexColor('0af')).toEqual({ r: 0, g: 170, b: 255 });
    expect(parseHexColor('no-color')).toBeNull();
  });

  test('converts standard HSV colors to RGB', () => {
    expect(hsvToRgb({ h: 0, s: 100, v: 100 })).toEqual({ r: 255, g: 0, b: 0 });
    expect(hsvToRgb({ h: 120, s: 100, v: 100 })).toEqual({ r: 0, g: 255, b: 0 });
    expect(hsvToRgb({ h: 240, s: 100, v: 100 })).toEqual({ r: 0, g: 0, b: 255 });
  });

  test('keeps RGB values through HSV conversion', () => {
    for (const color of ['#000000', '#ffffff', '#4ecdc4', '#b51f68']) {
      const rgb = parseHexColor(color)!;
      expect(rgbToHex(hsvToRgb(rgbToHsv(rgb)))).toBe(color);
    }
  });
});
