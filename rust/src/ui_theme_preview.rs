//! Window ownership and stale-callback protection for a temporary theme choice.
pub(super) struct ThemePreview<W> {
    active: Option<(W, u64)>,
    generation: u64,
}

impl<W: Copy + Eq> ThemePreview<W> {
    pub(super) fn new() -> Self {
        Self {
            active: None,
            generation: 0,
        }
    }

    pub(super) fn begin(&mut self, window: W) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.active = Some((window, self.generation));
        self.generation
    }

    pub(super) fn is_active(&self) -> bool {
        self.active.is_some()
    }

    pub(super) fn matches(&self, generation: u64) -> bool {
        self.active.is_some_and(|(_, id)| id == generation)
    }

    pub(super) fn finish(&mut self, generation: u64) -> bool {
        if !self.matches(generation) {
            return false;
        }
        self.clear()
    }

    pub(super) fn window_closed(&mut self, window: W) -> bool {
        if !self.active.is_some_and(|(owner, _)| owner == window) {
            return false;
        }
        self.clear()
    }

    pub(super) fn clear(&mut self) -> bool {
        self.active.take().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::ThemePreview;

    #[test]
    fn closing_settings_does_not_invalidate_main_preview_or_escape() {
        let mut preview = ThemePreview::new();
        let main = preview.begin("main");
        assert!(!preview.window_closed("settings"));
        assert!(preview.matches(main));
        assert!(preview.finish(main));
        assert!(!preview.is_active());
    }

    #[test]
    fn closing_other_utilities_preserves_selection_and_confirmation() {
        let mut preview = ThemePreview::new();
        let settings = preview.begin("settings");
        for window in ["runtimes", "recommendations", "other-game"] {
            assert!(!preview.window_closed(window));
            assert!(preview.matches(settings));
        }
        assert!(preview.finish(settings));
    }

    #[test]
    fn removed_preview_owner_releases_preview_and_ignores_delayed_callback() {
        let mut preview = ThemePreview::new();
        let old = preview.begin("settings");
        assert!(preview.window_closed("settings"));
        assert!(!preview.is_active());
        let new = preview.begin("main");
        assert!(!preview.finish(old));
        assert!(preview.matches(new));
        assert!(preview.finish(new));
    }

    #[test]
    fn repeated_dismissal_cannot_cancel_a_new_preview_in_the_same_window() {
        let mut preview = ThemePreview::new();
        let first = preview.begin(1);
        assert!(preview.finish(first));
        let second = preview.begin(1);
        assert!(!preview.finish(first));
        assert!(preview.matches(second));
        assert!(preview.finish(second));
        assert!(!preview.finish(second));
    }

    #[test]
    fn missing_dialog_recovers_and_next_preview_remains_usable() {
        let mut preview = ThemePreview::new();
        let old = preview.begin(1);
        assert!(preview.clear());
        assert!(!preview.clear());
        let new = preview.begin(2);
        assert!(!preview.matches(old));
        assert!(preview.finish(new));
    }
}
