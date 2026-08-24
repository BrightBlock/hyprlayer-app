// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { debouncedJSONStorage, flushPersistedWrites } from "./persistedStorage";

type Blob = { n: number };

const storage = debouncedJSONStorage<Blob>()!;

// Count real writes rather than reading the value back: the whole point of the
// adapter is how MANY times it reaches localStorage, not what lands there.
let writes: string[] = [];

beforeEach(() => {
  vi.useFakeTimers();
  const backing = new Map<string, string>();
  // A stand-in rather than a spy on jsdom's own Storage: the adapter reaches
  // for the global on every call, so swapping the whole object is both simpler
  // and closer to what it actually talks to.
  Object.defineProperty(window, "localStorage", {
    configurable: true,
    value: {
      getItem: (k: string) => backing.get(k) ?? null,
      setItem: (k: string, v: string) => {
        writes.push(k);
        backing.set(k, v);
      },
      removeItem: (k: string) => void backing.delete(k),
      clear: () => backing.clear(),
    },
  });
  flushPersistedWrites();
  writes = [];
});

afterEach(() => {
  vi.useRealTimers();
});

const put = (n: number) => storage.setItem("k", { state: { n }, version: 1 });

describe("debouncedJSONStorage", () => {
  it("collapses a burst of writes into one", () => {
    for (let n = 0; n < 50; n++) put(n);
    expect(writes).toHaveLength(0); // nothing has reached storage yet

    vi.advanceTimersByTime(1000);
    expect(writes).toEqual(["k"]);
  });

  it("stores the newest value in the burst, not the first", () => {
    put(1);
    put(2);
    put(3);
    vi.advanceTimersByTime(1000);
    expect(storage.getItem("k")).toMatchObject({ state: { n: 3 } });
  });

  it("skips a write that would store what is already there", () => {
    put(1);
    vi.advanceTimersByTime(1000);
    expect(writes).toHaveLength(1);

    put(1); // same bytes
    vi.advanceTimersByTime(1000);
    expect(writes).toHaveLength(1);

    put(2); // different
    vi.advanceTimersByTime(1000);
    expect(writes).toHaveLength(2);
  });

  it("serves a queued value to a read that races the write", () => {
    put(7);
    // Still pending — a rehydrate here must not see the older stored blob.
    expect(storage.getItem("k")).toMatchObject({ state: { n: 7 } });
    expect(writes).toHaveLength(0);
  });

  it("lands pending writes when the page goes away", () => {
    put(9);
    expect(writes).toHaveLength(0);

    window.dispatchEvent(new Event("pagehide"));
    expect(writes).toEqual(["k"]);
    expect(storage.getItem("k")).toMatchObject({ state: { n: 9 } });
  });

  it("drops a queued write when the key is removed", () => {
    put(1);
    storage.removeItem("k");
    vi.advanceTimersByTime(1000);
    expect(writes).toHaveLength(0);
  });
});
