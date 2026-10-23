/**
 * Fixed-capacity FIFO ring buffer. Pushing into a full queue drops the oldest
 * item and counts it, so memory stays bounded no matter what.
 */
export class BoundedQueue<T> {
  private readonly buf: (T | undefined)[];
  private head = 0;
  private size = 0;
  dropped = 0;

  constructor(readonly capacity: number) {
    this.buf = new Array<T | undefined>(capacity);
  }

  get length(): number {
    return this.size;
  }

  push(item: T): void {
    const tail = (this.head + this.size) % this.capacity;
    this.buf[tail] = item;
    if (this.size === this.capacity) {
      this.head = (this.head + 1) % this.capacity;
      this.dropped++;
    } else {
      this.size++;
    }
  }

  /** Removes and returns up to `n` items from the front. */
  take(n: number): T[] {
    const count = Math.min(n, this.size);
    const out = new Array<T>(count);
    for (let i = 0; i < count; i++) {
      const idx = (this.head + i) % this.capacity;
      out[i] = this.buf[idx] as T;
      this.buf[idx] = undefined;
    }
    this.head = (this.head + count) % this.capacity;
    this.size -= count;
    return out;
  }

  /** Puts items back at the front (after a failed send) as far as room allows. */
  unshift(items: T[]): void {
    const room = this.capacity - this.size;
    const keep = items.slice(Math.max(0, items.length - room));
    this.dropped += items.length - keep.length;
    for (let i = keep.length - 1; i >= 0; i--) {
      this.head = (this.head - 1 + this.capacity) % this.capacity;
      this.buf[this.head] = keep[i];
      this.size++;
    }
  }
}
