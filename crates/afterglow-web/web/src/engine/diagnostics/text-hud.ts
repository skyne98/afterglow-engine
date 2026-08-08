/** Bounded DOM text writer for game-owned HUD and status presentation. */
export class TextHud {
  constructor(private readonly element: HTMLElement | null) {}
  setText(text: string): void { if (this.element) this.element.textContent = text; }
  setVisible(visible: boolean): void {
    if (this.element) this.element.style.display = visible ? '' : 'none';
  }
}
