export interface ZoneControlLike {
  category: string;
  label: string;
}

export function selectTriangleCategory(
  categories: readonly [number, number, number],
  scores: readonly [number, number, number],
): number {
  const [a, b, c] = categories;
  if (a >= 0 && (a === b || a === c)) return a;
  if (b >= 0 && b === c) return b;
  let selected = -1;
  let maximum = -1;
  for (let index = 0; index < 3; index++) {
    if (categories[index] >= 0 && scores[index] > maximum) {
      selected = categories[index];
      maximum = scores[index];
    }
  }
  return selected;
}

export function controlBelongsToZone(spec: ZoneControlLike, category: string): boolean {
  if (spec.category === category) return true;
  if (category === 'breast' && spec.category === 'macro') {
    return spec.label === 'Cup size' || spec.label === 'Breast firmness';
  }
  if (spec.category !== 'asymmetry') return false;
  const tokens: Record<string, string[]> = {
    breast: ['breast'],
    cheek: ['cheek'],
    ears: ['ear'],
    eyes: ['eye'],
    eyebrows: ['brown'],
    mouth: ['mouth'],
    nose: ['nose'],
    chin: ['jaw'],
    torso: ['trunk'],
    head: ['temple', 'top'],
  };
  return (tokens[category] ?? []).some((token) => spec.label.includes(token));
}
