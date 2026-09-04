export type PaintShortcut =
  | 'actual-pixels'
  | 'brush-larger'
  | 'brush-smaller'
  | 'brush-tool'
  | 'clear-layer'
  | 'default-colors'
  | 'export-png'
  | 'fit-view'
  | 'hand-tool'
  | 'eyedropper-tool'
  | 'hardness-harder'
  | 'hardness-softer'
  | 'new-document'
  | 'new-layer'
  | 'next-brush'
  | 'open-document'
  | 'previous-brush'
  | 'redo'
  | 'rotate-tool'
  | 'save-document'
  | 'switch-colors'
  | 'toggle-panels'
  | 'undo'
  | 'zoom-in'
  | 'zoom-out'
  | 'zoom-tool'
  | `opacity-${number}`;

export interface PaintShortcutInput {
  key: string;
  code?: string;
  ctrlKey?: boolean;
  metaKey?: boolean;
  shiftKey?: boolean;
  altKey?: boolean;
  editable?: boolean;
}

export function resolvePaintShortcut(input: PaintShortcutInput): PaintShortcut | undefined {
  const key = input.key.toLowerCase();
  const command = input.ctrlKey === true || input.metaKey === true;

  if (command) {
    if (input.altKey && input.shiftKey && key === 'w') return 'export-png';
    if (input.altKey) return undefined;
    if (key === 'z') return input.shiftKey ? 'redo' : 'undo';
    if (key === 'n') return input.shiftKey ? 'new-layer' : 'new-document';
    if (key === '+' || key === '=') return 'zoom-in';
    if (input.shiftKey) return undefined;
    if (key === 'o') return 'open-document';
    if (key === 's') return 'save-document';
    if (key === '-') return 'zoom-out';
    if (key === '0') return 'fit-view';
    if (key === '1') return 'actual-pixels';
    return undefined;
  }

  if (input.editable || input.altKey) return undefined;
  if (key === '{') return 'hardness-softer';
  if (key === '}') return 'hardness-harder';
  if (input.shiftKey) return undefined;

  if (/^[0-9]$/.test(key)) return `opacity-${key === '0' ? 100 : Number(key) * 10}`;

  switch (input.code ?? input.key) {
    case 'KeyB': return 'brush-tool';
    case 'KeyH': return 'hand-tool';
    case 'KeyI': return 'eyedropper-tool';
    case 'KeyR': return 'rotate-tool';
    case 'KeyZ': return 'zoom-tool';
    case 'BracketLeft': return 'brush-smaller';
    case 'BracketRight': return 'brush-larger';
    case 'KeyD': return 'default-colors';
    case 'KeyX': return 'switch-colors';
    case 'Comma': return 'previous-brush';
    case 'Period': return 'next-brush';
    case 'Delete': return 'clear-layer';
    case 'Tab': return 'toggle-panels';
    default: return undefined;
  }
}
