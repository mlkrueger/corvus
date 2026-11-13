// Sources panel state: which sources are enabled and each one's last fetched view (polled ~30 s, system/05).
import { errorText } from './api';
import { fetchSource, fetchSources } from './sources';
import type { SourceId } from './types/SourceId';
import type { SourceSummary } from './types/SourceSummary';
import type { SourceView } from './types/SourceView';

export const SOURCES_POLL_MS = 30_000;

export class SourcesState {
  summaries = $state<SourceSummary[]>([]);
  views = $state<Partial<Record<SourceId, SourceView>>>({});
  /** The last fetch error per source (and `list` for the source list); cleared by the next success. */
  errors = $state<Partial<Record<SourceId | 'list', string>>>({});
  readonly #seq: Partial<Record<SourceId, number>> = {};

  get enabled(): SourceId[] {
    return this.summaries.filter((s) => s.enabled).map((s) => s.source);
  }

  /** Fetches one source's view; an older response never overwrites a newer one. */
  async reload(source: SourceId): Promise<void> {
    const seq = (this.#seq[source] ?? 0) + 1;
    this.#seq[source] = seq;
    try {
      const view = await fetchSource(source);
      if (this.#seq[source] !== seq) return;
      this.views[source] = view;
      delete this.errors[source];
    } catch (e) {
      if (this.#seq[source] !== seq) return;
      this.errors[source] = errorText(e);
    }
  }

  /** Applies a view returned by a mutation (the filter save returns the fresh view). */
  setView(source: SourceId, view: SourceView): void {
    this.#seq[source] = (this.#seq[source] ?? 0) + 1;
    this.views[source] = view;
    delete this.errors[source];
  }

  /** One poll: the source list, then every enabled source's view. */
  async refresh(): Promise<void> {
    try {
      this.summaries = await fetchSources();
      delete this.errors.list;
    } catch (e) {
      this.errors.list = errorText(e);
      return;
    }
    await Promise.all(this.enabled.map((s) => this.reload(s)));
  }
}
