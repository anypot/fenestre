//! Config-layer rule types shared by all loaders.
//!
//! `RulePattern` and `WindowRule` are pure data types with no dependency on the
//! state module. The evaluation engine that applies them lives in
//! `state/rule.rs`.
use crate::layout::{Rect, WindowState};
use regex::Regex;

/// How a single `app_id`/`title` criterion matches a window string.
///
/// Mirrors the literal-vs-regex idea from other compositors: a plain value is
/// an exact match, while `Prefix`/`Regex` opt into `starts_with` / regex
/// matching (the latter lets `.*` act as a `*` wildcard, at the cost of
/// backtracking).
#[derive(Debug, Clone)]
pub(crate) enum RulePattern {
    Exact(String),
    Prefix(String),
    Regex(Regex),
}

impl RulePattern {
    /// Build an `Exact` pattern: matched with `==`.
    pub(crate) fn exact(s: impl Into<String>) -> Self {
        RulePattern::Exact(s.into())
    }

    /// Build a `Prefix` pattern: matched with `starts_with` (a `*`-style wildcard).
    pub(crate) fn prefix(s: impl Into<String>) -> Self {
        RulePattern::Prefix(s.into())
    }

    /// Build a `Regex` pattern, compiling `s` with a 1 MiB size limit.
    /// Returns `None` if the pattern fails to compile.
    pub(crate) fn regex(s: &str) -> Option<Self> {
        // size_limit bounds compile-time memory; matching still uses
        // backtracking semantics, so pathological patterns can slow runtime
        // evaluation. Prefer `Prefix` for simple wildcards.
        regex::RegexBuilder::new(s)
            .size_limit(1 << 20)
            .build()
            .ok()
            .map(RulePattern::Regex)
    }

    pub(crate) fn matches(&self, s: &str) -> bool {
        match self {
            RulePattern::Exact(p) => p == s,
            RulePattern::Prefix(p) => s.starts_with(p),
            RulePattern::Regex(r) => r.is_match(s),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct WindowRule {
    /// Criterion matched against the window's `app_id`. `None` means "any".
    pub(crate) app_id: Option<RulePattern>,
    /// Criterion matched against the window's `title`. `None` means "any".
    pub(crate) title: Option<RulePattern>,

    /// Desired window state (`tiled` / `floating` / `pseudo_tiled` / `fullscreen`).
    pub(crate) target: WindowState,
    /// Optional explicit rect for `floating` / `pseudo_tiled` targets; when
    /// `None`, a ratio-based size is used (see `WindowRules::evaluate`).
    pub(crate) floating_rect: Option<Rect>,
}

impl WindowRule {
    pub(crate) fn is_pending_metadata(&self, app_id: Option<&str>, title: Option<&str>) -> bool {
        (self.app_id.is_some() && app_id.is_none()) || (self.title.is_some() && title.is_none())
    }

    pub(crate) fn matches(&self, app_id: &str, title: &str) -> bool {
        if !field_matches(&self.app_id, app_id) {
            return false;
        }
        if !field_matches(&self.title, title) {
            return false;
        }
        true
    }
}

/// A rule field matches when it has no pattern (wildcard) or its pattern matches.
fn field_matches(pattern: &Option<RulePattern>, input: &str) -> bool {
    match pattern {
        Some(p) => p.matches(input),
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_rule(
        app_id: Option<RulePattern>,
        title: Option<RulePattern>,
        target: WindowState,
    ) -> WindowRule {
        WindowRule {
            app_id,
            title,
            target,
            floating_rect: None,
        }
    }

    #[test]
    fn rule_matches_exact_app_id() {
        let rule = test_rule(
            Some(RulePattern::exact("foot")),
            None,
            WindowState::Floating {
                rect: Rect::new(0, 0, 0, 0),
            },
        );
        assert!(rule.matches("foot", "anything"));
        assert!(!rule.matches("football", "anything"));
        assert!(!rule.matches("kitty", "anything"));
    }

    #[test]
    fn rule_matches_prefix_app_id() {
        let rule = test_rule(
            Some(RulePattern::prefix("mate-")),
            None,
            WindowState::Floating {
                rect: Rect::new(0, 0, 0, 0),
            },
        );
        assert!(rule.matches("mate-calc", "anything"));
        assert!(rule.matches("mate-dictionary", "anything"));
        assert!(!rule.matches("gnome-calculator", "anything"));
    }

    #[test]
    fn rule_matches_regex_app_id() {
        // Regex is a substring match unless anchored; "foot" matches any
        // app_id containing "foot".
        let rule = test_rule(
            Some(RulePattern::regex("foot").unwrap()),
            None,
            WindowState::Floating {
                rect: Rect::new(0, 0, 0, 0),
            },
        );
        assert!(rule.matches("foot", "anything"));
        assert!(rule.matches("football", "anything"));
        assert!(!rule.matches("kitty", "anything"));
    }

    #[test]
    fn rule_matches_title_regex() {
        let rule = test_rule(
            None,
            Some(RulePattern::regex("news").unwrap()),
            WindowState::Tiled,
        );
        assert!(rule.matches("", "news"));
        assert!(!rule.matches("", "other"));
    }

    #[test]
    fn rule_requires_metadata() {
        let rule = test_rule(
            Some(RulePattern::exact("foot")),
            None,
            WindowState::Floating {
                rect: Rect::new(0, 0, 0, 0),
            },
        );
        assert!(rule.app_id.is_some());
        assert!(rule.title.is_none());

        let title_rule = test_rule(
            None,
            Some(RulePattern::exact("news")),
            WindowState::Floating {
                rect: Rect::new(0, 0, 0, 0),
            },
        );
        assert!(title_rule.app_id.is_none());
        assert!(title_rule.title.is_some());
    }
}
