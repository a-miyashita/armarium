import { describe, expect, it } from "vitest";
import { PageCache, type Page } from "./pageCache";

function fakePage(offset: number, limit: number): Page {
  const rids = Array.from({ length: limit }, (_, i) => offset + i + 1);
  return { rids, rowNumbers: rids, data: [rids.map(String)], notes: new Map() };
}

describe("PageCache", () => {
  it("loads pages on demand and keeps at most maxPages", async () => {
    const calls: number[] = [];
    const loaded: number[] = [];
    const cache = new PageCache(
      async (offset, limit) => {
        calls.push(offset);
        return fakePage(offset, limit);
      },
      (p) => loaded.push(p),
      10,
      3,
    );
    expect(cache.row(25)).toBeUndefined();
    await cache.ensure(25);
    expect(cache.row(25)?.page.rids[cache.row(25)!.index]).toBe(26);
    cache.prefetch(0, 59, 100);
    await Promise.all([0, 1, 2, 3, 4, 5].map((p) => cache.load(p)));
    expect(cache.loadedPages().length).toBe(3);
    expect(calls.filter((c) => c === 20).length).toBe(1);
  });

  it("ignores pages that arrive after clear", async () => {
    let resolve!: (p: Page) => void;
    const cache = new PageCache(
      () => new Promise<Page>((r) => (resolve = r)),
      () => {},
      10,
      5,
    );
    const pending = cache.load(0);
    cache.clear();
    resolve(fakePage(0, 10));
    await pending;
    expect(cache.loadedPages()).toEqual([]);
  });
});
