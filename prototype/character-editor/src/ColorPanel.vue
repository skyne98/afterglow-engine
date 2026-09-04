<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from 'vue';
import { RotateCcw } from '@lucide/vue';
import { Button } from '@/components/ui/button';
import { hsvToRgb, parseHexColor, rgbToHex, rgbToHsv, type RgbColor } from './paint-color.ts';

const wheel = ref<HTMLDivElement>();
const colorInput = ref<HTMLInputElement>();
const hue = ref(174);
const saturation = ref(62);
const value = ref(80);
const previous = ref('#4ecdc4');
const hexDraft = ref('#4ecdc4');
let drag: 'hue' | 'sv' | null = null;

const rgb = computed(() => hsvToRgb({ h: hue.value, s: saturation.value, v: value.value }));
const hex = computed(() => rgbToHex(rgb.value));
const hueHex = computed(() => rgbToHex(hsvToRgb({ h: hue.value, s: 100, v: 100 })));
const hueHandle = computed(() => {
  const angle = (hue.value - 90) * Math.PI / 180;
  return { left: `${50 + Math.cos(angle) * 44}%`, top: `${50 + Math.sin(angle) * 44}%` };
});
const svHandle = computed(() => ({ left: `${saturation.value}%`, top: `${100 - value.value}%` }));

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(maximum, Math.max(minimum, Number.isFinite(value) ? value : minimum));
}
function publish(): void {
  hexDraft.value = hex.value;
  void nextTick(() => colorInput.value?.dispatchEvent(new Event('input', { bubbles: true })));
}
function setHsv(h: number, s: number, v: number, send = true): void {
  hue.value = ((h % 360) + 360) % 360;
  saturation.value = clamp(s, 0, 100);
  value.value = clamp(v, 0, 100);
  if (send) publish();
}
function setRgb(channel: keyof RgbColor, next: number): void {
  const color = { ...rgb.value, [channel]: clamp(next, 0, 255) };
  const hsv = rgbToHsv(color);
  setHsv(hsv.h, hsv.s, hsv.v);
}
function setHex(value: string, send = true): boolean {
  const color = parseHexColor(value);
  if (!color) return false;
  const hsv = rgbToHsv(color);
  setHsv(hsv.h, hsv.s, hsv.v, send);
  hexDraft.value = rgbToHex(color);
  return true;
}
function applyHex(): void {
  if (!setHex(hexDraft.value)) hexDraft.value = hex.value;
}
function remember(): void {
  previous.value = hex.value;
}
function restorePrevious(): void {
  const current = hex.value;
  if (setHex(previous.value)) previous.value = current;
}
function updateFromPointer(event: PointerEvent): void {
  const element = wheel.value;
  if (!element || !drag) return;
  const bounds = element.getBoundingClientRect();
  const x = event.clientX - bounds.left;
  const y = event.clientY - bounds.top;
  if (drag === 'hue') {
    const angle = Math.atan2(y - bounds.height / 2, x - bounds.width / 2) * 180 / Math.PI + 90;
    setHsv(angle, saturation.value, value.value);
  } else {
    const side = bounds.width * 0.52;
    const left = (bounds.width - side) / 2;
    const top = (bounds.height - side) / 2;
    setHsv(hue.value, ((x - left) / side) * 100, (1 - (y - top) / side) * 100);
  }
}
function startPointer(event: PointerEvent): void {
  const element = wheel.value;
  if (!element) return;
  remember();
  const bounds = element.getBoundingClientRect();
  const x = event.clientX - bounds.left - bounds.width / 2;
  const y = event.clientY - bounds.top - bounds.height / 2;
  const halfSquare = bounds.width * 0.26;
  drag = Math.abs(x) <= halfSquare && Math.abs(y) <= halfSquare ? 'sv' : 'hue';
  element.setPointerCapture(event.pointerId);
  updateFromPointer(event);
}
function movePointer(event: PointerEvent): void {
  if (drag) updateFromPointer(event);
}
function endPointer(event: PointerEvent): void {
  drag = null;
  if (wheel.value?.hasPointerCapture(event.pointerId)) wheel.value.releasePointerCapture(event.pointerId);
}
function syncExternal(event: Event): void {
  const value = (event.target as HTMLInputElement).value;
  if (value !== hex.value) {
    previous.value = hex.value;
    setHex(value, false);
  }
}

onMounted(() => setHex('#4ecdc4', false));
</script>

<template>
  <div class="color-picker">
    <input id="color" ref="colorInput" type="hidden" :value="hex" @input="syncExternal" @change="syncExternal" />
    <div
      ref="wheel"
      class="color-wheel"
      aria-label="Hue and saturation-value picker"
      @pointerdown.prevent="startPointer"
      @pointermove.prevent="movePointer"
      @pointerup="endPointer"
      @pointercancel="endPointer"
    >
      <div class="wheel-gap" />
      <span class="hue-handle" :style="hueHandle" />
      <div
        class="sv-square"
        :style="{ backgroundColor: hueHex }"
      >
        <span class="sv-handle" :style="svHandle" />
      </div>
    </div>

    <div class="color-preview-row">
      <button class="color-preview current" type="button" :style="{ background: hex }" aria-label="Current color" />
      <button class="color-preview previous" type="button" :style="{ background: previous }" aria-label="Previous color" @click="restorePrevious" />
      <label class="hex-field">#<input id="colorHex" v-model="hexDraft" maxlength="7" spellcheck="false" @focus="remember" @change="applyHex" /></label>
      <Button variant="ghost" size="icon-sm" aria-label="Restore previous color" @click="restorePrevious"><RotateCcw /></Button>
    </div>

    <div class="color-fields">
      <label>H <input type="number" min="0" max="359" :value="Math.round(hue)" @focus="remember" @input="setHsv(Number(($event.target as HTMLInputElement).value), saturation, value)" /></label>
      <label>S <input type="number" min="0" max="100" :value="Math.round(saturation)" @focus="remember" @input="setHsv(hue, Number(($event.target as HTMLInputElement).value), value)" /></label>
      <label>V <input type="number" min="0" max="100" :value="Math.round(value)" @focus="remember" @input="setHsv(hue, saturation, Number(($event.target as HTMLInputElement).value))" /></label>
    </div>
    <div class="color-fields">
      <label>R <input type="number" min="0" max="255" :value="rgb.r" @focus="remember" @input="setRgb('r', Number(($event.target as HTMLInputElement).value))" /></label>
      <label>G <input type="number" min="0" max="255" :value="rgb.g" @focus="remember" @input="setRgb('g', Number(($event.target as HTMLInputElement).value))" /></label>
      <label>B <input type="number" min="0" max="255" :value="rgb.b" @focus="remember" @input="setRgb('b', Number(($event.target as HTMLInputElement).value))" /></label>
    </div>
  </div>
</template>
