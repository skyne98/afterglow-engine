import { expect, test } from 'bun:test';
import { inversePaintView } from './paint-view.ts';

test('pointer coordinates invert every paint view transform', () => {
  for (const zoom of [0.1, 1, 8]) {
    for (const degrees of [-135, 0, 45, 90, 360]) {
      for (const mirror of [false, true]) {
        const x = -123.5, y = 48.25;
        const angle = degrees * Math.PI / 180;
        const mirroredX = x * (mirror ? -1 : 1);
        const screenX = zoom * (Math.cos(angle) * mirroredX - Math.sin(angle) * y);
        const screenY = zoom * (Math.sin(angle) * mirroredX + Math.cos(angle) * y);
        const point = inversePaintView(screenX, screenY, zoom, degrees, mirror);
        expect(point[0]).toBeCloseTo(x, 8);
        expect(point[1]).toBeCloseTo(y, 8);
      }
    }
  }
});
