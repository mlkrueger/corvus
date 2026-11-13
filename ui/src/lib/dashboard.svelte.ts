// Dashboard state: what the polls fetched plus the UI selections panels share.
import { errorText, request } from './api';
import { ALL_MESSAGES, messagesPath, type MessageFilter } from './messages';
import type { MessageListItem } from './types/MessageListItem';
import type { Overview } from './types/Overview';

export class Dashboard {
  overview = $state<Overview | null>(null);
  messages = $state<MessageListItem[]>([]);
  error = $state<string | null>(null);
  updatedAt = $state<number | null>(null);
  filter = $state<MessageFilter>(ALL_MESSAGES);

  /** The operator identity comes from the server (overview.operator_name); "operator" until the first poll. */
  get operator(): string {
    return this.overview?.operator_name ?? 'operator';
  }

  /** Distinct agent names seen in sessions and messages, for the recipient and filter pickers. */
  get agents(): string[] {
    const names: string[] = [];
    for (const s of this.overview?.sessions ?? []) names.push(s.agent_name);
    for (const m of this.messages) {
      names.push(m.sender);
      if (m.recipient) names.push(m.recipient);
    }
    if (this.filter.mode === 'agent') names.push(this.filter.agent);
    return names.filter((n, i) => n !== this.operator && names.indexOf(n) === i).sort();
  }

  async refresh(): Promise<void> {
    const [overview, messages] = await Promise.allSettled([
      request<Overview>('GET', '/api/overview'),
      request<MessageListItem[]>('GET', messagesPath(this.filter)),
    ]);
    const errors: string[] = [];
    if (overview.status === 'fulfilled') this.overview = overview.value;
    else errors.push(errorText(overview.reason));
    if (messages.status === 'fulfilled') this.messages = messages.value;
    else errors.push(errorText(messages.reason));
    this.error = errors.length ? errors.join('; ') : null;
    if (errors.length < 2) this.updatedAt = Date.now();
  }

}
