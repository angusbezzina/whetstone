use regex::Regex;
use serde_json::Value;

use super::content_hash;
use super::http::http_get;

/// Attempt to fetch a changelog from a GitHub repository.
/// Tries CHANGELOG.md, CHANGES.md, HISTORY.md at the repo root.
/// Returns a section Value with recency-filtered content, or None.
pub fn probe_github_changelog(repo_url: &str, timeout: u64) -> Option<Value> {
    let (owner, repo) = parse_github_repo(repo_url)?;

    let filenames = ["CHANGELOG.md", "CHANGES.md", "HISTORY.md", "RELEASES.md"];
    // Try HEAD first (default branch), then main, then master
    let branches = ["HEAD", "main", "master"];

    for branch in &branches {
        for filename in &filenames {
            let raw_url = format!(
                "https://raw.githubusercontent.com/{owner}/{repo}/{branch}/{filename}"
            );
            if let Some(content) = http_get(&raw_url, timeout) {
                // Reject HTML error pages
                if content.trim_start().starts_with('<') {
                    continue;
                }
                if content.len() < 100 {
                    continue;
                }

                let (filtered, versions_covered) = filter_recent_changelog(&content, 18);
                if filtered.len() < 50 {
                    continue;
                }

                let hash = content_hash(&filtered);
                return Some(serde_json::json!({
                    "type": "changelog",
                    "content": filtered,
                    "url": raw_url,
                    "content_hash": hash,
                    "versions_covered": versions_covered,
                }));
            }
        }
    }

    None
}

/// Parse a GitHub URL into (owner, repo).
/// Handles formats: https://github.com/owner/repo, git://github.com/owner/repo.git, etc.
pub fn parse_github_repo(url: &str) -> Option<(String, String)> {
    let cleaned = url
        .trim()
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .replace("git://", "https://")
        .replace("git+https://", "https://")
        .replace("ssh://git@github.com", "https://github.com");

    // Match github.com/owner/repo
    let re = Regex::new(r"github\.com[/:]([^/]+)/([^/]+)").ok()?;
    let caps = re.captures(&cleaned)?;
    let owner = caps.get(1)?.as_str().to_string();
    let repo = caps.get(2)?.as_str().to_string();

    if owner.is_empty() || repo.is_empty() {
        return None;
    }

    Some((owner, repo))
}

/// Filter a changelog to only include entries from the last `months` months.
/// Parses markdown headings like:
///   ## [4.5.0] - 2025-01-15
///   ## 4.5.0 (2025-01-15)
///   ## v4.5.0
///   # Version 4.5.0
/// Returns (filtered_content, versions_covered_string).
pub fn filter_recent_changelog(content: &str, months: u32) -> (String, String) {
    let cutoff = chrono::Utc::now() - chrono::Duration::days(i64::from(months) * 30);

    // Regex for version headings with dates
    let date_re =
        Regex::new(r"(?m)^#{1,3}\s+(?:\[)?v?(\d+\.\d+(?:\.\d+)?)(?:\])?.+?(\d{4}-\d{2}-\d{2})")
            .unwrap();

    // Regex for any version heading (without date)
    let version_re =
        Regex::new(r"(?m)^#{1,3}\s+(?:\[)?v?(\d+\.\d+(?:\.\d+)?)(?:\])?").unwrap();

    // Try date-based filtering first
    let mut sections: Vec<(usize, &str, Option<chrono::NaiveDate>)> = Vec::new();

    for caps in date_re.captures_iter(content) {
        let full_match = caps.get(0).unwrap();
        let version = caps.get(1).unwrap().as_str();
        let date_str = caps.get(2).unwrap().as_str();
        let date = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d").ok();
        sections.push((full_match.start(), version, date));
    }

    // If we found date-annotated sections, filter by cutoff
    if !sections.is_empty() {
        let mut filtered = String::new();
        let mut versions = Vec::new();
        let cutoff_date = cutoff.date_naive();

        for (i, (start, version, date)) in sections.iter().enumerate() {
            if let Some(d) = date {
                if *d < cutoff_date {
                    continue; // Too old
                }
            }
            // Include this section up to the next section or end of content
            let end = sections
                .get(i + 1)
                .map(|(s, _, _)| *s)
                .unwrap_or(content.len());
            filtered.push_str(&content[*start..end]);
            filtered.push('\n');
            versions.push(version.to_string());
        }

        if !filtered.is_empty() {
            let versions_str = if versions.len() > 1 {
                format!("{}–{}", versions.last().unwrap(), versions.first().unwrap())
            } else {
                versions.first().cloned().unwrap_or_default()
            };
            return (filtered.trim().to_string(), versions_str);
        }
    }

    // Fallback: no dates found, take first N version sections (most recent at top)
    let mut headings: Vec<(usize, &str)> = Vec::new();
    for caps in version_re.captures_iter(content) {
        let full_match = caps.get(0).unwrap();
        let version = caps.get(1).unwrap().as_str();
        headings.push((full_match.start(), version));
    }

    if headings.is_empty() {
        // No version headings found — return truncated content
        let truncated = if content.len() > 50_000 {
            &content[..50_000]
        } else {
            content
        };
        return (truncated.trim().to_string(), String::new());
    }

    // Take first 5 versions (assumed most recent)
    let max_sections = 5.min(headings.len());
    let end = headings
        .get(max_sections)
        .map(|(s, _)| *s)
        .unwrap_or(content.len());
    let filtered = &content[..end];
    let versions: Vec<&str> = headings[..max_sections]
        .iter()
        .map(|(_, v)| *v)
        .collect();
    let versions_str = if versions.len() > 1 {
        format!("{}–{}", versions.last().unwrap(), versions.first().unwrap())
    } else {
        versions.first().copied().unwrap_or("").to_string()
    };

    (filtered.trim().to_string(), versions_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_github_repo_https() {
        let (owner, repo) = parse_github_repo("https://github.com/clap-rs/clap").unwrap();
        assert_eq!(owner, "clap-rs");
        assert_eq!(repo, "clap");
    }

    #[test]
    fn test_parse_github_repo_git_suffix() {
        let (owner, repo) =
            parse_github_repo("https://github.com/serde-rs/serde.git").unwrap();
        assert_eq!(owner, "serde-rs");
        assert_eq!(repo, "serde");
    }

    #[test]
    fn test_parse_github_repo_git_plus() {
        let (owner, repo) =
            parse_github_repo("git+https://github.com/user/repo.git").unwrap();
        assert_eq!(owner, "user");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn test_parse_github_repo_non_github() {
        assert!(parse_github_repo("https://gitlab.com/user/repo").is_none());
    }

    #[test]
    fn test_filter_recent_changelog_with_dates() {
        let changelog = r#"# Changelog

## [2.0.0] - 2026-03-01

- Breaking change

## [1.5.0] - 2026-01-15

- New feature

## [1.0.0] - 2020-01-01

- Initial release
"#;
        let (filtered, versions) = filter_recent_changelog(changelog, 18);
        assert!(filtered.contains("2.0.0"));
        assert!(filtered.contains("1.5.0"));
        assert!(!filtered.contains("Initial release"));
        assert!(versions.contains("2.0.0"));
    }

    #[test]
    fn test_filter_recent_changelog_no_dates() {
        let changelog = r#"# Changelog

## 3.0.0

- Third

## 2.0.0

- Second

## 1.0.0

- First
"#;
        let (filtered, _versions) = filter_recent_changelog(changelog, 18);
        // Should take first 3 (all we have, under the 5 limit)
        assert!(filtered.contains("3.0.0"));
        assert!(filtered.contains("2.0.0"));
        assert!(filtered.contains("1.0.0"));
    }
}
