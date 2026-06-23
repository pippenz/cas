//! Read-only mirror of web-authored task comments (cas-7d54, cloud contract §2).
//!
//! Comments are authored in the web ticket explorer and stored server-side in a
//! dedicated `task_comments` table — NOT a `sync_entities` entity, so there is
//! no push/pull wire path and no `task_comment` `EntityType`. The CLI fetches
//! them per task via REST (`GET /api/teams/{teamId}/tasks/{taskId}/comments`)
//! for display only. There is no client write path in v1: comments are authored
//! in the web UI and are read-only here.
//!
//! Every network path is best-effort: any failure (not logged in, no team,
//! HTTP/parse error, offline) degrades to an empty list so a caller such as
//! `task show` never fails because of comments.

use serde::Deserialize;

use crate::cloud::CloudConfig;

/// One attachment on a comment. `kind` is `"image" | "video" | "link"`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct CommentAttachment {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub mime: String,
    #[serde(default)]
    pub size: u64,
}

/// A single task comment from
/// `GET /api/teams/{teamId}/tasks/{taskId}/comments`. Server-authoritative
/// fields (`id`, `author_email`, timestamps) are read-only.
#[derive(Debug, Clone, Deserialize)]
pub struct TaskComment {
    #[serde(default)]
    pub id: String,
    /// Joined from `users.email` server-side; the human-readable author.
    #[serde(default)]
    pub author_email: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub attachments: Vec<CommentAttachment>,
    #[serde(default)]
    pub created_at: String,
}

/// Response envelope for the comments endpoint: `{ "comments": [...] }`.
#[derive(Debug, Default, Deserialize)]
pub struct CommentsResponse {
    #[serde(default)]
    pub comments: Vec<TaskComment>,
}

/// Parse a raw comments JSON payload into the typed list, preserving the
/// server's ordering (created_at ASC). Pure — unit-testable without HTTP.
/// A malformed payload yields an empty list rather than an error.
pub fn parse_comments(raw: &str) -> Vec<TaskComment> {
    serde_json::from_str::<CommentsResponse>(raw)
        .map(|r| r.comments)
        .unwrap_or_default()
}

/// Fetch comments for `task_id` for display. Read-only and best-effort: any
/// failure (not logged in, no team resolved, network/HTTP/parse error) yields
/// an empty list so callers never fail because of comments. A short timeout
/// keeps `task show` responsive when the cloud is slow or unreachable.
pub fn fetch_task_comments(task_id: &str) -> Vec<TaskComment> {
    let config = match CloudConfig::load() {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let token = match config.token.as_deref() {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => return Vec::new(),
    };
    let team_id = match config.active_team_id() {
        Some(t) => t,
        None => return Vec::new(),
    };
    let url = format!(
        "{}/api/teams/{}/tasks/{}/comments",
        config.endpoint, team_id, task_id
    );
    match ureq::get(&url)
        .set("Authorization", &format!("Bearer {token}"))
        .timeout(std::time::Duration::from_secs(4))
        .call()
    {
        Ok(r) => match r.into_string() {
            Ok(body) => parse_comments(&body),
            Err(_) => Vec::new(),
        },
        Err(_) => Vec::new(),
    }
}

/// Render comments as a `task show` section. Returns an empty string when there
/// are no comments so the caller can append unconditionally. Attachments render
/// as `[kind] url` lines so links/media are clickable in a terminal.
pub fn render_comments_section(comments: &[TaskComment]) -> String {
    if comments.is_empty() {
        return String::new();
    }
    let mut out = format!("\nComments ({}):\n", comments.len());
    for c in comments {
        let who = if c.author_email.is_empty() {
            "unknown"
        } else {
            c.author_email.as_str()
        };
        let when = c.created_at.as_str();
        // Indent the body so multi-line comments stay visually grouped.
        let body = c.body.replace('\n', "\n    ");
        if when.is_empty() {
            out.push_str(&format!("\n  • {who}:\n    {body}\n"));
        } else {
            out.push_str(&format!("\n  • {who} ({when}):\n    {body}\n"));
        }
        for a in &c.attachments {
            out.push_str(&format!("    [{}] {}\n", a.kind, a.url));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
        "comments": [
            {
                "id": "c1",
                "author_email": "daniel@petrastella.io",
                "body": "First pass looks good.",
                "attachments": [],
                "created_at": "2026-06-20T10:00:00.000Z"
            },
            {
                "id": "c2",
                "author_email": "ben@petrastella.io",
                "body": "See the mock + clip + spec.",
                "attachments": [
                    { "kind": "image", "url": "https://blob.vercel/abc.png", "mime": "image/png", "size": 1234 },
                    { "kind": "video", "url": "https://blob.vercel/clip.mp4", "mime": "video/mp4", "size": 999999 },
                    { "kind": "link",  "url": "https://example.com/spec",    "mime": "text/html",  "size": 0 }
                ],
                "created_at": "2026-06-20T11:30:00.000Z"
            }
        ]
    }"#;

    #[test]
    fn parses_comments_with_each_attachment_kind() {
        let comments = parse_comments(FIXTURE);
        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].author_email, "daniel@petrastella.io");
        assert!(comments[0].attachments.is_empty());

        let atts = &comments[1].attachments;
        assert_eq!(atts.len(), 3);
        assert_eq!(atts[0].kind, "image");
        assert_eq!(atts[1].kind, "video");
        assert_eq!(atts[2].kind, "link");
        assert_eq!(atts[0].url, "https://blob.vercel/abc.png");
        assert_eq!(atts[2].size, 0);
    }

    #[test]
    fn parse_is_resilient_to_garbage_and_empty() {
        assert!(parse_comments("not json").is_empty());
        assert!(parse_comments("{}").is_empty());
        assert!(parse_comments(r#"{"comments":[]}"#).is_empty());
    }

    #[test]
    fn render_empty_is_blank() {
        assert_eq!(render_comments_section(&[]), "");
    }

    #[test]
    fn render_includes_author_body_and_attachment_links() {
        let comments = parse_comments(FIXTURE);
        let rendered = render_comments_section(&comments);
        assert!(rendered.contains("Comments (2):"));
        assert!(rendered.contains("daniel@petrastella.io"));
        assert!(rendered.contains("First pass looks good."));
        // Attachment URLs surface as clickable links with their kind.
        assert!(rendered.contains("[image] https://blob.vercel/abc.png"));
        assert!(rendered.contains("[video] https://blob.vercel/clip.mp4"));
        assert!(rendered.contains("[link] https://example.com/spec"));
    }

    #[test]
    fn render_handles_missing_author_and_timestamp() {
        let comments = vec![TaskComment {
            id: "x".to_string(),
            author_email: String::new(),
            body: "anon".to_string(),
            attachments: vec![],
            created_at: String::new(),
        }];
        let rendered = render_comments_section(&comments);
        assert!(rendered.contains("unknown:"));
        assert!(rendered.contains("anon"));
    }
}
