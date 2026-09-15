// Text-control edits are a bounded UI slow path, not a sealed game path.
export class TextHistory {
  constructor(maxEdits = 512, maxBytes = 32 * 1024 * 1024) {
    if (!Number.isSafeInteger(maxEdits) || maxEdits < 1 || !Number.isSafeInteger(maxBytes) || maxBytes < 1) {
      throw new RangeError('Text history limits must be positive safe integers');
    }
    this.maxEdits = maxEdits;
    this.maxBytes = maxBytes;
    this.bytes = 0;
    this.states = new WeakMap();
    this.first = null;
    this.last = null;
  }
  remove(entry) {
    if (entry.previous) entry.previous.next = entry.next;
    else this.first = entry.next;
    if (entry.next) entry.next.previous = entry.previous;
    else this.last = entry.previous;
    const state = entry.state;
    // ponytail: removal scans at most maxEdits entries in this UI slow path.
    const index = state.entries.indexOf(entry);
    state.entries.splice(index, 1);
    if (index < state.position) state.position--;
    this.bytes -= entry.bytes;
  }
  clear(control) {
    const state = this.states.get(control);
    if (state) while (state.entries.length) this.remove(state.entries[0]);
    this.states.delete(control);
  }
  record(control, before, after) {
    if (before.value === after.value) return;
    let state = this.states.get(control);
    if (!state) {
      state = { entries: [], position: 0 };
      this.states.set(control, state);
    }
    while (state.entries.length > state.position) this.remove(state.entries[state.entries.length - 1]);
    const bytes = 256 + 2 * (before.value.length + after.value.length);
    if (bytes > this.maxBytes) { this.clear(control); return; }
    while (state.entries.length >= this.maxEdits) this.remove(state.entries[0]);
    while (this.bytes + bytes > this.maxBytes) this.remove(this.first);
    const entry = { before, after, bytes, state, previous: this.last, next: null };
    if (this.last) this.last.next = entry;
    else this.first = entry;
    this.last = entry;
    state.entries.push(entry);
    state.position++;
    this.bytes += bytes;
  }
  peek(control, redo) {
    const state = this.states.get(control);
    if (!state) return null;
    const entry = state.entries[redo ? state.position : state.position - 1];
    return entry ? { entry, value: redo ? entry.after : entry.before } : null;
  }
  accept(control, token, redo) {
    const current = this.peek(control, redo);
    if (!current || current.entry !== token.entry) return false;
    current.entry.state.position += redo ? 1 : -1;
    return true;
  }
}
