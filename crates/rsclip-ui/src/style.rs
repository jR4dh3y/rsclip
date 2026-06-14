use std::cell::RefCell;

use gtk::gdk;
use gtk4 as gtk;
use rsclip_core::colors::parse_color;
use rsclip_core::{AppConfig, UiColors};

thread_local! {
    static CSS_PROVIDER: RefCell<Option<gtk::CssProvider>> = const { RefCell::new(None) };
}

pub(crate) fn load_css(config: &AppConfig) -> anyhow::Result<()> {
    let css = build_css(config)?;
    CSS_PROVIDER.with(|slot| {
        let provider = if let Some(provider) = slot.borrow().as_ref().cloned() {
            provider
        } else {
            let provider = gtk::CssProvider::new();
            if let Some(display) = gdk::Display::default() {
                gtk::style_context_add_provider_for_display(
                    &display,
                    &provider,
                    gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
                );
            }
            *slot.borrow_mut() = Some(provider.clone());
            provider
        };
        provider.load_from_data(&css);
    });
    Ok(())
}

pub(crate) fn build_css(config: &AppConfig) -> anyhow::Result<String> {
    let mut css = String::new();
    for color in theme_colors() {
        let value = theme_color_value(config, color)?;
        css.push_str("@define-color ");
        css.push_str(color.name);
        css.push(' ');
        css.push_str(&value);
        css.push_str(";\n");
    }
    css.push('\n');
    css.push_str(include_str!("../resources/css/rsclip.css"));
    Ok(css)
}

fn theme_color_value(config: &AppConfig, color: &ThemeColor) -> anyhow::Result<String> {
    let value = (color.configured)(&config.ui.colors)
        .unwrap_or(color.default_value)
        .trim();

    if color.name == "shell_bg"
        && let Some(opacity) = config.ui.background_opacity
    {
        validate_opacity(opacity)?;
        return color_with_alpha("shell_bg", value, opacity);
    }

    validate_color(color.name, value)?;
    Ok(value.to_string())
}

struct ThemeColor {
    name: &'static str,
    default_value: &'static str,
    configured: fn(&UiColors) -> Option<&str>,
}

fn theme_colors() -> &'static [ThemeColor] {
    &[
        ThemeColor {
            name: "shell_bg",
            default_value: "rgba(30, 30, 32, 0.70)",
            configured: |colors| colors.shell_bg.as_deref(),
        },
        ThemeColor {
            name: "shell_border",
            default_value: "rgba(220, 217, 231, 0.14)",
            configured: |colors| colors.shell_border.as_deref(),
        },
        ThemeColor {
            name: "surface",
            default_value: "#2a2a2c",
            configured: |colors| colors.surface.as_deref(),
        },
        ThemeColor {
            name: "surface_subtle",
            default_value: "rgba(42, 42, 44, 0.54)",
            configured: |colors| colors.surface_subtle.as_deref(),
        },
        ThemeColor {
            name: "surface_overlay",
            default_value: "#1e1e20",
            configured: |colors| colors.surface_overlay.as_deref(),
        },
        ThemeColor {
            name: "preview_bg",
            default_value: "rgba(30, 30, 32, 0.40)",
            configured: |colors| colors.preview_bg.as_deref(),
        },
        ThemeColor {
            name: "preview_text_bg",
            default_value: "rgba(12, 12, 14, 0.34)",
            configured: |colors| colors.preview_text_bg.as_deref(),
        },
        ThemeColor {
            name: "scrim_bg",
            default_value: "rgba(12, 12, 14, 0.42)",
            configured: |colors| colors.scrim_bg.as_deref(),
        },
        ThemeColor {
            name: "text",
            default_value: "#dcd9e7",
            configured: |colors| colors.text.as_deref(),
        },
        ThemeColor {
            name: "text_strong",
            default_value: "#f0eafd",
            configured: |colors| colors.text_strong.as_deref(),
        },
        ThemeColor {
            name: "text_muted",
            default_value: "#9c96ad",
            configured: |colors| colors.text_muted.as_deref(),
        },
        ThemeColor {
            name: "text_selected_muted",
            default_value: "#c8bed6",
            configured: |colors| colors.text_selected_muted.as_deref(),
        },
        ThemeColor {
            name: "border",
            default_value: "#4a4653",
            configured: |colors| colors.border.as_deref(),
        },
        ThemeColor {
            name: "border_subtle",
            default_value: "rgba(220, 217, 231, 0.08)",
            configured: |colors| colors.border_subtle.as_deref(),
        },
        ThemeColor {
            name: "border_preview",
            default_value: "rgba(220, 217, 231, 0.10)",
            configured: |colors| colors.border_preview.as_deref(),
        },
        ThemeColor {
            name: "border_dialog",
            default_value: "#5c516b",
            configured: |colors| colors.border_dialog.as_deref(),
        },
        ThemeColor {
            name: "hover_bg",
            default_value: "#3b304a",
            configured: |colors| colors.hover_bg.as_deref(),
        },
        ThemeColor {
            name: "selected_bg",
            default_value: "#453657",
            configured: |colors| colors.selected_bg.as_deref(),
        },
        ThemeColor {
            name: "accent",
            default_value: "#c3fb5b",
            configured: |colors| colors.accent.as_deref(),
        },
        ThemeColor {
            name: "accent_hover",
            default_value: "#d4ff76",
            configured: |colors| colors.accent_hover.as_deref(),
        },
        ThemeColor {
            name: "accent_text",
            default_value: "#11130f",
            configured: |colors| colors.accent_text.as_deref(),
        },
        ThemeColor {
            name: "destructive",
            default_value: "#6f2d38",
            configured: |colors| colors.destructive.as_deref(),
        },
        ThemeColor {
            name: "destructive_border",
            default_value: "#9a4350",
            configured: |colors| colors.destructive_border.as_deref(),
        },
        ThemeColor {
            name: "destructive_text",
            default_value: "#fff1f3",
            configured: |colors| colors.destructive_text.as_deref(),
        },
    ]
}

fn validate_color(name: &str, value: &str) -> anyhow::Result<()> {
    let trimmed = value.trim();
    if parse_color(trimmed).is_some() {
        return Ok(());
    }

    anyhow::bail!(
        "invalid ui.colors.{name}: expected CSS color like #c3fb5b or rgba(30, 30, 32, 0.70)"
    )
}

fn validate_opacity(opacity: f32) -> anyhow::Result<()> {
    if opacity.is_finite() && (0.0..=1.0).contains(&opacity) {
        return Ok(());
    }
    anyhow::bail!("invalid ui.background_opacity: expected a number from 0.0 to 1.0")
}

fn color_with_alpha(name: &str, value: &str, alpha: f32) -> anyhow::Result<String> {
    let color = parse_color(value).ok_or_else(|| {
        anyhow::anyhow!(
            "invalid ui.colors.{name}: expected CSS color like #c3fb5b or rgba(30, 30, 32, 0.70)"
        )
    })?;
    Ok(format!(
        "rgba({}, {}, {}, {})",
        color.rgb.0,
        color.rgb.1,
        color.rgb.2,
        css_alpha(alpha)
    ))
}

fn css_alpha(alpha: f32) -> String {
    let mut text = format!("{alpha:.3}");
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_generation_includes_every_color_definition() {
        let css = build_css(&AppConfig::default()).expect("default theme CSS should build");

        for color in theme_colors() {
            assert!(css.contains(&format!("@define-color {} ", color.name)));
        }
    }

    #[test]
    fn overridden_colors_appear_in_generated_css() {
        let mut config = AppConfig::default();
        config.ui.colors.accent = Some("#ff00aa".to_string());
        config.ui.colors.accent_text = Some("#000000".to_string());

        let css = build_css(&config).expect("CSS with valid overrides should build");

        assert!(css.contains("@define-color accent #ff00aa;"));
        assert!(css.contains("@define-color accent_text #000000;"));
    }

    #[test]
    fn missing_colors_fall_back_to_defaults() {
        let css = build_css(&AppConfig::default()).expect("default theme CSS should build");

        assert!(css.contains("@define-color text #dcd9e7;"));
        assert!(css.contains("@define-color shell_bg rgba(30, 30, 32, 0.70);"));
    }

    #[test]
    fn invalid_color_returns_offending_key() {
        let mut config = AppConfig::default();
        config.ui.colors.accent = Some("not-a-color".to_string());

        let err = build_css(&config).unwrap_err();

        assert!(format!("{err:#}").contains("ui.colors.accent"));
    }

    #[test]
    fn validates_supported_color_formats() {
        for value in [
            "#abc",
            "#aabbcc",
            "#aabbccdd",
            "rgb(195, 251, 91)",
            "rgba(30, 30, 32, 0.70)",
        ] {
            validate_color("accent", value).expect("supported color format should validate");
        }
    }

    #[test]
    fn background_opacity_overrides_shell_alpha() {
        let mut config = AppConfig::default();
        config.ui.background_opacity = Some(0.42);
        config.ui.colors.shell_bg = Some("#203040".to_string());

        let css = build_css(&config).expect("CSS with background opacity should build");

        assert!(css.contains("@define-color shell_bg rgba(32, 48, 64, 0.42);"));
    }

    #[test]
    fn invalid_background_opacity_returns_offending_key() {
        let mut config = AppConfig::default();
        config.ui.background_opacity = Some(1.2);

        let err = build_css(&config).unwrap_err();

        assert!(format!("{err:#}").contains("ui.background_opacity"));
    }
}
