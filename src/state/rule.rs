use crate::config::WindowRule;
use crate::layout::{Rect, WindowState};

#[derive(Debug, Clone)]
pub(super) struct WindowRules {
    rules: Vec<WindowRule>,
}

impl WindowRules {
    /// Build a `WindowRules` matcher from an ordered list of rules.
    ///
    /// Rules are evaluated in order; a later matching rule overrides an earlier
    /// one for the same property (later wins).
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
        ratio: f32,
    ) -> bool {
        if window.rules_applied {
            return false;
        }

        let app_id = window.app_id.as_deref();
        let title = window.title.as_deref();

        let before_state = tree.window_state(window.id.0).cloned();

        let mut applied = false;
        for rule in &self.rules {
            if rule.is_pending_metadata(app_id, title) {
                continue;
            }
            if !rule.matches(app_id.unwrap_or(""), title.unwrap_or("")) {
                continue;
            }

            let rect = rule
                .floating_rect
                .unwrap_or_else(|| window.pseudo_tiled_rect(fallback_rect, ratio));
            let target = match rule.target.clone() {
                WindowState::Floating { .. } => WindowState::Floating { rect },
                WindowState::PseudoTiled { .. } => WindowState::PseudoTiled { rect },
                other => other,
            };
            tree.set_window_state(window.id.0, target);
            applied = true;
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

        // Preserve the pre-rule state in Fullscreen's restore field so toggling
        // fullscreen off returns to the window's original state, not hardcoded Tiled.
        if let Some(mut state) = tree.window_state(window.id.0).cloned()
            && let WindowState::Fullscreen { ref mut restore } = state
        {
            // Only overwrite `restore` when the pre-rule state was not itself
            // fullscreen, so we never nest a `Fullscreen` inside `restore`
            // (which would require two toggles to actually exit).
            if !matches!(before_state, Some(WindowState::Fullscreen { .. })) {
                let restore_state = before_state.clone().unwrap_or(WindowState::Tiled);
                **restore = restore_state;
            }
            tree.set_window_state(window.id.0, state);
        }

        let after_state = tree.window_state(window.id.0).cloned();
        after_state != before_state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RulePattern;

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
    fn evaluate_skips_finalized_window() {
        let mut window = super::super::window::Window::new(
            super::super::window::WindowId(1),
            super::super::output::OutputId(1),
        );
        window.app_id = Some("foot".to_string());
        window.title = Some("term".to_string());
        window.rules_applied = true;

        let mut tree = crate::layout::LayoutTree::new(Rect::new(0, 0, 1920, 1080));
        tree.insert_window(1, None);

        let rules = WindowRules::new(vec![test_rule(
            Some(RulePattern::exact("foot")),
            None,
            WindowState::Floating {
                rect: Rect::new(0, 0, 0, 0),
            },
        )]);
        assert!(!rules.evaluate(&mut window, &mut tree, Rect::new(0, 0, 1920, 1080), 0.5));
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
        tree.insert_window(1, None);

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
                WindowState::Floating {
                    rect: Rect::new(0, 0, 0, 0),
                },
            ),
        ]);

        assert!(rules.evaluate(&mut window, &mut tree, Rect::new(0, 0, 1920, 1080), 0.5));
        assert!(window.rules_applied);
        assert_eq!(
            tree.window_state(1).cloned(),
            Some(WindowState::Floating {
                rect: Rect::new(480, 270, 960, 540)
            })
        );
    }

    #[test]
    fn evaluate_re_runs_when_title_arrives() {
        let mut window = super::super::window::Window::new(
            super::super::window::WindowId(1),
            super::super::output::OutputId(1),
        );
        window.app_id = Some("firefox".to_string());

        let mut tree = crate::layout::LayoutTree::new(Rect::new(0, 0, 1920, 1080));
        tree.insert_window(1, None);

        let rules = WindowRules::new(vec![
            test_rule(
                Some(RulePattern::regex("firefox").unwrap()),
                None,
                WindowState::Tiled,
            ),
            test_rule(
                Some(RulePattern::regex("firefox").unwrap()),
                Some(RulePattern::exact("Library")),
                WindowState::Floating {
                    rect: Rect::new(0, 0, 0, 0),
                },
            ),
        ]);

        // app_id known, title missing: general rule applies, not finalized yet.
        // The window starts as Tiled and the general rule sets Tiled, so the
        // state does not change and evaluate returns false.
        assert!(!rules.evaluate(&mut window, &mut tree, Rect::new(0, 0, 1920, 1080), 0.5));
        assert!(!window.rules_applied);
        assert_eq!(tree.window_state(1).cloned(), Some(WindowState::Tiled));

        // Title arrives: re-evaluation lets the later, specific rule override.
        window.title = Some("Library".to_string());
        assert!(rules.evaluate(&mut window, &mut tree, Rect::new(0, 0, 1920, 1080), 0.5));
        assert!(window.rules_applied);
        assert_eq!(
            tree.window_state(1).cloned(),
            Some(WindowState::Floating {
                rect: Rect::new(480, 270, 960, 540)
            })
        );
    }
}
