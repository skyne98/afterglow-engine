import { describe, expect, test } from 'bun:test';
import { controlBelongsToZone, selectTriangleCategory } from './zone-utils.ts';

describe('selectTriangleCategory', () => {
  test('uses the category with two vertices', () => {
    expect(selectTriangleCategory([3, 1, 3], [0.1, 1, 0.2])).toBe(3);
  });

  test('uses the strongest vertex when all categories differ', () => {
    expect(selectTriangleCategory([1, 2, 3], [0.1, 0.8, 0.4])).toBe(2);
  });

  test('ignores vertices without a zone', () => {
    expect(selectTriangleCategory([-1, 4, -1], [0, 0.2, 0])).toBe(4);
  });
});

describe('controlBelongsToZone', () => {
  test('includes direct controls', () => {
    expect(controlBelongsToZone({ category: 'torso', label: 'waist' }, 'torso')).toBe(true);
  });

  test('includes breast macro controls', () => {
    expect(controlBelongsToZone({ category: 'macro', label: 'Cup size' }, 'breast')).toBe(true);
    expect(controlBelongsToZone({ category: 'macro', label: 'Weight' }, 'breast')).toBe(false);
  });

  test('includes related asymmetry controls', () => {
    expect(controlBelongsToZone({ category: 'asymmetry', label: 'asymm-breast-1-l' }, 'breast')).toBe(true);
    expect(controlBelongsToZone({ category: 'asymmetry', label: 'asym-eye-1-r' }, 'eyes')).toBe(true);
  });
});
