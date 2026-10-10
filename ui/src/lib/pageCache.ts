// An LRU cache of row pages for the grid ([ADR-0014]).
// Default: 200 rows for each page, 2 pages read ahead, and at most 50 pages.

export const PAGE_SIZE = 200;
export const READ_AHEAD = 2;
export const MAX_PAGES = 50;

export interface Page {
  rids: number[];
  rowNumbers: number[];
  /** data[column][row] */
  data: (string | null)[][];
  notes: Map<string, { kind: string; text: string }>;
}

export type Fetcher = (offset: number, limit: number) => Promise<Page>;

export class PageCache {
  private pages = new Map<number, Page>();
  private loading = new Map<number, Promise<Page | undefined>>();
  private generation = 0;

  constructor(
    private fetcher: Fetcher,
    private onLoaded: (pageIndex: number) => void,
    private pageSize = PAGE_SIZE,
    private maxPages = MAX_PAGES,
  ) {}

  /** The page of a row, or undefined if it is not loaded (the load starts). */
  row(row: number): { page: Page; index: number } | undefined {
    const p = Math.floor(row / this.pageSize);
    const page = this.pages.get(p);
    if (!page) {
      void this.load(p);
      return undefined;
    }
    // Move the page to the end (most recently used).
    this.pages.delete(p);
    this.pages.set(p, page);
    return { page, index: row - p * this.pageSize };
  }

  /** Loads the pages around a visible range. */
  prefetch(firstRow: number, lastRow: number, total: number) {
    const first = Math.max(0, Math.floor(firstRow / this.pageSize) - READ_AHEAD);
    const last = Math.min(
      Math.floor(Math.max(0, total - 1) / this.pageSize),
      Math.floor(lastRow / this.pageSize) + READ_AHEAD,
    );
    for (let p = first; p <= last; p++) {
      if (!this.pages.has(p)) void this.load(p);
    }
  }

  /** Loads a page and waits for it. */
  async ensure(row: number): Promise<{ page: Page; index: number } | undefined> {
    const p = Math.floor(row / this.pageSize);
    if (!this.pages.has(p)) await this.load(p);
    return this.row(row);
  }

  load(p: number): Promise<Page | undefined> {
    const ready = this.pages.get(p);
    if (ready) return Promise.resolve(ready);
    const existing = this.loading.get(p);
    if (existing) return existing;
    const gen = this.generation;
    const promise = this.fetcher(p * this.pageSize, this.pageSize)
      .then((page) => {
        if (gen !== this.generation) return undefined;
        this.pages.set(p, page);
        while (this.pages.size > this.maxPages) {
          const oldest = this.pages.keys().next().value as number;
          this.pages.delete(oldest);
        }
        this.onLoaded(p);
        return page;
      })
      .catch(() => undefined)
      .finally(() => {
        if (this.loading.get(p) === promise) this.loading.delete(p);
      });
    this.loading.set(p, promise);
    return promise;
  }

  /** Discards all pages (after a change of the table). */
  clear() {
    this.generation++;
    this.pages.clear();
    this.loading.clear();
  }

  get pageSizeValue() {
    return this.pageSize;
  }

  loadedPages(): number[] {
    return [...this.pages.keys()];
  }
}
