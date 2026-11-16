<script lang="ts">
  import { MESSAGE_TYPES, sendBroadcast, sendDirect, sendReply } from '../lib/actions';
  import { errorText } from '../lib/api';
  import { ago } from '../lib/format';
  import { filterMessages, recipientLabel, replyType, type MessageFilter } from '../lib/messages';
  import type { MessageListItem } from '../lib/types/MessageListItem';
  import type { MessageType } from '../lib/types/MessageType';

  interface Props {
    messages: MessageListItem[];
    filter: MessageFilter;
    agents: string[];
    operator: string;
    now: number;
    onFilter: (filter: MessageFilter) => void;
    onChanged: () => void;
  }
  let { messages, filter, agents, operator, now, onFilter, onChanged }: Props = $props();

  const visible = $derived(filterMessages(messages, filter));
  const filterValue = $derived(filter.mode === 'agent' ? `agent:${filter.agent}` : filter.mode);

  function chooseFilter(value: string): void {
    if (value === 'unread') onFilter({ mode: 'unread' });
    else if (value.startsWith('agent:')) onFilter({ mode: 'agent', agent: value.slice('agent:'.length) });
    else onFilter({ mode: 'all' });
  }

  // Compose
  let to = $state('');
  let composeType = $state<MessageType>('note');
  let composeBody = $state('');
  let wakeEveryone = $state(false);
  let composeBusy = $state(false);
  let composeError = $state<string | null>(null);

  async function send(e: SubmitEvent): Promise<void> {
    e.preventDefault();
    const body = composeBody.trim();
    if (body === '' || composeBusy) return;
    composeBusy = true;
    composeError = null;
    try {
      if (to === '') await sendBroadcast(operator, body, composeType, wakeEveryone);
      else await sendDirect(operator, to, body, composeType);
      composeBody = '';
      wakeEveryone = false;
      onChanged();
    } catch (err) {
      composeError = errorText(err);
    } finally {
      composeBusy = false;
    }
  }

  // Reply
  let replyingTo = $state<number | null>(null);
  let replyBody = $state('');
  let replyKind = $state<MessageType>('note');
  let replyBusy = $state(false);
  let replyError = $state<string | null>(null);

  function startReply(m: MessageListItem): void {
    replyingTo = m.id;
    replyBody = '';
    replyKind = replyType(m);
    replyError = null;
  }

  async function reply(e: SubmitEvent, m: MessageListItem): Promise<void> {
    e.preventDefault();
    const body = replyBody.trim();
    if (body === '' || replyBusy) return;
    replyBusy = true;
    replyError = null;
    try {
      await sendReply(operator, m, body, replyKind);
      replyingTo = null;
      replyBody = '';
      onChanged();
    } catch (err) {
      replyError = errorText(err);
    } finally {
      replyBusy = false;
    }
  }
</script>

<section class="panel" aria-labelledby="messages-h">
  <div class="panel-head">
    <h2 id="messages-h">Messages</h2>
    <label class="inline">
      Filter
      <select aria-label="Message filter" value={filterValue} onchange={(e) => chooseFilter(e.currentTarget.value)}>
        <option value="all">all</option>
        <option value="unread">unread</option>
        {#each agents as a (a)}
          <option value="agent:{a}">agent: {a}</option>
        {/each}
      </select>
    </label>
  </div>

  <form class="compose" onsubmit={send} aria-label="Compose message">
    <div class="compose-row">
      <label class="inline">
        To
        <select aria-label="Recipient" bind:value={to}>
          <option value="">everyone</option>
          {#each agents as a (a)}<option value={a}>{a}</option>{/each}
        </select>
      </label>
      <label class="inline">
        Type
        <select aria-label="Message type" bind:value={composeType}>
          {#each MESSAGE_TYPES as t (t)}<option value={t}>{t}</option>{/each}
        </select>
      </label>
      <span class="muted">as {operator}</span>
      {#if to === ''}
        <label class="inline" title="Starts a turn in every idle session now. Without it, each session sees the message at its next turn.">
          <input type="checkbox" aria-label="Wake everyone" bind:checked={wakeEveryone} /> wake everyone now
        </label>
      {/if}
    </div>
    <textarea aria-label="Message body" rows="2" placeholder="Message" bind:value={composeBody}></textarea>
    <div class="compose-row">
      <button type="submit" disabled={composeBusy || composeBody.trim() === ''}>Send</button>
      {#if composeError}<span class="error" role="alert">{composeError}</span>{/if}
    </div>
  </form>

  {#if visible.length === 0}
    <p class="empty">No messages.</p>
  {:else}
    <ul class="rows scroll tall">
      {#each visible as m (m.id)}
        <li class="row message" class:unread={m.unread_by.length > 0} data-message={m.id}>
          <div class="row-main">
            <strong class="agent">{m.sender}</strong>
            <span class="muted">to</span>
            <span class="agent">{recipientLabel(m)}</span>
            <span class="badge type-{m.type}">{m.type}</span>
            {#if m.wake}<span class="badge wake">wake</span>{/if}
            <span class="muted">{ago(m.created_at, now)}</span>
          </div>
          <div class="row-sub muted thread">thread: {m.thread_title}</div>
          <div class="row-sub body">{m.body}</div>
          <div class="row-sub muted receipts">
            {#if m.read_by.length > 0}<span>read by {m.read_by.join(', ')}</span>{/if}
            {#if m.unread_by.length > 0}<span class="unread-by">unread by {m.unread_by.join(', ')}</span>{/if}
            {#if m.reassigned_to}<span class="reassigned">reassigned to {m.reassigned_to}</span>{/if}
            {#if m.reassigned_from !== null}<span class="reassigned">moved here from message {m.reassigned_from}</span>{/if}
          </div>
          {#if replyingTo === m.id}
            <form class="reply" onsubmit={(e) => reply(e, m)} aria-label="Reply to message {m.id}">
              <div class="compose-row">
                <label class="inline">
                  Type
                  <select aria-label="Reply type" bind:value={replyKind}>
                    {#each MESSAGE_TYPES as t (t)}<option value={t}>{t}</option>{/each}
                  </select>
                </label>
              </div>
              <textarea aria-label="Reply body" rows="2" bind:value={replyBody}></textarea>
              <div class="compose-row">
                <button type="submit" disabled={replyBusy || replyBody.trim() === ''}>Send reply</button>
                <button type="button" onclick={() => (replyingTo = null)}>Cancel</button>
                {#if replyError}<span class="error" role="alert">{replyError}</span>{/if}
              </div>
            </form>
          {:else}
            <div class="row-sub"><button type="button" class="link-button" onclick={() => startReply(m)}>Reply</button></div>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>
