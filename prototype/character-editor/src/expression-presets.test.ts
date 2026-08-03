import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import {
  EXPRESSION_PRESETS,
  META_VISEMES,
  MICROSOFT_VISEMES,
  SPEECH_PRESETS,
} from './expression-presets.ts';

const morphNames = new Set<string>(JSON.parse(
  readFileSync(join(import.meta.dir, '..', 'public', 'character_female.morphs.json'), 'utf8'),
));

describe('face preview presets', () => {
  test('has unique names and the complete speech library', () => {
    const all = [...EXPRESSION_PRESETS, ...SPEECH_PRESETS];
    expect(new Set(all.map((preset) => preset.name)).size).toBe(all.length);
    expect(META_VISEMES.length).toBe(14);
    expect(MICROSOFT_VISEMES.length).toBe(21);
  });

  test('uses available morphs and bounded weights', () => {
    for (const preset of [...EXPRESSION_PRESETS, ...SPEECH_PRESETS]) {
      for (const [target, weight] of Object.entries(preset.weights)) {
        expect(morphNames.has(target), `${preset.name}: ${target}`).toBe(true);
        expect(weight, `${preset.name}: ${target}`).toBeGreaterThan(0);
        expect(weight, `${preset.name}: ${target}`).toBeLessThanOrEqual(1);
      }
    }
  });

  test('has neutral reset entries', () => {
    expect(Object.keys(EXPRESSION_PRESETS[0].weights)).toHaveLength(0);
    expect(Object.keys(SPEECH_PRESETS[0].weights)).toHaveLength(0);
  });
});
