//! The Jira Cloud work source ([system/07a](../../../spec/system/07a-jira.md), plan 0009).
//!
//! A read-only REST v3 client: Basic auth in a header only, redirects disabled, bounded timeouts and body size,
//! and the host fixed by configuration. Errors handed to the engine never contain credentials.

mod wire;

use std::sync::Arc;
use std::time::Duration;

use corvus_core::Config;
use corvus_core::sources::{BoxFuture, WorkSource};
use corvus_domain::adf::adf_to_text;
use corvus_domain::jql::{build_jql, default_filter, jira_priority, jira_status_category};
use corvus_domain::{Comment, CorvusError, Result, SourceFilter, SourceId, Ticket, TicketKey};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use serde_json::{Value, json};

/// Jira returns at most 50 issues per search page (OpenAPI `SearchAndReconcileRequestBean.maxResults`, checked 2026-11-13).
const PAGE_SIZE: u32 = 50;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(20);
const BODY_CAP: usize = 8 * 1024 * 1024;
/// Upper bound on pages per call, so a misbehaving server cannot loop a pass forever.
const MAX_PAGES: usize = 1000;
const SEARCH_FIELDS: [&str; 8] = [
    "summary",
    "status",
    "priority",
    "updated",
    "resolutiondate",
    "assignee",
    "project",
    "description",
];

/// The Jira source.
pub struct JiraSource {
    base: String,
    site: String,
    auth: HeaderValue,
    client: reqwest::Client,
    body_cap: usize,
    pool: String,
    poll: Duration,
    ignore: Vec<String>,
}

impl std::fmt::Debug for JiraSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JiraSource")
            .field("site", &self.site)
            .field("pool", &self.pool)
            .finish_non_exhaustive()
    }
}

/// The configured Jira source, or `None` when Jira is not configured.
pub fn from_config(config: &Config) -> std::result::Result<Option<Arc<dyn WorkSource>>, CorvusError> {
    let Some(j) = &config.jira else {
        return Ok(None);
    };
    let mut s = JiraSource::build(
        &j.base_url,
        &j.email,
        j.api_token.expose(),
        CONNECT_TIMEOUT,
        TOTAL_TIMEOUT,
        BODY_CAP,
        true,
    )?;
    s.pool = j.pool.clone();
    s.poll = Duration::from_secs(j.poll_seconds);
    s.ignore = j.ignore_accounts.clone();
    Ok(Some(Arc::new(s)))
}

impl JiraSource {
    #[allow(clippy::too_many_arguments)]
    fn build(
        base_url: &str,
        email: &str,
        token: &str,
        connect: Duration,
        total: Duration,
        body_cap: usize,
        https_only: bool,
    ) -> Result<Self> {
        let base = base_url.trim().trim_end_matches('/').to_owned();
        let scheme_ok = base.starts_with("https://") || (!https_only && base.starts_with("http://"));
        if !scheme_ok {
            return Err(CorvusError::Validation("Jira base URL must be https://".into()));
        }
        let site = base
            .split_once("://")
            .map_or("", |(_, rest)| rest)
            .split('/')
            .next()
            .unwrap_or("")
            .to_owned();
        let mut auth = HeaderValue::from_str(&format!(
            "Basic {}",
            wire::base64(format!("{email}:{token}").as_bytes())
        ))
        .map_err(|_| CorvusError::Validation("Jira credentials cannot be sent as a header".into()))?;
        auth.set_sensitive(true);
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(connect)
            .timeout(total)
            .https_only(https_only)
            .user_agent("corvus")
            .build()
            .map_err(|_| CorvusError::Internal("could not build the Jira HTTP client".into()))?;
        Ok(Self {
            base,
            site,
            auth,
            client,
            body_cap,
            pool: "default".into(),
            poll: Duration::from_secs(60),
            ignore: Vec::new(),
        })
    }

    /// Test-only constructor: plain `http://127.0.0.1:port` is allowed and the limits are adjustable.
    #[doc(hidden)]
    pub fn for_tests(
        base_url: &str,
        email: &str,
        token: &str,
        connect_timeout: Duration,
        total_timeout: Duration,
        body_cap: usize,
    ) -> Result<Self> {
        Self::build(
            base_url,
            email,
            token,
            connect_timeout,
            total_timeout,
            body_cap,
            false,
        )
    }

    /// One request; returns the parsed JSON body. `what` names the call in errors.
    async fn call(&self, req: reqwest::RequestBuilder, what: &str) -> Result<Value> {
        let fail = |why: &str| {
            let msg = format!("Jira API {why} for {what}");
            tracing::warn!(source = "jira", call = what, "{msg}");
            CorvusError::Internal(msg)
        };
        let mut resp = req
            .header(AUTHORIZATION, self.auth.clone())
            .header(ACCEPT, "application/json")
            .send()
            .await
            .map_err(|e| {
                fail(if e.is_timeout() {
                    "request timed out"
                } else if e.is_connect() {
                    "connection failed"
                } else {
                    "request failed"
                })
            })?;
        let status = resp.status();
        if !status.is_success() {
            return Err(fail(&format!("returned HTTP {}", status.as_u16())));
        }
        if resp.content_length().is_some_and(|n| n > self.body_cap as u64) {
            return Err(fail("response too large"));
        }
        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(|e| {
            fail(if e.is_timeout() {
                "request timed out"
            } else {
                "read failed"
            })
        })? {
            if body.len() + chunk.len() > self.body_cap {
                return Err(fail("response too large"));
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body).map_err(|_| fail("returned an unreadable body"))
    }

    async fn search_all(&self, filter: &SourceFilter) -> Result<Vec<Ticket>> {
        let jql = build_jql(filter);
        let url = format!("{}/rest/api/3/search/jql", self.base);
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let mut body = json!({"jql": jql, "fields": SEARCH_FIELDS, "maxResults": PAGE_SIZE});
            if let Some(t) = &token {
                body["nextPageToken"] = json!(t);
            }
            let page = self
                .call(
                    self.client
                        .post(&url)
                        .header(CONTENT_TYPE, "application/json")
                        .body(body.to_string()),
                    "search",
                )
                .await?;
            let issues = page
                .get("issues")
                .and_then(Value::as_array)
                .ok_or_else(|| bad("search", "no issues list"))?;
            for i in issues {
                out.push(self.ticket(i)?);
            }
            let last = page.get("isLast").and_then(Value::as_bool).unwrap_or(false);
            let next = page
                .get("nextPageToken")
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty());
            match next {
                Some(n) if !last => {
                    if token.as_deref() == Some(n) {
                        return Err(bad("search", "paging did not advance"));
                    }
                    token = Some(n.to_owned());
                }
                _ => return Ok(out),
            }
        }
        Err(bad("search", "too many pages"))
    }

    fn ticket(&self, issue: &Value) -> Result<Ticket> {
        let key = issue
            .get("key")
            .and_then(Value::as_str)
            .and_then(|k| TicketKey::parse(k).ok())
            .ok_or_else(|| bad("search", "an issue without a valid key"))?;
        let f = issue.get("fields").unwrap_or(&Value::Null);
        let text = |p: &str| f.pointer(p).and_then(Value::as_str);
        let status = text("/status/name").ok_or_else(|| bad("search", "an issue without a status"))?;
        let updated = text("/updated").ok_or_else(|| bad("search", "an issue without an updated stamp"))?;
        Ok(Ticket {
            project: text("/project/key").map_or_else(|| key.project().to_owned(), str::to_owned),
            summary: text("/summary").unwrap_or("").to_owned(),
            status: status.to_owned(),
            status_category: jira_status_category(text("/status/statusCategory/key").unwrap_or("")),
            priority: jira_priority(text("/priority/name").unwrap_or("")),
            url: format!("{}/browse/{key}", self.base),
            updated_stamp: updated.to_owned(),
            resolved_at: text("/resolutiondate").and_then(wire::parse_jira_time),
            assignee: text("/assignee/displayName").map(str::to_owned),
            description: f.get("description").map(adf_to_text).unwrap_or_default(),
            key,
        })
    }

    async fn comments_all(&self, key: &TicketKey) -> Result<Vec<Comment>> {
        // `key` is a validated TicketKey: only [A-Z0-9_-] can reach the path.
        let url = format!("{}/rest/api/3/issue/{}/comment", self.base, key.as_str());
        let mut out: Vec<Comment> = Vec::new();
        let mut start: u64 = 0;
        for _ in 0..MAX_PAGES {
            let page = self
                .call(
                    self.client.get(&url).query(&[
                        ("startAt", start.to_string()),
                        ("maxResults", PAGE_SIZE.to_string()),
                    ]),
                    "comments",
                )
                .await?;
            let items = page
                .get("comments")
                .and_then(Value::as_array)
                .ok_or_else(|| bad("comments", "no comments list"))?;
            for c in items {
                out.push(comment(c)?);
            }
            let total = page.get("total").and_then(Value::as_u64).unwrap_or(0);
            start += items.len() as u64;
            if items.is_empty() || start >= total {
                return Ok(out);
            }
        }
        Err(bad("comments", "too many pages"))
    }

    async fn statuses(&self) -> Result<Vec<String>> {
        let v = self
            .call(
                self.client.get(format!("{}/rest/api/3/status", self.base)),
                "statuses",
            )
            .await?;
        let list = v.as_array().ok_or_else(|| bad("statuses", "not a list"))?;
        let mut names: Vec<String> = list
            .iter()
            .filter_map(|s| s.get("name").and_then(Value::as_str).map(str::to_owned))
            .collect();
        names.sort();
        names.dedup();
        Ok(names)
    }
}

fn bad(what: &str, why: &str) -> CorvusError {
    let msg = format!("Jira API returned an unexpected response for {what}: {why}");
    tracing::warn!(source = "jira", call = what, "{msg}");
    CorvusError::Internal(msg)
}

fn comment(c: &Value) -> Result<Comment> {
    let id = match c.get("id") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => return Err(bad("comments", "a comment without an id")),
    };
    let order_key: i64 = id
        .parse()
        .map_err(|_| bad("comments", "a comment with a non-numeric id"))?;
    let author = |p: &str| c.pointer(p).and_then(Value::as_str).unwrap_or("").to_owned();
    Ok(Comment {
        order_key,
        author_id: author("/author/accountId"),
        author_name: author("/author/displayName"),
        created_at: c
            .get("created")
            .and_then(Value::as_str)
            .and_then(wire::parse_jira_time)
            .unwrap_or(0),
        body: c.get("body").map(adf_to_text).unwrap_or_default(),
        id,
    })
}

impl WorkSource for JiraSource {
    fn id(&self) -> SourceId {
        SourceId::Jira
    }
    fn site(&self) -> String {
        self.site.clone()
    }
    fn default_filter(&self) -> SourceFilter {
        default_filter()
    }
    fn pool(&self) -> String {
        self.pool.clone()
    }
    fn poll_interval(&self) -> Duration {
        self.poll
    }
    fn ignored_authors(&self) -> Vec<String> {
        self.ignore.clone()
    }
    fn search<'a>(&'a self, filter: &'a SourceFilter, _now_ms: i64) -> BoxFuture<'a, Result<Vec<Ticket>>> {
        Box::pin(self.search_all(filter))
    }
    fn comments<'a>(&'a self, key: &'a TicketKey) -> BoxFuture<'a, Result<Vec<Comment>>> {
        Box::pin(self.comments_all(key))
    }
    fn known_statuses(&self) -> BoxFuture<'_, Result<Vec<String>>> {
        Box::pin(self.statuses())
    }
}
