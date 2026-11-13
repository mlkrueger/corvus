//! The Linear work source ([system/07b](../../../spec/system/07b-linear.md)): a read-only GraphQL client.
//!
//! Only queries are sent, and their text is the constant documents in `corvus_domain::linear`; every variable
//! value travels in `variables`. Any GraphQL error, HTTP error or rate limit fails the whole call (no partial
//! results). Errors never carry the API key, headers or URLs.

use std::sync::Arc;
use std::time::Duration;

use corvus_core::Config;
use corvus_core::config::{LinearConfig, Secret};
use corvus_core::sources::{BoxFuture, WorkSource};
use corvus_domain::linear as dom;
use corvus_domain::{Comment, CorvusError, Result, SourceFilter, SourceId, Ticket, TicketKey};
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderValue};
use serde_json::{Value, json};

/// The one endpoint. Not configurable.
const ENDPOINT: &str = "https://api.linear.app/graphql";
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
const MAX_PAGES: usize = 500;
const MAX_ERR_CHARS: usize = 200;

/// Transport limits. Production uses [`Limits::default`]; tests shorten them.
#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub connect_timeout: Duration,
    pub total_timeout: Duration,
    pub max_body_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(5),
            total_timeout: Duration::from_secs(20),
            max_body_bytes: MAX_BODY_BYTES,
        }
    }
}

/// The Linear source.
#[derive(Debug)]
pub struct LinearSource {
    endpoint: String,
    api_key: Secret,
    client: reqwest::Client,
    pool: String,
    poll: Duration,
    ignored: Vec<String>,
    max_body_bytes: usize,
}

/// The configured Linear source, or `None` when Linear is not configured.
pub fn from_config(config: &Config) -> Result<Option<Arc<dyn WorkSource>>> {
    let Some(c) = &config.linear else {
        return Ok(None);
    };
    Ok(Some(Arc::new(LinearSource::build(
        ENDPOINT,
        c,
        Limits::default(),
    )?)))
}

fn internal(msg: impl Into<String>) -> CorvusError {
    CorvusError::Internal(msg.into())
}

impl LinearSource {
    /// Test-only: point at a fake server on loopback (`http://127.0.0.1:…` is allowed here and nowhere else).
    #[doc(hidden)]
    pub fn with_endpoint_for_tests(endpoint: &str, config: &LinearConfig, limits: Limits) -> Result<Self> {
        Self::build(endpoint, config, limits)
    }

    fn build(endpoint: &str, c: &LinearConfig, limits: Limits) -> Result<Self> {
        let loopback_http =
            endpoint.starts_with("http://127.0.0.1:") || endpoint.starts_with("http://localhost:");
        if !endpoint.starts_with("https://") && !(cfg!(any(test, debug_assertions)) && loopback_http) {
            return Err(internal("Linear endpoint must be https"));
        }
        if endpoint != ENDPOINT && !loopback_http {
            return Err(internal("Linear endpoint is fixed"));
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(limits.connect_timeout)
            .timeout(limits.total_timeout)
            .user_agent("corvus")
            .build()
            .map_err(|_| internal("could not build the Linear HTTP client"))?;
        Ok(Self {
            endpoint: endpoint.to_owned(),
            api_key: c.api_key.clone(),
            client,
            pool: c.pool.clone(),
            poll: Duration::from_secs(c.poll_seconds),
            ignored: c.ignore_users.clone(),
            max_body_bytes: limits.max_body_bytes,
        })
    }

    /// Removes the key from text that may reach logs or error strings, and clips it.
    fn scrub(&self, s: &str) -> String {
        let key = self.api_key.expose();
        let s = if key.is_empty() {
            s.to_owned()
        } else {
            s.replace(key, "[redacted]")
        };
        let s: String = s.chars().filter(|c| !c.is_control()).collect();
        if s.chars().count() > MAX_ERR_CHARS {
            let mut t: String = s.chars().take(MAX_ERR_CHARS).collect();
            t.push('…');
            t
        } else {
            s
        }
    }

    fn fail(&self, msg: &str) -> CorvusError {
        let msg = self.scrub(msg);
        tracing::warn!(source = "linear", error = %msg, "Linear request failed");
        internal(msg)
    }

    /// One GraphQL request; returns `data`. Fails on transport errors, non-2xx, rate limits and `errors`.
    async fn query(&self, query: &str, variables: Value) -> Result<Value> {
        let mut auth = HeaderValue::from_str(self.api_key.expose())
            .map_err(|_| internal("Linear API key is not a valid header value"))?;
        auth.set_sensitive(true);
        let sent = self
            .client
            .post(&self.endpoint)
            .header(AUTHORIZATION, auth)
            .header(ACCEPT, "application/json")
            .json(&json!({ "query": query, "variables": variables }))
            .send()
            .await;
        let mut resp = match sent {
            Ok(r) => r,
            Err(e) => {
                let what = if e.is_timeout() {
                    "Linear request timed out"
                } else if e.is_connect() {
                    "could not connect to Linear"
                } else {
                    "Linear request failed"
                };
                return Err(self.fail(what));
            }
        };
        let status = resp.status();
        if status.as_u16() == 429 || resp.headers().contains_key("retry-after") {
            let h = |n: &str| {
                resp.headers()
                    .get(n)
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.chars().take(40).collect::<String>())
            };
            tracing::warn!(
                source = "linear",
                status = status.as_u16(),
                retry_after = ?h("retry-after"),
                requests_remaining = ?h("x-ratelimit-requests-remaining"),
                requests_reset = ?h("x-ratelimit-requests-reset"),
                complexity_remaining = ?h("x-ratelimit-complexity-remaining"),
                "Linear rate-limit headers"
            );
        }
        if let Some(len) = resp.content_length()
            && len > self.max_body_bytes as u64
        {
            return Err(self.fail("Linear response too large"));
        }
        let mut body: Vec<u8> = Vec::new();
        loop {
            match resp.chunk().await {
                Ok(Some(c)) => {
                    if body.len() + c.len() > self.max_body_bytes {
                        return Err(self.fail("Linear response too large"));
                    }
                    body.extend_from_slice(&c);
                }
                Ok(None) => break,
                Err(e) => {
                    let what = if e.is_timeout() {
                        "Linear request timed out"
                    } else {
                        "Linear response could not be read"
                    };
                    return Err(self.fail(what));
                }
            }
        }
        let parsed: Option<Value> = serde_json::from_slice(&body).ok();
        let rate_limited = status.as_u16() == 429
            || parsed.as_ref().is_some_and(|v| {
                v["errors"].as_array().is_some_and(|es| {
                    es.iter().any(|e| {
                        e["extensions"]["code"] == "RATELIMITED" || e["extensions"]["type"] == "ratelimited"
                    })
                })
            });
        if rate_limited {
            return Err(self.fail("Linear rate limit exceeded"));
        }
        if !status.is_success() {
            return Err(self.fail(&format!("Linear API returned HTTP {}", status.as_u16())));
        }
        let Some(mut v) = parsed else {
            return Err(self.fail("Linear returned a response that is not JSON"));
        };
        if let Some(errs) = v.get("errors").filter(|e| !e.is_null()) {
            let msg = errs
                .as_array()
                .and_then(|a| a.first())
                .and_then(|e| e["message"].as_str())
                .unwrap_or("unknown error");
            return Err(self.fail(&format!("Linear GraphQL error: {msg}")));
        }
        match v.get_mut("data").map(Value::take) {
            Some(d) if !d.is_null() => Ok(d),
            _ => Err(self.fail("Linear response had no data")),
        }
    }

    /// Follows `pageInfo` of the connection at `path` (e.g. `["issues"]`); returns every node.
    async fn all_nodes(&self, query: &str, base: Value, path: &[&str]) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        let mut cursor = Value::Null;
        for _ in 0..MAX_PAGES {
            let mut vars = base.clone();
            vars["cursor"] = cursor.clone();
            let data = self.query(query, vars).await?;
            let mut conn = &data;
            for p in path {
                conn = &conn[*p];
            }
            let Some(nodes) = conn["nodes"].as_array() else {
                return Err(self.fail("Linear response had an unexpected shape"));
            };
            out.extend(nodes.iter().cloned());
            if conn["pageInfo"]["hasNextPage"].as_bool() != Some(true) {
                return Ok(out);
            }
            let next = conn["pageInfo"]["endCursor"].clone();
            if !next.is_string() || next == cursor {
                return Err(self.fail("Linear pagination did not advance"));
            }
            cursor = next;
        }
        Err(self.fail("Linear pagination exceeded the page limit"))
    }

    fn ticket(&self, n: &Value) -> Result<Ticket> {
        let s = |v: &Value| v.as_str().map(str::to_owned);
        let shape = || self.fail("Linear issue had an unexpected shape");
        let key = TicketKey::parse(n["identifier"].as_str().ok_or_else(shape)?)
            .map_err(|_| self.fail("Linear returned a malformed issue identifier"))?;
        let priority = n["priority"].as_f64().map_or(0, |p| p.round() as i64);
        Ok(Ticket {
            project: n["team"]["key"]
                .as_str()
                .unwrap_or_else(|| key.project())
                .to_owned(),
            key,
            summary: s(&n["title"]).unwrap_or_default(),
            status: s(&n["state"]["name"]).ok_or_else(shape)?,
            status_category: dom::state_category(n["state"]["type"].as_str().unwrap_or("")),
            priority: dom::map_priority(priority),
            url: dom::accept_url(n["url"].as_str().unwrap_or("")),
            updated_stamp: s(&n["updatedAt"]).ok_or_else(shape)?,
            resolved_at: n["completedAt"].as_str().and_then(dom::parse_rfc3339_ms),
            assignee: s(&n["assignee"]["name"]),
            description: s(&n["description"]).unwrap_or_default(),
        })
    }

    fn comment(&self, n: &Value) -> Result<Comment> {
        let shape = || self.fail("Linear comment had an unexpected shape");
        let created = n["createdAt"]
            .as_str()
            .and_then(dom::parse_rfc3339_ms)
            .ok_or_else(shape)?;
        Ok(Comment {
            id: n["id"].as_str().ok_or_else(shape)?.to_owned(),
            order_key: created,
            author_id: n["user"]["id"].as_str().unwrap_or("").to_owned(),
            author_name: n["user"]["name"].as_str().unwrap_or("").to_owned(),
            created_at: created,
            body: n["body"].as_str().unwrap_or("").to_owned(),
        })
    }
}

impl WorkSource for LinearSource {
    fn id(&self) -> SourceId {
        SourceId::Linear
    }

    fn site(&self) -> String {
        "linear.app".into()
    }

    fn default_filter(&self) -> SourceFilter {
        dom::default_filter()
    }

    fn pool(&self) -> String {
        self.pool.clone()
    }

    fn poll_interval(&self) -> Duration {
        self.poll
    }

    fn ignored_authors(&self) -> Vec<String> {
        self.ignored.clone()
    }

    fn search<'a>(&'a self, filter: &'a SourceFilter, now_ms: i64) -> BoxFuture<'a, Result<Vec<Ticket>>> {
        Box::pin(async move {
            filter.validate()?;
            let base = json!({ "filter": dom::issue_filter(filter, now_ms) });
            let nodes = self.all_nodes(dom::SEARCH_QUERY, base, &["issues"]).await?;
            nodes.iter().map(|n| self.ticket(n)).collect()
        })
    }

    fn comments<'a>(&'a self, key: &'a TicketKey) -> BoxFuture<'a, Result<Vec<Comment>>> {
        Box::pin(async move {
            let base = json!({ "id": key.as_str() });
            let nodes = self
                .all_nodes(dom::COMMENTS_QUERY, base, &["issue", "comments"])
                .await?;
            let mut out: Vec<Comment> = nodes.iter().map(|n| self.comment(n)).collect::<Result<_>>()?;
            out.sort_by(|a, b| (a.order_key, &a.id).cmp(&(b.order_key, &b.id)));
            Ok(out)
        })
    }

    fn known_statuses(&self) -> BoxFuture<'_, Result<Vec<String>>> {
        Box::pin(async move {
            let data = self.query(dom::STATUSES_QUERY, json!({})).await?;
            let Some(nodes) = data["workflowStates"]["nodes"].as_array() else {
                return Err(self.fail("Linear response had an unexpected shape"));
            };
            let mut names: Vec<String> = Vec::new();
            for n in nodes {
                if let Some(s) = n["name"].as_str()
                    && !names.iter().any(|x| x == s)
                {
                    names.push(s.to_owned());
                }
            }
            Ok(names)
        })
    }
}
