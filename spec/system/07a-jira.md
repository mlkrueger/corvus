# 07a — Jira adapter

The Jira Cloud implementation of the [work-source contract](07-work-sources.md). Requirements: brief §4.9.
Crate: `corvus-jira`. Authoring agent: `jira`.

## Configuration

Enabled only when all three are set; if only some are, startup fails with an error naming the missing ones.

| Variable | Default | Notes |
|---|---|---|
| `CORVUS_JIRA_BASE_URL` | — | must be `https://…`; the only host ever contacted |
| `CORVUS_JIRA_EMAIL` | — | Basic auth user |
| `CORVUS_JIRA_API_TOKEN` | — | Basic auth password; secret |
| `CORVUS_JIRA_POLL_SECONDS` | 60 | minimum 15 |
| `CORVUS_JIRA_POOL` | `default` | pool for created tasks |
| `CORVUS_JIRA_IGNORE_ACCOUNTS` | empty | comma-separated account ids whose comments are not work |

## API usage (REST v3, read-only)

| Purpose | Call |
|---|---|
| Search | `POST /rest/api/3/search/jql` (enhanced JQL search) with `jql`, `fields`, `maxResults`; follow `nextPageToken` until the last page |
| Comments | `GET /rest/api/3/issue/{key}/comment` paged with `startAt` until `total` is reached |
| Statuses | `GET /rest/api/3/status` → distinct names |

Endpoint shapes are re-verified against Atlassian's current docs at implementation ([plan 0009](../plans/0009-jira-source.md)).

## Query building

Built only from the validated filter, in `corvus-domain` (pure, unit-tested):

```
assignee = currentUser()
AND ( status in ("A", "B")  OR  (statusCategory = Done AND resolved >= -7d) )     -- either side may be absent
[AND project in (AA, LAI)]
ORDER BY updated DESC
```

Status names are emitted as double-quoted JQL strings with `\` and `"` backslash-escaped; control characters
were already rejected by validation. Project keys match `[A-Z][A-Z0-9_]{0,9}` and need no quoting.

## Mapping

| Ticket field | From |
|---|---|
| `key`, `project` | `key`, `fields.project.key` |
| `status`, `status_category` | `fields.status.name`; `statusCategory.key`: `new`→todo, `indeterminate`→in_progress, `done`→done |
| `priority` | name: Highest 10, High 5, Medium 0, Low −5, Lowest −10, anything else 0 |
| `url` | `<base>/browse/<key>` (built by Corvus, not read from the response) |
| `updated_stamp` | `fields.updated`, verbatim string |
| `resolved_at` | `fields.resolutiondate` |
| `description`, comment `body` | Atlassian Document Format → plain text (below) |
| comment `id`, `order_key` | `id`; `id` parsed as an integer (Jira comment ids are numeric). A non-numeric id fails the pass. |
| comment `author_id` | `author.accountId` |

## Rich text → plain text

`adf_to_text(json) -> String`, pure, in `corvus-domain`. Paragraphs and headings become lines; bullet and
ordered lists become `- ` / `1. ` with indentation for nesting; code blocks are fenced; `hardBreak` is a
newline; mentions become `@name`; links become `text (url)`; tables become one line per row with ` | `
separators; media becomes `[attachment]`; unknown nodes contribute their children's text. Never fails: a value
that is not a document yields an empty string.

## Default filter

`statuses`: Backlog, To Do, In Progress, Waiting for response · `done_days`: 7 · `projects`: all ·
`auto_queue`: one rule — project `AA`, statuses To Do + In Progress.
