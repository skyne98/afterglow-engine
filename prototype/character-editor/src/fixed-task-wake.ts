export class FixedTaskWake {
  private readonly channel = new MessageChannel();
  private pending = false;

  constructor(callback: () => void) {
    this.channel.port1.onmessage = () => {
      this.pending = false;
      callback();
    };
  }

  get isPending(): boolean {
    return this.pending;
  }

  schedule(): void {
    if (this.pending) return;
    this.pending = true;
    this.channel.port2.postMessage(null);
  }

  close(): void {
    this.pending = false;
    this.channel.port1.close();
    this.channel.port2.close();
  }
}
