import { readFileSync } from 'fs';
import {
  ARKIT_BLENDSHAPES,
  DEAD_BLENDSHAPES,
  BlendshapeSmoother,
  applyBlendshapes,
  buildBlendshapeMapping,
} from './face-tracker.ts';

/** The generated character must own all 52 ARKit shapes (regression gate). */
function loadCharacterMorphNames(sex: 'male' | 'female'): string[] {
  return JSON.parse(readFileSync(`public/character_${sex}.morphs.json`, 'utf8')) as string[];
}

function sampleMorphNames(extra: string[] = []): string[] {
  const names = [
    'jawOpen', 'jawForward', 'mouthSmileLeft', 'mouthSmileRight',
    'eyeBlinkLeft', 'eyeBlinkRight', 'browDownLeft', 'browDownRight',
    'tongueOut', 'cheekPuff', 'mouthDimpleLeft',
    'penis-length-incr', 'breast-cup-incr', 'height', // non-face morphs
  ];
  for (const name of extra) if (!names.includes(name)) names.push(name);
  return names;
}

describe('buildBlendshapeMapping', () => {
  test('maps the 52 ARKit names onto the generated character morphs 1:1', () => {
    for (const sex of ['male', 'female'] as const) {
      const names = loadCharacterMorphNames(sex);
      const mapping = buildBlendshapeMapping(names);
      expect(mapping.count).toBe(52);
      // Every ARKit name resolves to a distinct glTF index.
      const seen = new Set<number>();
      for (const name of ARKIT_BLENDSHAPES) {
        const index = names.indexOf(name);
        expect(index).toBeGreaterThanOrEqual(0);
        expect(seen.has(index)).toBe(false);
        seen.add(index);
      }
    }
  });

  test('marks dead shapes and leaves absent shapes unmapped', () => {
    const mapping = buildBlendshapeMapping(sampleMorphNames());
    const categoryOf = (name: string) => ARKIT_BLENDSHAPES.indexOf(name);
    expect(mapping.deadByCategory[categoryOf('tongueOut')]).toBe(1);
    expect(mapping.deadByCategory[categoryOf('cheekPuff')]).toBe(1);
    expect(mapping.deadByCategory[categoryOf('jawForward')]).toBe(1);
    expect(mapping.deadByCategory[categoryOf('jawOpen')]).toBe(0);
    // 'mouthDimpleRight' is absent from the sample list.
    expect(mapping.morphIndexByCategory[categoryOf('mouthDimpleRight')]).toBe(-1);
    expect(mapping.morphIndexByCategory[categoryOf('jawOpen')]).toBeGreaterThanOrEqual(0);
  });

  test('reports the mapped count', () => {
    expect(buildBlendshapeMapping(sampleMorphNames()).count).toBe(11);
  });
});

describe('applyBlendshapes', () => {
  const categories = (entries: Array<[string, number]>) =>
    entries.map(([categoryName, score]) => ({ categoryName, score }));

  test('applies live shapes by name and skips dead ones', () => {
    const mapping = buildBlendshapeMapping(sampleMorphNames());
    const writes = new Map<number, number>();
    const applied = applyBlendshapes(mapping, categories([
      ['jawOpen', 0.8],
      ['eyeBlinkLeft', 0.5],
      ['tongueOut', 0.9], // dead -> 0
      ['cheekPuff', 0.7], // dead -> 0
    ]), (index, value) => {
      writes.set(index, value);
      return true;
    });
    expect(applied).toBe(4);
    const jawOpen = sampleMorphNames().indexOf('jawOpen');
    const blink = sampleMorphNames().indexOf('eyeBlinkLeft');
    const tongue = sampleMorphNames().indexOf('tongueOut');
    const puff = sampleMorphNames().indexOf('cheekPuff');
    expect(writes.get(jawOpen)).toBeCloseTo(0.8);
    expect(writes.get(blink)).toBeCloseTo(0.5);
    expect(writes.get(tongue)).toBe(0);
    expect(writes.get(puff)).toBe(0);
  });

  test('clamps scores to [0, 1] and skips unknown or absent names', () => {
    const mapping = buildBlendshapeMapping(sampleMorphNames());
    let applied = 0;
    const count = applyBlendshapes(mapping, categories([
      ['jawOpen', 1.7],
      ['browDownLeft', -0.3],
      ['notAnARKitShape', 0.9],
      ['mouthDimpleRight', 0.9], // absent from the rig
    ]), () => {
      applied++;
      return true;
    });
    // Unknown names and unmapped morphs are skipped before apply is called.
    expect(applied).toBe(2);
    expect(count).toBe(2);
  });
});

describe('BlendshapeSmoother', () => {
  test('snaps values below the threshold to zero', () => {
    const smoother = new BlendshapeSmoother(0.5, 0.02);
    expect(smoother.smooth(0, 0.01)).toBe(0);
  });

  test('approaches the target asymptotically and resets cleanly', () => {
    const smoother = new BlendshapeSmoother(0.5, 0.02);
    const first = smoother.smooth(0, 1);
    expect(first).toBeCloseTo(1); // first sample takes the target directly
    const second = smoother.smooth(0, 1);
    expect(second).toBeCloseTo(1);
    const fall = smoother.smooth(0, 0);
    expect(fall).toBe(0); // below snap -> snapped
    smoother.reset();
    expect(smoother.smooth(0, 0.5)).toBeCloseTo(0.5); // fresh start, no memory
  });

  test('keeps per-category state independent', () => {
    const smoother = new BlendshapeSmoother(0.5, 0.02);
    smoother.smooth(1, 1);
    const other = smoother.smooth(2, 0.9);
    expect(other).toBeCloseTo(0.9);
  });
});

describe('character rig gate', () => {
  test('the generated character has every ARKit shape by exact name', () => {
    for (const sex of ['male', 'female'] as const) {
      const names = loadCharacterMorphNames(sex);
      for (const name of ARKIT_BLENDSHAPES) {
        expect(names).toContain(name);
      }
    }
  });

  test('all dead shapes are part of the ARKit 52', () => {
    for (const name of DEAD_BLENDSHAPES) {
      expect(ARKIT_BLENDSHAPES).toContain(name);
    }
  });
});
