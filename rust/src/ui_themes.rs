//! Embedded GPUI Kit themes. Selection never depends on a working-directory file.
use anyhow::{Context as _, Result};
use gpui::{
    App,
    component::{Theme, ThemeConfig, ThemeMode, ThemeRegistry},
    px,
};
use std::rc::Rc;

const FONT_FAMILY: &str = "Microsoft YaHei UI";

/// A preview is an ephemeral choice; only an explicit confirmation persists it.
#[derive(Clone, Debug, PartialEq)]
pub enum Choice {
    System,
    Named { mode: ThemeMode, name: String },
}

impl Choice {
    pub fn apply(&self, saved: &serde_json::Value, system_dark: bool, cx: &mut App) {
        let (mode, light, dark) = self.selection(saved, system_dark);
        apply(mode, &light, &dark, cx);
    }

    fn selection(
        &self,
        saved: &serde_json::Value,
        system_dark: bool,
    ) -> (ThemeMode, String, String) {
        let mut light = saved["ui_theme_light"]
            .as_str()
            .unwrap_or("Default Light")
            .to_owned();
        let mut dark = saved["ui_theme_dark"]
            .as_str()
            .unwrap_or("Default Dark")
            .to_owned();
        let mode = match self {
            Self::System => {
                if system_dark {
                    ThemeMode::Dark
                } else {
                    ThemeMode::Light
                }
            }
            Self::Named { mode, name } => {
                if mode.is_dark() {
                    dark = name.clone();
                } else {
                    light = name.clone();
                }
                *mode
            }
        };
        (mode, light, dark)
    }

    pub fn persist(&self, saved: &mut serde_json::Value) {
        saved["theme_schema"] = serde_json::json!(1);
        match self {
            Self::System => saved["theme"] = serde_json::json!("跟随系统"),
            Self::Named { mode, name } => {
                saved["theme"] =
                    serde_json::json!(if mode.is_dark() { "深色" } else { "浅色" });
                saved[if mode.is_dark() {
                    "ui_theme_dark"
                } else {
                    "ui_theme_light"
                }] = serde_json::json!(name);
            }
        }
    }
}

pub fn choices(cx: &App) -> Vec<Choice> {
    std::iter::once(Choice::System)
        .chain(
            [ThemeMode::Light, ThemeMode::Dark]
                .into_iter()
                .flat_map(|mode| {
                    names(mode, cx)
                        .into_iter()
                        .map(move |name| Choice::Named { mode, name })
                }),
        )
        .collect()
}
const THEME_SETS: &[(&str, &str)] = &[
    ("adventure", include_str!("../assets/themes/adventure.json")),
    ("alduin", include_str!("../assets/themes/alduin.json")),
    ("asciinema", include_str!("../assets/themes/asciinema.json")),
    ("aurora", include_str!("../assets/themes/aurora.json")),
    ("ayu", include_str!("../assets/themes/ayu.json")),
    (
        "catppuccin",
        include_str!("../assets/themes/catppuccin.json"),
    ),
    (
        "everforest",
        include_str!("../assets/themes/everforest.json"),
    ),
    (
        "fahrenheit",
        include_str!("../assets/themes/fahrenheit.json"),
    ),
    ("flexoki", include_str!("../assets/themes/flexoki.json")),
    ("gruvbox", include_str!("../assets/themes/gruvbox.json")),
    ("harper", include_str!("../assets/themes/harper.json")),
    ("hybrid", include_str!("../assets/themes/hybrid.json")),
    (
        "jellybeans",
        include_str!("../assets/themes/jellybeans.json"),
    ),
    ("kibble", include_str!("../assets/themes/kibble.json")),
    (
        "macos-classic",
        include_str!("../assets/themes/macos-classic.json"),
    ),
    (
        "mellifluous",
        include_str!("../assets/themes/mellifluous.json"),
    ),
    ("molokai", include_str!("../assets/themes/molokai.json")),
    ("solarized", include_str!("../assets/themes/solarized.json")),
    ("spaceduck", include_str!("../assets/themes/spaceduck.json")),
    (
        "tokyonight",
        include_str!("../assets/themes/tokyonight.json"),
    ),
    ("twilight", include_str!("../assets/themes/twilight.json")),
];

/// Call once after `gpui::init`; the Kit has already registered its two defaults.
pub fn register(cx: &mut App) -> Result<()> {
    let registry = ThemeRegistry::global_mut(cx);
    for (name, json) in THEME_SETS {
        registry
            .load_themes_from_str(json)
            .with_context(|| format!("Invalid embedded GPUI Kit theme: {name}"))?;
    }
    Ok(())
}

/// Only variants matching this mode, with the Kit default first, then by name.
pub fn names(mode: ThemeMode, cx: &App) -> Vec<String> {
    ordered_names(
        mode,
        ThemeRegistry::global(cx).themes().values().map(Rc::as_ref),
    )
}

fn ordered_names<'a>(
    mode: ThemeMode,
    themes: impl Iterator<Item = &'a ThemeConfig>,
) -> Vec<String> {
    let mut themes = themes
        .filter(|theme| theme.mode == mode)
        .collect::<Vec<_>>();
    themes.sort_by(|a, b| {
        b.is_default
            .cmp(&a.is_default)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    themes.iter().map(|theme| theme.name.to_string()).collect()
}

fn resolve<'a>(
    mode: ThemeMode,
    name: &str,
    mut themes: impl Iterator<Item = &'a Rc<ThemeConfig>>,
    fallback: &Rc<ThemeConfig>,
) -> Rc<ThemeConfig> {
    themes
        .find(|theme| theme.mode == mode && theme.name.as_ref() == name)
        .cloned()
        .unwrap_or_else(|| fallback.clone())
}

/// Restore both independent selections and apply the active mode.
/// Unknown or opposite-mode names resolve to the corresponding Kit default.
/// An unchanged selection performs no global write or window refresh. Typography
/// is also repaired on first use, preserving the manager's existing sizing.
pub fn apply(mode: ThemeMode, light_name: &str, dark_name: &str, cx: &mut App) -> bool {
    let registry = ThemeRegistry::global(cx);
    let light = resolve(
        ThemeMode::Light,
        light_name,
        registry.themes().values(),
        registry.default_light_theme(),
    );
    let dark = resolve(
        ThemeMode::Dark,
        dark_name,
        registry.themes().values(),
        registry.default_dark_theme(),
    );
    let fallback = if mode.is_dark() {
        registry.default_dark_theme().clone()
    } else {
        registry.default_light_theme().clone()
    };
    let current = Theme::global(cx);
    if current.mode == mode
        && Rc::ptr_eq(&current.light_theme, &light)
        && Rc::ptr_eq(&current.dark_theme, &dark)
        && current.font_family.as_ref() == FONT_FAMILY
        && current.font_size == px(14.)
    {
        return false;
    }

    Theme::update(cx, |theme| {
        theme.light_theme = light.clone();
        theme.dark_theme = dark.clone();
        apply_config_with_defaults(
            theme,
            if mode.is_dark() { &dark } else { &light },
            &fallback,
        );
    });
    true
}

fn apply_config_with_defaults(
    theme: &mut Theme,
    selected: &Rc<ThemeConfig>,
    fallback: &Rc<ThemeConfig>,
) {
    // Kit's apply_config patches optional fields rather than resetting them.
    // Start those fields from Kit defaults plus this mode's default config, so
    // a theme that omits shadows, radii, fonts or highlighting cannot inherit
    // them from the previously selected theme. Keep interaction settings such
    // as scrollbar behavior, focus rings and motion policy on the live theme.
    let mut defaults = Theme::default();
    defaults.apply_config(fallback);
    theme.font_family = defaults.font_family;
    theme.font_size = defaults.font_size;
    theme.mono_font_family = defaults.mono_font_family;
    theme.mono_font_size = defaults.mono_font_size;
    theme.radius = defaults.radius;
    theme.radius_lg = defaults.radius_lg;
    theme.shadow = defaults.shadow;
    theme.highlight_theme = defaults.highlight_theme;
    theme.apply_config(selected);
    theme.font_family = FONT_FAMILY.into();
    theme.font_size = px(14.);
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::component::ThemeSet;
    use std::collections::HashSet;

    #[test]
    fn preview_preserves_preferences_and_commit_changes_only_selected_mode() {
        let mut saved = serde_json::json!({"theme":"跟随系统", "ui_theme_light":"Ayu Light", "ui_theme_dark":"Ayu Dark", "language":"ja"});
        let original = saved.clone();
        let choice = Choice::Named {
            mode: ThemeMode::Dark,
            name: "Tokyo Night".into(),
        };
        assert_eq!(
            choice.selection(&saved, false),
            (ThemeMode::Dark, "Ayu Light".into(), "Tokyo Night".into())
        );
        assert_eq!(saved, original);
        choice.persist(&mut saved);
        assert_eq!(saved["theme"], "深色");
        assert_eq!(saved["ui_theme_light"], "Ayu Light");
        assert_eq!(saved["ui_theme_dark"], "Tokyo Night");
        assert_eq!(saved["language"], "ja");
        Choice::System.persist(&mut saved);
        assert_eq!(saved["theme"], "跟随系统");
        assert_eq!(
            Choice::System.selection(&saved, false),
            (ThemeMode::Light, "Ayu Light".into(), "Tokyo Night".into())
        );
        assert_eq!(Choice::System.selection(&saved, true).0, ThemeMode::Dark);
    }

    fn built_in_themes() -> Vec<Rc<ThemeConfig>> {
        THEME_SETS
            .iter()
            .flat_map(|(name, json)| {
                serde_json::from_str::<ThemeSet>(json)
                    .unwrap_or_else(|error| panic!("{name}: {error}"))
                    .themes
            })
            .map(Rc::new)
            .collect()
    }

    fn default_theme(mode: ThemeMode) -> Rc<ThemeConfig> {
        Rc::new(ThemeConfig {
            name: if mode.is_dark() {
                "Default Dark".into()
            } else {
                "Default Light".into()
            },
            mode,
            is_default: true,
            ..Default::default()
        })
    }

    #[test]
    fn embedded_theme_sets_parse_without_duplicate_or_reserved_names() {
        assert_eq!(THEME_SETS.len(), 21);
        let themes = built_in_themes();
        assert_eq!(themes.len(), 36);
        assert_eq!(themes.iter().filter(|t| !t.mode.is_dark()).count(), 11);
        assert_eq!(themes.iter().filter(|t| t.mode.is_dark()).count(), 25);
        let mut names = HashSet::from(["Default Light", "Default Dark"]);
        for theme in &themes {
            assert!(!theme.name.trim().is_empty());
            assert!(
                !theme.is_default,
                "Bundled themes must not replace defaults"
            );
            assert!(names.insert(theme.name.as_ref()), "Duplicate theme name");
            // A Kit registry reload must preserve the manager's font override.
            assert!(theme.font_family.is_none());
            assert!(theme.font_size.is_none());
        }
        let mut registry = ThemeRegistry::default();
        for (_, json) in THEME_SETS {
            registry.load_themes_from_str(json).unwrap();
        }
        assert_eq!(registry.themes().len(), themes.len());
    }

    #[test]
    fn mode_lists_keep_default_first_and_sort_only_matching_variants() {
        let mut themes = built_in_themes();
        themes.push(default_theme(ThemeMode::Light));
        themes.push(default_theme(ThemeMode::Dark));
        for (mode, count, default) in [
            (ThemeMode::Light, 12, "Default Light"),
            (ThemeMode::Dark, 26, "Default Dark"),
        ] {
            let names = ordered_names(mode, themes.iter().map(Rc::as_ref));
            assert_eq!(names.len(), count);
            assert_eq!(names[0], default);
            assert!(
                names[1..]
                    .windows(2)
                    .all(|pair| { pair[0].to_lowercase() <= pair[1].to_lowercase() })
            );
            for name in names {
                assert!(
                    themes
                        .iter()
                        .any(|t| t.mode == mode && t.name.as_ref() == name)
                );
            }
        }
    }

    #[test]
    fn unknown_or_opposite_mode_names_fall_back_without_affecting_other_mode() {
        let themes = built_in_themes();
        let light = default_theme(ThemeMode::Light);
        let dark = default_theme(ThemeMode::Dark);
        for name in ["", "missing-theme", "Ayu Dark", "Default Dark"] {
            assert!(Rc::ptr_eq(
                &resolve(ThemeMode::Light, name, themes.iter(), &light),
                &light,
            ));
        }
        for name in ["", "missing-theme", "Aurora Light", "Default Light"] {
            assert!(Rc::ptr_eq(
                &resolve(ThemeMode::Dark, name, themes.iter(), &dark),
                &dark,
            ));
        }
        assert_eq!(
            resolve(ThemeMode::Light, "Ayu Light", themes.iter(), &light).name,
            "Ayu Light",
        );
        assert_eq!(
            resolve(ThemeMode::Dark, "Ayu Dark", themes.iter(), &dark).name,
            "Ayu Dark",
        );
    }

    #[test]
    fn switching_from_hybrid_to_default_restores_kit_shadows() {
        let themes = built_in_themes();
        let fallback = default_theme(ThemeMode::Light);
        let hybrid = resolve(ThemeMode::Light, "Hybrid Light", themes.iter(), &fallback);
        assert_eq!(hybrid.shadow, Some(false));
        let mut theme = Theme::default();
        apply_config_with_defaults(&mut theme, &hybrid, &fallback);
        assert!(!theme.shadow);
        apply_config_with_defaults(&mut theme, &fallback, &fallback);
        assert_eq!(theme.shadow, Theme::default().shadow);
        assert_eq!(theme.font_family, FONT_FAMILY);
        assert_eq!(theme.font_size, px(14.));
    }

    #[test]
    fn omitted_config_fields_restore_mode_defaults_without_resetting_interactions() {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let fallback = Rc::new(ThemeConfig {
                highlight: Some(Default::default()),
                ..default_theme(mode).as_ref().clone()
            });
            let custom = Rc::new(ThemeConfig {
                name: "Custom optional fields".into(),
                mode,
                font_family: Some("Temporary UI font".into()),
                font_size: Some(22.),
                mono_font_family: Some("Temporary mono font".into()),
                mono_font_size: Some(19.),
                radius: Some(1),
                radius_lg: Some(2),
                shadow: Some(false),
                highlight: Some(Default::default()),
                ..Default::default()
            });
            let mut theme = Theme {
                focus_ring: false,
                ..Default::default()
            };
            apply_config_with_defaults(&mut theme, &custom, &fallback);
            assert_eq!(theme.mono_font_family, "Temporary mono font");
            assert_eq!(theme.radius, px(1.));
            assert_eq!(theme.highlight_theme.name, custom.name.as_ref());

            apply_config_with_defaults(&mut theme, &fallback, &fallback);
            let mut expected = Theme::default();
            expected.apply_config(&fallback);
            assert_eq!(theme.mode, mode);
            assert_eq!(theme.shadow, expected.shadow);
            assert_eq!(theme.radius, expected.radius);
            assert_eq!(theme.radius_lg, expected.radius_lg);
            assert_eq!(theme.mono_font_family, expected.mono_font_family);
            assert_eq!(theme.mono_font_size, expected.mono_font_size);
            assert_eq!(theme.highlight_theme.name, fallback.name.as_ref());
            assert_eq!(theme.highlight_theme.appearance, mode);
            assert_eq!(theme.font_family, FONT_FAMILY);
            assert_eq!(theme.font_size, px(14.));
            assert!(!theme.focus_ring);
        }
    }
}
