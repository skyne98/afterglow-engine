// Software Canvas2D implementation for image-backed game assets. The backing
// Uint8ClampedArray is exposed on the canvas as `data`, allowing WebGPU's
// copyExternalImageToTexture bridge to upload CanvasTexture sources.

const clampByte = (value) => Math.max(0, Math.min(255, Math.round(value)));

const namedColors = new Map(Object.entries({
  transparent: [0, 0, 0, 0], black: [0, 0, 0, 255], white: [255, 255, 255, 255],
  red: [255, 0, 0, 255], green: [0, 128, 0, 255], blue: [0, 0, 255, 255],
  yellow: [255, 255, 0, 255], magenta: [255, 0, 255, 255], cyan: [0, 255, 255, 255],
  gray: [128, 128, 128, 255], grey: [128, 128, 128, 255],
}));

function parseColor(value) {
  if (Array.isArray(value)) return value;
  const color = String(value).trim().toLowerCase();
  if (namedColors.has(color)) return namedColors.get(color).slice();
  let match = color.match(/^#([0-9a-f]{3,8})$/i);
  if (match) {
    let hex = match[1];
    if (hex.length === 3 || hex.length === 4) hex = [...hex].map((c) => c + c).join('');
    if (hex.length === 6) hex += 'ff';
    return [0, 2, 4, 6].map((offset) => parseInt(hex.slice(offset, offset + 2), 16));
  }
  match = color.match(/^rgba?\((.+)\)$/);
  if (match) {
    const parts = match[1].split(/[ ,/]+/).filter(Boolean);
    const component = (part) => part.endsWith('%') ? parseFloat(part) * 2.55 : parseFloat(part);
    const alpha = parts[3] === undefined ? 255 : (parts[3].endsWith('%') ? parseFloat(parts[3]) * 2.55 : parseFloat(parts[3]) * 255);
    return [clampByte(component(parts[0])), clampByte(component(parts[1])), clampByte(component(parts[2])), clampByte(alpha)];
  }
  match = color.match(/^hsla?\((.+)\)$/);
  if (match) {
    const parts = match[1].split(/[ ,/]+/).filter(Boolean);
    const h = ((parseFloat(parts[0]) % 360) + 360) % 360 / 360;
    const s = parseFloat(parts[1]) / 100;
    const l = parseFloat(parts[2]) / 100;
    const hue = (p, q, t) => {
      if (t < 0) t++;
      if (t > 1) t--;
      if (t < 1 / 6) return p + (q - p) * 6 * t;
      if (t < 1 / 2) return q;
      if (t < 2 / 3) return p + (q - p) * (2 / 3 - t) * 6;
      return p;
    };
    const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
    const p = 2 * l - q;
    const rgb = s === 0 ? [l, l, l] : [hue(p, q, h + 1 / 3), hue(p, q, h), hue(p, q, h - 1 / 3)];
    const alpha = parts[3] === undefined ? 1 : parseFloat(parts[3]);
    return [...rgb.map((v) => clampByte(v * 255)), clampByte(alpha * 255)];
  }
  return [0, 0, 0, 255];
}

class CanvasGradient {
  constructor(kind, coordinates) {
    this.kind = kind;
    this.coordinates = coordinates;
    this.stops = [];
  }
  addColorStop(offset, color) {
    offset = Number(offset);
    if (!Number.isFinite(offset) || offset < 0 || offset > 1) throw new RangeError('Color stop offset must be between 0 and 1');
    this.stops.push([offset, parseColor(color)]);
    this.stops.sort((a, b) => a[0] - b[0]);
  }
  colorAt(x, y) {
    let t = 0;
    if (this.kind === 'linear') {
      const [x0, y0, x1, y1] = this.coordinates;
      const dx = x1 - x0, dy = y1 - y0;
      t = (dx * (x - x0) + dy * (y - y0)) / (dx * dx + dy * dy || 1);
    } else {
      const [x0, y0, r0, x1, y1, r1] = this.coordinates;
      const distance = Math.hypot(x - x1, y - y1);
      t = (distance - r0) / (r1 - r0 || 1);
      if (x0 !== x1 || y0 !== y1) t = Math.min(t, Math.hypot(x - x0, y - y0) / (r1 || 1));
    }
    t = Math.max(0, Math.min(1, t));
    if (this.stops.length === 0) return [0, 0, 0, 0];
    let left = this.stops[0], right = this.stops[this.stops.length - 1];
    for (let i = 1; i < this.stops.length; i++) {
      if (t <= this.stops[i][0]) { left = this.stops[i - 1]; right = this.stops[i]; break; }
    }
    const f = right[0] === left[0] ? 0 : (t - left[0]) / (right[0] - left[0]);
    return left[1].map((value, i) => clampByte(value + (right[1][i] - value) * f));
  }
}

class ImageData {
  constructor(dataOrWidth, widthOrHeight, maybeHeight) {
    if (typeof dataOrWidth === 'number') {
      this.width = dataOrWidth;
      this.height = widthOrHeight;
      this.data = new Uint8ClampedArray(this.width * this.height * 4);
    } else {
      this.data = dataOrWidth instanceof Uint8ClampedArray ? dataOrWidth : new Uint8ClampedArray(dataOrWidth);
      this.width = widthOrHeight;
      this.height = maybeHeight ?? this.data.length / 4 / this.width;
    }
  }
}

const identity = () => [1, 0, 0, 1, 0, 0];
const multiply = (a, b) => [
  a[0] * b[0] + a[2] * b[1], a[1] * b[0] + a[3] * b[1],
  a[0] * b[2] + a[2] * b[3], a[1] * b[2] + a[3] * b[3],
  a[0] * b[4] + a[2] * b[5] + a[4], a[1] * b[4] + a[3] * b[5] + a[5],
];

class CanvasRenderingContext2D {
  constructor(canvas) {
    this.canvas = canvas;
    this.fillStyle = '#000000';
    this.strokeStyle = '#000000';
    this.globalAlpha = 1;
    this.lineWidth = 1;
    this.font = '10px sans-serif';
    this.textAlign = 'start';
    this.textBaseline = 'alphabetic';
    this.imageSmoothingEnabled = true;
    this._width = 0;
    this._height = 0;
    this._data = new Uint8ClampedArray();
    this._path = [];
    this._matrix = identity();
    this._clip = null;
    this._stack = [];
    this._syncSize();
  }

  _syncSize() {
    const width = Math.max(0, Number(this.canvas.width) | 0);
    const height = Math.max(0, Number(this.canvas.height) | 0);
    if (width !== this._width || height !== this._height) {
      this._width = width;
      this._height = height;
      this._data = new Uint8ClampedArray(width * height * 4);
    }
  }
  get data() { this._syncSize(); return this._data; }
  _point(x, y) {
    const m = this._matrix;
    return [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]];
  }
  _styleColor(style, x, y) {
    const color = style instanceof CanvasGradient ? style.colorAt(x, y) : parseColor(style);
    color[3] = clampByte(color[3] * this.globalAlpha);
    return color;
  }
  _pixel(x, y, color) {
    x = x | 0; y = y | 0;
    if (x < 0 || y < 0 || x >= this._width || y >= this._height) return;
    if (this._clip && (x < this._clip.x0 || y < this._clip.y0 || x >= this._clip.x1 || y >= this._clip.y1)) return;
    const index = (y * this._width + x) * 4;
    const sourceAlpha = color[3] / 255;
    const destinationAlpha = this._data[index + 3] / 255;
    const outputAlpha = sourceAlpha + destinationAlpha * (1 - sourceAlpha);
    if (outputAlpha === 0) {
      this._data.fill(0, index, index + 4);
      return;
    }
    for (let channel = 0; channel < 3; channel++) {
      this._data[index + channel] = clampByte((color[channel] * sourceAlpha + this._data[index + channel] * destinationAlpha * (1 - sourceAlpha)) / outputAlpha);
    }
    this._data[index + 3] = clampByte(outputAlpha * 255);
  }
  _line(x0, y0, x1, y1, color, width = this.lineWidth) {
    const steps = Math.max(1, Math.ceil(Math.max(Math.abs(x1 - x0), Math.abs(y1 - y0))));
    const radius = Math.max(0, Math.floor(width / 2));
    for (let step = 0; step <= steps; step++) {
      const x = Math.round(x0 + (x1 - x0) * step / steps);
      const y = Math.round(y0 + (y1 - y0) * step / steps);
      for (let dy = -radius; dy <= radius; dy++) for (let dx = -radius; dx <= radius; dx++) this._pixel(x + dx, y + dy, color);
    }
  }

  clearRect(x, y, width, height) {
    this._syncSize();
    const [x0, y0] = this._point(x, y), [x1, y1] = this._point(x + width, y + height);
    for (let py = Math.floor(Math.min(y0, y1)); py < Math.ceil(Math.max(y0, y1)); py++) {
      for (let px = Math.floor(Math.min(x0, x1)); px < Math.ceil(Math.max(x0, x1)); px++) {
        if (px >= 0 && py >= 0 && px < this._width && py < this._height) this._data.fill(0, (py * this._width + px) * 4, (py * this._width + px) * 4 + 4);
      }
    }
  }
  fillRect(x, y, width, height) {
    this._syncSize();
    const [x0, y0] = this._point(x, y), [x1, y1] = this._point(x + width, y + height);
    for (let py = Math.floor(Math.min(y0, y1)); py < Math.ceil(Math.max(y0, y1)); py++) {
      for (let px = Math.floor(Math.min(x0, x1)); px < Math.ceil(Math.max(x0, x1)); px++) this._pixel(px, py, this._styleColor(this.fillStyle, px, py));
    }
  }
  strokeRect(x, y, width, height) {
    this.beginPath(); this.rect(x, y, width, height); this.stroke();
  }
  beginPath() { this._path = []; }
  closePath() { this._path.push({ type: 'close' }); }
  moveTo(x, y) { this._path.push({ type: 'move', point: this._point(x, y) }); }
  lineTo(x, y) { this._path.push({ type: 'line', point: this._point(x, y) }); }
  rect(x, y, width, height) {
    this.moveTo(x, y); this.lineTo(x + width, y); this.lineTo(x + width, y + height); this.lineTo(x, y + height); this.closePath();
  }
  arc(x, y, radius, start, end, anticlockwise = false) {
    const center = this._point(x, y);
    const edge = this._point(x + radius, y);
    const transformedRadius = Math.hypot(edge[0] - center[0], edge[1] - center[1]);
    this._path.push({ type: 'arc', x: center[0], y: center[1], radius: transformedRadius, start, end, anticlockwise });
  }
  arcTo(x1, y1, x2, y2) { this.lineTo(x1, y1); this.lineTo(x2, y2); }
  fill() {
    this._syncSize();
    for (const command of this._path) {
      if (command.type !== 'arc') continue;
      const minX = Math.floor(command.x - command.radius), maxX = Math.ceil(command.x + command.radius);
      const minY = Math.floor(command.y - command.radius), maxY = Math.ceil(command.y + command.radius);
      for (let y = minY; y <= maxY; y++) for (let x = minX; x <= maxX; x++) {
        if ((x - command.x) ** 2 + (y - command.y) ** 2 <= command.radius ** 2) this._pixel(x, y, this._styleColor(this.fillStyle, x, y));
      }
    }
    const points = this._path.filter((entry) => entry.point).map((entry) => entry.point);
    if (points.length >= 3) {
      const minY = Math.floor(Math.min(...points.map((p) => p[1]))), maxY = Math.ceil(Math.max(...points.map((p) => p[1])));
      for (let y = minY; y <= maxY; y++) {
        const intersections = [];
        for (let i = 0, j = points.length - 1; i < points.length; j = i++) {
          const a = points[i], b = points[j];
          if ((a[1] > y) !== (b[1] > y)) intersections.push((b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) + a[0]);
        }
        intersections.sort((a, b) => a - b);
        for (let i = 0; i + 1 < intersections.length; i += 2) for (let x = Math.ceil(intersections[i]); x < intersections[i + 1]; x++) this._pixel(x, y, this._styleColor(this.fillStyle, x, y));
      }
    }
  }
  stroke() {
    this._syncSize();
    const color = this._styleColor(this.strokeStyle, 0, 0);
    let start = null, current = null;
    for (const command of this._path) {
      if (command.type === 'move') { start = current = command.point; }
      else if (command.type === 'line') { if (current) this._line(...current, ...command.point, color); current = command.point; }
      else if (command.type === 'close' && current && start) this._line(...current, ...start, color);
      else if (command.type === 'arc') {
        const span = command.end - command.start;
        const steps = Math.max(12, Math.ceil(Math.abs(span) * command.radius));
        let previous = [command.x + Math.cos(command.start) * command.radius, command.y + Math.sin(command.start) * command.radius];
        for (let i = 1; i <= steps; i++) {
          const angle = command.start + span * i / steps;
          const next = [command.x + Math.cos(angle) * command.radius, command.y + Math.sin(angle) * command.radius];
          this._line(...previous, ...next, color); previous = next;
        }
      }
    }
  }
  clip() {
    const bounds = [];
    for (const command of this._path) {
      if (command.point) bounds.push(command.point);
      if (command.type === 'arc') {
        bounds.push([command.x - command.radius, command.y - command.radius]);
        bounds.push([command.x + command.radius, command.y + command.radius]);
      }
    }
    if (bounds.length === 0) return;
    const clip = {
      x0: Math.floor(Math.min(...bounds.map((point) => point[0]))),
      y0: Math.floor(Math.min(...bounds.map((point) => point[1]))),
      x1: Math.ceil(Math.max(...bounds.map((point) => point[0]))),
      y1: Math.ceil(Math.max(...bounds.map((point) => point[1]))),
    };
    if (this._clip) {
      clip.x0 = Math.max(clip.x0, this._clip.x0);
      clip.y0 = Math.max(clip.y0, this._clip.y0);
      clip.x1 = Math.min(clip.x1, this._clip.x1);
      clip.y1 = Math.min(clip.y1, this._clip.y1);
    }
    this._clip = clip;
  }

  createLinearGradient(...coordinates) { return new CanvasGradient('linear', coordinates.map(Number)); }
  createRadialGradient(...coordinates) { return new CanvasGradient('radial', coordinates.map(Number)); }
  createImageData(width, height) { return new ImageData(width, height); }
  getImageData(x, y, width, height) {
    this._syncSize();
    const result = new ImageData(width, height);
    for (let row = 0; row < height; row++) for (let column = 0; column < width; column++) {
      const sx = x + column, sy = y + row;
      if (sx < 0 || sy < 0 || sx >= this._width || sy >= this._height) continue;
      const source = (sy * this._width + sx) * 4, destination = (row * width + column) * 4;
      result.data.set(this._data.subarray(source, source + 4), destination);
    }
    return result;
  }
  putImageData(image, x, y) {
    this._syncSize();
    for (let row = 0; row < image.height; row++) for (let column = 0; column < image.width; column++) {
      const dx = x + column, dy = y + row;
      if (dx < 0 || dy < 0 || dx >= this._width || dy >= this._height) continue;
      const source = (row * image.width + column) * 4, destination = (dy * this._width + dx) * 4;
      this._data.set(image.data.subarray(source, source + 4), destination);
    }
  }
  drawImage(image, ...args) {
    this._syncSize();
    const sourceData = image?.data ?? image?._canvas2d?.data;
    const sourceWidth = Number(image?.width ?? image?._canvas2d?._width);
    const sourceHeight = Number(image?.height ?? image?._canvas2d?._height);
    if (!sourceData || !sourceWidth || !sourceHeight) throw new TypeError('drawImage source has no pixel data');
    let sx = 0, sy = 0, sw = sourceWidth, sh = sourceHeight, dx, dy, dw, dh;
    if (args.length === 2) [dx, dy, dw, dh] = [args[0], args[1], sw, sh];
    else if (args.length === 4) [dx, dy, dw, dh] = args;
    else if (args.length === 8) [sx, sy, sw, sh, dx, dy, dw, dh] = args;
    else throw new TypeError('Invalid drawImage arguments');
    const origin = this._point(dx, dy), corner = this._point(dx + dw, dy + dh);
    const outWidth = Math.max(1, Math.round(Math.abs(corner[0] - origin[0]))), outHeight = Math.max(1, Math.round(Math.abs(corner[1] - origin[1])));
    for (let y = 0; y < outHeight; y++) for (let x = 0; x < outWidth; x++) {
      const sourceX = Math.max(0, Math.min(sourceWidth - 1, Math.floor(sx + x * sw / outWidth)));
      const sourceY = Math.max(0, Math.min(sourceHeight - 1, Math.floor(sy + y * sh / outHeight)));
      const index = (sourceY * sourceWidth + sourceX) * 4;
      const color = [sourceData[index], sourceData[index + 1], sourceData[index + 2], clampByte(sourceData[index + 3] * this.globalAlpha)];
      this._pixel(Math.round(origin[0]) + x, Math.round(origin[1]) + y, color);
    }
  }

  save() {
    this._stack.push({ fillStyle: this.fillStyle, strokeStyle: this.strokeStyle, globalAlpha: this.globalAlpha, lineWidth: this.lineWidth, font: this.font, textAlign: this.textAlign, textBaseline: this.textBaseline, _matrix: this._matrix.slice(), _clip: this._clip ? { ...this._clip } : null });
  }
  restore() { Object.assign(this, this._stack.pop() ?? {}); }
  translate(x, y) { this._matrix = multiply(this._matrix, [1, 0, 0, 1, x, y]); }
  scale(x, y) { this._matrix = multiply(this._matrix, [x, 0, 0, y, 0, 0]); }
  rotate(angle) { const c = Math.cos(angle), s = Math.sin(angle); this._matrix = multiply(this._matrix, [c, s, -s, c, 0, 0]); }
  transform(...matrix) { this._matrix = multiply(this._matrix, matrix.map(Number)); }
  setTransform(...args) {
    if (args.length === 1 && typeof args[0] === 'object') {
      const m = args[0]; this._matrix = [m.a, m.b, m.c, m.d, m.e, m.f].map(Number);
    } else this._matrix = args.length === 0 ? identity() : args.map(Number);
  }
  resetTransform() { this._matrix = identity(); }

  measureText(text) {
    const size = parseFloat(this.font) || 10;
    const width = String(text).length * size * 0.6;
    return { width, actualBoundingBoxLeft: 0, actualBoundingBoxRight: width, actualBoundingBoxAscent: size * 0.8, actualBoundingBoxDescent: size * 0.2 };
  }
  fillText(text, x, y) {
    const size = Math.max(1, Math.round(parseFloat(this.font) || 10));
    const glyphWidth = Math.max(1, Math.round(size * 0.5));
    const glyphHeight = Math.max(1, Math.round(size * 0.75));
    let cursor = x;
    for (const character of String(text)) {
      if (character !== ' ') {
        const code = character.charCodeAt(0);
        for (let row = 0; row < 7; row++) for (let column = 0; column < 5; column++) {
          if (((code * 1103515245 + row * 31 + column * 17) >>> ((row + column) % 16)) & 1) {
            this.fillRect(cursor + column * glyphWidth / 5, y - glyphHeight + row * glyphHeight / 7, Math.ceil(glyphWidth / 5), Math.ceil(glyphHeight / 7));
          }
        }
      }
      cursor += size * 0.6;
    }
  }
  strokeText(text, x, y) { const previous = this.fillStyle; this.fillStyle = this.strokeStyle; this.fillText(text, x, y); this.fillStyle = previous; }
}

export function installCanvas2D(canvas) {
  if (canvas._canvas2d) return canvas._canvas2d;
  const context = new CanvasRenderingContext2D(canvas);
  Object.defineProperty(canvas, '_canvas2d', { value: context, configurable: false });
  Object.defineProperty(canvas, 'data', { configurable: true, get: () => context.data });
  return context;
}

export { CanvasGradient, CanvasRenderingContext2D, ImageData };
