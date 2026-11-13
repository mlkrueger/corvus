# 07b — Linear adapter

The Linear implementation of the [work-source contract](07-work-sources.md). Added by owner direction as a v1
feature ([D0011](../decisions/0011-work-source-abstraction.md)). It mirrors the Jira source's behaviour exactly;
only the API, the vocabulary and the defaults differ. Crate: `corvus-linear`. Authoring agent: `linear`.

## Configuration

Enabled when `CORVUS_LINEAR_API_KEY` is set.

| Variable | Default | Notes |
|---|---|---|
| `CORVUS_LINEAR_API_KEY` | — | personal API key; secret; create it read-only if the workspace allows |
| `CORVUS_LINEAR_POLL_SECONDS` | 60 | minimum 15 |
| `CORVUS_LINEAR_POOL` | `default` | pool for created tasks |
| `CORVUS_LINEAR_IGNORE_USERS` | empty | comma-separated Linear user ids whose comments are not work |

The endpoint is the constant `https://api.linear.app/graphql`. It is not configurable (no caller- or
operator-supplied hosts).

## Vocabulary mapping

| Engine term | Linear |
|---|---|
| project (filter, rules) | **team key** (`ENG`) |
| ticket key | issue `identifier` (`ENG-123`) — fits `[A-Z][A-Z0-9_]*-\d+` |
| status | workflow state `name` |
| status category | state `type`: `triage`/`backlog`/`unstarted` → todo, `started` → in_progress, `completed`/`canceled` → done |
| resolved time | `completedAt` |
| "assigned to the API user" | `assignee: { isMe: { eq: true } }` |

## API usage (GraphQL, read-only)

Only queries are ever sent; the adapter contains no mutation documents.

| Purpose | Query |
|---|---|
| Search | `issues(filter: $filter, first: 100, after: $cursor)` selecting identifier, title, description, url, priority, updatedAt, completedAt, `state{name type}`, `team{key}`, `assignee{name}`; follow `pageInfo.endCursor` while `hasNextPage` |
| Comments | `issue(id: $identifier){ comments(first: 100, after: $cursor){ nodes{ id body createdAt user{ id name } } pageInfo{…} } }` |
| Statuses | `workflowStates(first: 250){ nodes{ name } }` → distinct names |

- Auth header: `Authorization: <api key>`.
- **The query text is a compile-time constant.** Everything variable — status names, team keys, the done-window
  cutoff, cursors, identifiers — travels in GraphQL `variables`, so no operator or tracker text is ever
  interpolated into a query. This is the Linear equivalent of the Jira adapter's JQL escaping.
- Filter object built in `corvus-domain` (pure, unit-tested):
  ```
  { assignee: {isMe: {eq: true}},
    or: [ {state: {name: {in: $statuses}}},
          {completedAt: {gte: $cutoff}} ],           -- cutoff = now − done_days, computed by Corvus; omitted when 0
    team: {key: {in: $projects}} }                   -- omitted when projects is empty
  ```
- A GraphQL response with an `errors` array, an HTTP error, or a rate-limit response fails the pass (no partial
  views). Rate-limit headers are logged; the next attempt is the next scheduled poll.
- Field names and filter operators are re-verified against Linear's current schema at implementation
  ([plan 0008](../plans/0008-work-sources-and-linear.md)).

## Mapping

| Ticket field | From |
|---|---|
| `priority` | Linear `priority`: 1 Urgent → 10, 2 High → 5, 3 Medium → 0, 4 Low → −5, 0 None → 0 |
| `url` | `url` from the response, accepted only if it parses as `https://linear.app/…`; otherwise empty |
| `updated_stamp` | `updatedAt`, verbatim string |
| `description`, comment `body` | Markdown, passed through as plain text (it is already readable; it is never rendered as markup) |
| comment `id` | comment UUID |
| comment `order_key` | `createdAt` as epoch milliseconds. Ids are UUIDs, so order comes from time; same-millisecond ties are handled by `hwm_ids` ([D0012](../decisions/0012-comment-tracking.md)) |
| comment `author_id` | `user.id` (absent for integration/bot comments → treated as author `""`, never ignored by default) |

## Default filter

`statuses`: Backlog, Todo, In Progress, In Review · `done_days`: 7 · `projects`: all teams ·
`auto_queue`: **no rules**. Nothing auto-queues until the operator adds a rule; tickets can be queued by hand
from the board. (Jira's default rule names a specific project from the brief; Linear has no equivalent safe
guess — see [11](11-defaults.md).)
