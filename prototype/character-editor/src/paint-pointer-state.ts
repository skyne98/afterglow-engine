export class PaintPointerState {
  strokeActive = false;
  strokePointer: number | null = null;
  panPointer: number | null = null;
  strokeOpen = false;

  beginStroke(pointerId: number): boolean {
    const commitPrevious = this.strokeOpen;
    this.strokeActive = true;
    this.strokePointer = pointerId;
    this.panPointer = null;
    this.strokeOpen = true;
    return commitPrevious;
  }

  beginPan(pointerId: number): boolean {
    const commitPrevious = this.strokeOpen;
    this.strokeActive = false;
    this.strokePointer = null;
    this.panPointer = pointerId;
    this.strokeOpen = false;
    return commitPrevious;
  }

  finishStroke(pointerId: number): boolean {
    if (!this.strokeActive || this.strokePointer !== pointerId) return false;
    this.strokeActive = false;
    this.strokePointer = null;
    const commit = this.strokeOpen;
    this.strokeOpen = false;
    return commit;
  }

  finishPan(pointerId: number): boolean {
    if (this.panPointer !== pointerId) return false;
    this.panPointer = null;
    return true;
  }

  losePointer(pointerId: number): boolean {
    if (this.panPointer === pointerId) this.panPointer = null;
    if (this.strokePointer !== pointerId) return false;
    this.strokeActive = false;
    this.strokePointer = null;
    const commit = this.strokeOpen;
    this.strokeOpen = false;
    return commit;
  }

  finishForViewChange(): boolean {
    this.strokeActive = false;
    this.strokePointer = null;
    this.panPointer = null;
    const commit = this.strokeOpen;
    this.strokeOpen = false;
    return commit;
  }
}
