#![allow(dead_code)]
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
    pub(crate) fn exact(s: impl Into<String>) -> Self {
        RulePattern::Exact(s.into())
    }

    pub(crate) fn prefix(s: impl Into<String>) -> Self {
        RulePattern::Prefix(s.into())
    }

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

    fn matches(&self, s: &str) -> bool {
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

    pub(crate) target: WindowState,
    pub(crate) floating_rect: Option<Rect>,
}

#[derive(Debug, Clone)]
pub(super) struct WindowRules {
    rules: Vec<WindowRule>,
}

impl WindowRules {
    pub(super) fn new(rules: Vec<WindowRule>) -> Self {
        Self { rules }
    }

    /// Apply every rule that matches the window, in order. A later matching
    /// rule overrides an earlier one for the same property, mirroring River's
    /// `rule-add` semantics (all matching rules apply; later wins).
    ///
    /// Evaluation re-runs on each metadata event until every field referenced
    /// by any rule is known, after which the window is finalized and never
    /// re-evaluated (matching River, which applies rules once at view creation).
    pub(super) fn evaluate(
        &self,
        window: &mut super::window::Window,
        tree: &mut crate::layout::LayoutTree,
        fallback_rect: Rect,
    ) -> bool {
        if window.rules_applied {
            return false;
        }

        let app_id = window.app_id.as_deref();
        let title = window.title.as_deref();

        let before_state = tree.window_state(window.id.0);
        let before_base_state = tree.window_base_state(window.id.0);
        let before_rect = tree.window_floating_rect(window.id.0);

        let mut applied = false;
        let mut last_target = crate::layout::WindowState::Tiled;
        let mut last_rect = Rect::new(0, 0, 0, 0);
        for rule in &self.rules {
            if rule.is_pending_metadata(app_id, title) {
                continue;
            }
            if !rule.matches(app_id.unwrap_or(""), title.unwrap_or("")) {
                continue;
            }

            let rect = rule.floating_rect.unwrap_or_else(|| {
                window
                    .pseudo_tiled_rect(fallback_rect)
                    .unwrap_or_else(|| Rect::new(0, 0, 0, 0))
            });
            tree.set_window_state(window.id.0, rule.target, rect);
            last_target = rule.target;
            last_rect = rect;
            applied = true;
        }

        // Rules set the active state but must not overwrite the home state that
        // toggle commands rely on. Restore the pre-rule base_state (defaulting
        // to Tiled when the window has no recorded base state yet).
        if applied {
            let base = before_base_state.unwrap_or(crate::layout::WindowState::Tiled);
            let _ = tree.set_window_base_state(window.id.0, base);
        }

        // Finalize once no rule still awaits metadata, so a later metadata
        // event can re-run evaluation (letting a more specific, later-listed
        // rule override) until all referenced fields are present.
        let pending = self
            .rules
            .iter()
            .any(|rule| rule.is_pending_metadata(app_id, title));
        window.rules_applied = !pending;

        if !applied {
            return false;
        }

        // Sync the persistent mode so reassign / toggle paths stay consistent
        // with the tree.
        window.mode = match last_target {
            crate::layout::WindowState::Floating => super::window::WindowMode::Floating {
                x: last_rect.x,
                y: last_rect.y,
                width: last_rect.width,
                height: last_rect.height,
            },
            crate::layout::WindowState::Fullscreen => super::window::WindowMode::Fullscreen,
            crate::layout::WindowState::PseudoTiled => super::window::WindowMode::PseudoTiled,
            crate::layout::WindowState::Tiled => super::window::WindowMode::Tiled,
        };

        let after_state = tree.window_state(window.id.0);
        let after_rect = tree.window_floating_rect(window.id.0);
        after_state != before_state || after_rect != before_rect
    }
}

impl WindowRule {
    fn requires_app_id(&self) -> bool {
        self.app_id.is_some()
    }

    fn requires_title(&self) -> bool {
        self.title.is_some()
    }

    fn is_pending_metadata(&self, app_id: Option<&str>, title: Option<&str>) -> bool {
        (self.app_id.is_some() && app_id.is_none()) || (self.title.is_some() && title.is_none())
    }

    fn matches(&self, app_id: &str, title: &str) -> bool {
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
            WindowState::Floating,
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
            WindowState::Floating,
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
            WindowState::Floating,
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
            WindowState::Floating,
        );
        assert!(rule.requires_app_id());
        assert!(!rule.requires_title());

        let title_rule = test_rule(
            None,
            Some(RulePattern::exact("news")),
            WindowState::Floating,
        );
        assert!(!title_rule.requires_app_id());
        assert!(title_rule.requires_title());
    }

    #[test]
    fn evaluate_skips_finalized_window() {
        let mut window = super::super::window::Window::new(
            super::super::window::WindowId(1),
            super::super::output::OutputId(1),
        );
        window.app_id = Some("foot".to_string());
        window.title = Some("term".to_string());
        window.rules_applied = true;

        let mut tree = crate::layout::LayoutTree::new(Rect::new(0, 0, 1920, 1080));
        tree.insert_window(1);

        let rules = WindowRules::new(vec![test_rule(
            Some(RulePattern::exact("foot")),
            None,
            WindowState::Floating,
        )]);
        assert!(!rules.evaluate(&mut window, &mut tree, Rect::new(0, 0, 1920, 1080)));
    }

    #[test]
    fn evaluate_applies_all_matching_rules_later_wins() {
        let mut window = super::super::window::Window::new(
            super::super::window::WindowId(1),
            super::super::output::OutputId(1),
        );
        window.app_id = Some("firefox".to_string());
        window.title = Some("Library".to_string());

        let mut tree = crate::layout::LayoutTree::new(Rect::new(0, 0, 1920, 1080));
        tree.insert_window(1);

        // General rule listed first, specific rule listed later. Both match;
        // the later rule overrides (River semantics).
        let rules = WindowRules::new(vec![
            test_rule(
                Some(RulePattern::regex("firefox").unwrap()),
                None,
                WindowState::Tiled,
            ),
            test_rule(
                Some(RulePattern::regex("firefox").unwrap()),
                Some(RulePattern::exact("Library")),
                WindowState::Floating,
            ),
        ]);

        assert!(rules.evaluate(&mut window, &mut tree, Rect::new(0, 0, 1920, 1080)));
        assert!(window.rules_applied);
        assert_eq!(tree.window_base_state(1), Some(WindowState::Tiled));
        assert_eq!(tree.window_state(1), Some(WindowState::Floating));
    }

    #[test]
    fn evaluate_re_runs_when_title_arrives() {
        let mut window = super::super::window::Window::new(
            super::super::window::WindowId(1),
            super::super::output::OutputId(1),
        );
        window.app_id = Some("firefox".to_string());

        let mut tree = crate::layout::LayoutTree::new(Rect::new(0, 0, 1920, 1080));
        tree.insert_window(1);

        let rules = WindowRules::new(vec![
            test_rule(
                Some(RulePattern::regex("firefox").unwrap()),
                None,
                WindowState::Tiled,
            ),
            test_rule(
                Some(RulePattern::regex("firefox").unwrap()),
                Some(RulePattern::exact("Library")),
                WindowState::Floating,
            ),
        ]);

        // app_id known, title missing: general rule applies, not finalized yet.
        assert!(rules.evaluate(&mut window, &mut tree, Rect::new(0, 0, 1920, 1080)));
        assert!(!window.rules_applied);
        assert_eq!(tree.window_base_state(1), Some(WindowState::Tiled));

        // Title arrives: re-evaluation lets the later, specific rule override.
        window.title = Some("Library".to_string());
        assert!(rules.evaluate(&mut window, &mut tree, Rect::new(0, 0, 1920, 1080)));
        assert!(window.rules_applied);
        assert_eq!(tree.window_base_state(1), Some(WindowState::Tiled));
        assert_eq!(tree.window_state(1), Some(WindowState::Floating));
    }
}
