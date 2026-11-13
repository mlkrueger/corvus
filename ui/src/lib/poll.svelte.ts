// Polling with pause, visibility handling and no overlap (system/05 "Data flow").

export interface PollDoc {
  hidden: boolean;
  addEventListener(type: 'visibilitychange', fn: () => void): void;
  removeEventListener(type: 'visibilitychange', fn: () => void): void;
}

export class Poller {
  paused = $state(false);
  #inFlight = false;
  #timer: ReturnType<typeof setInterval> | null = null;
  readonly #tick: () => Promise<void>;
  readonly #intervalMs: number;
  readonly #doc: PollDoc;
  readonly #onVisibility = () => {
    if (!this.#doc.hidden) void this.run();
  };

  constructor(tick: () => Promise<void>, intervalMs = 3000, doc: PollDoc = document) {
    this.#tick = tick;
    this.#intervalMs = intervalMs;
    this.#doc = doc;
  }

  start(): void {
    if (this.#timer !== null) return;
    this.#doc.addEventListener('visibilitychange', this.#onVisibility);
    this.#timer = setInterval(() => void this.run(), this.#intervalMs);
    void this.run();
  }

  stop(): void {
    if (this.#timer !== null) clearInterval(this.#timer);
    this.#timer = null;
    this.#doc.removeEventListener('visibilitychange', this.#onVisibility);
  }

  pause(): void {
    this.paused = true;
  }

  resume(): void {
    this.paused = false;
    void this.run();
  }

  toggle(): void {
    if (this.paused) this.resume();
    else this.pause();
  }

  /** One poll, unless paused, hidden or one is already in flight. Returns whether the tick ran. */
  async run(force = false): Promise<boolean> {
    if (this.#inFlight) return false;
    if (!force && (this.paused || this.#doc.hidden)) return false;
    this.#inFlight = true;
    try {
      await this.#tick();
    } catch {
      // The tick records its own errors; the loop never dies.
    } finally {
      this.#inFlight = false;
    }
    return true;
  }
}
