export const enum DemoInputAction {
  OrbitLeft = 0,
  OrbitRight = 1,
  ZoomIn = 2,
  ZoomOut = 3,
  ModelOne = 4,
  ModelTwo = 5,
  ToggleAnimation = 6,
  ToggleSkeleton = 7,
  ToggleFeedback = 8,
  ResetView = 9,
  Overview = 10,
  PixelView = 11,
  Sprint = 12,
  PoseThree = 13,
  Count = 14,
}

function actionFor(event: KeyboardEvent): DemoInputAction | -1 {
  switch (event.code || event.key) {
    case 'KeyA': case 'a': case 'A': return DemoInputAction.OrbitLeft;
    case 'KeyD': case 'd': case 'D': return DemoInputAction.OrbitRight;
    case 'KeyW': case 'w': case 'W': return DemoInputAction.ZoomIn;
    case 'KeyS': case 's': case 'S': return DemoInputAction.ZoomOut;
    case 'Digit1': case '1': return DemoInputAction.ModelOne;
    case 'Digit2': case '2': return DemoInputAction.ModelTwo;
    case 'Space': case ' ': return DemoInputAction.ToggleAnimation;
    case 'KeyB': case 'b': case 'B': return DemoInputAction.ToggleSkeleton;
    case 'KeyF': case 'f': case 'F': return DemoInputAction.ToggleFeedback;
    case 'KeyR': case 'r': case 'R': return DemoInputAction.ResetView;
    case 'KeyO': case 'o': case 'O': return DemoInputAction.Overview;
    case 'KeyP': case 'p': case 'P': return DemoInputAction.PixelView;
    case 'ShiftLeft': case 'ShiftRight': case 'Shift': return DemoInputAction.Sprint;
    case 'Digit3': case '3': return DemoInputAction.PoseThree;
    default: return -1;
  }
}

/** Fixed action table and owned browser-listener lifecycle for visual demos. */
export class BoundedKeyboardInput {
  private readonly down = new Uint8Array(DemoInputAction.Count);
  private readonly pressed = new Uint8Array(DemoInputAction.Count);
  private readonly onKeyDown: (event: KeyboardEvent) => void;
  private readonly onKeyUp: (event: KeyboardEvent) => void;
  private readonly onBlur: () => void;
  private readonly onWheel: (event: WheelEvent) => void;
  private wheelDelta = 0;
  private disposed = false;
  programmatic = false;

  constructor(private readonly target: Window = window) {
    this.onKeyDown = (event): void => {
      if (this.programmatic) return;
      const action = actionFor(event);
      if (action < 0) return;
      if (this.down[action] === 0 && !event.repeat) this.pressed[action] = 1;
      this.down[action] = 1;
    };
    this.onKeyUp = (event): void => {
      const action = actionFor(event);
      if (action >= 0) this.down[action] = 0;
    };
    this.onBlur = (): void => { this.down.fill(0); this.pressed.fill(0); };
    this.onWheel = (event): void => { if (!this.programmatic) this.wheelDelta += event.deltaY; };
    target.addEventListener('keydown', this.onKeyDown);
    target.addEventListener('keyup', this.onKeyUp);
    target.addEventListener('blur', this.onBlur);
    target.addEventListener('wheel', this.onWheel);
  }

  /** @alloc-effect none */
  isDown(action: DemoInputAction): boolean { return this.down[action] !== 0; }

  /** @alloc-effect none */
  consumePressed(action: DemoInputAction): boolean {
    const value = this.pressed[action] !== 0;
    this.pressed[action] = 0;
    return value;
  }

  /** @alloc-effect none */
  consumeWheelDelta(): number {
    const delta = this.wheelDelta;
    this.wheelDelta = 0;
    return delta;
  }

  /** @alloc-effect none */
  clear(): void { this.down.fill(0); this.pressed.fill(0); this.wheelDelta = 0; }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.target.removeEventListener('keydown', this.onKeyDown);
    this.target.removeEventListener('keyup', this.onKeyUp);
    this.target.removeEventListener('blur', this.onBlur);
    this.target.removeEventListener('wheel', this.onWheel);
    this.clear();
  }
}
