//! Dark-first visual theme and semantic colors.

use egui::{Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Theme, Visuals};

use crate::core::config::ThemePreference;
use crate::core::health::Severity;

/// Semantic colors, resolved for the active theme.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub accent: Color32,
    pub cpu: Color32,
    pub memory: Color32,
    pub disk_read: Color32,
    pub disk_write: Color32,
    pub net_rx: Color32,
    pub net_tx: Color32,
    pub ok: Color32,
    pub warning: Color32,
    pub critical: Color32,
    pub muted: Color32,
    pub border: Color32,
    pub surface: Color32,
}

impl Palette {
    pub fn for_visuals(visuals: &Visuals) -> Self {
        if visuals.dark_mode { DARK } else { LIGHT }
    }

    pub fn of(ui: &egui::Ui) -> Self {
        Self::for_visuals(ui.visuals())
    }

    pub fn severity(&self, severity: Severity) -> Color32 {
        match severity {
            Severity::Ok => self.ok,
            Severity::Warning => self.warning,
            Severity::Critical => self.critical,
        }
    }
}

const DARK: Palette = Palette {
    accent: Color32::from_rgb(0x4c, 0x9a, 0xff),
    cpu: Color32::from_rgb(0x4c, 0x9a, 0xff),
    memory: Color32::from_rgb(0xb0, 0x7c, 0xff),
    disk_read: Color32::from_rgb(0x3f, 0xc1, 0xa5),
    disk_write: Color32::from_rgb(0xe8, 0x7d, 0x5a),
    net_rx: Color32::from_rgb(0xf2, 0xa5, 0x41),
    net_tx: Color32::from_rgb(0x5a, 0xc8, 0xe8),
    ok: Color32::from_rgb(0x3f, 0xb9, 0x50),
    warning: Color32::from_rgb(0xe5, 0xb4, 0x54),
    critical: Color32::from_rgb(0xe5, 0x53, 0x4b),
    muted: Color32::from_rgb(0x8b, 0x93, 0x9e),
    border: Color32::from_rgb(0x2a, 0x2f, 0x36),
    surface: Color32::from_rgb(0x17, 0x1a, 0x1f),
};

const LIGHT: Palette = Palette {
    accent: Color32::from_rgb(0x1f, 0x6f, 0xeb),
    cpu: Color32::from_rgb(0x1f, 0x6f, 0xeb),
    memory: Color32::from_rgb(0x82, 0x50, 0xdf),
    disk_read: Color32::from_rgb(0x1a, 0x7f, 0x64),
    disk_write: Color32::from_rgb(0xc4, 0x4e, 0x24),
    net_rx: Color32::from_rgb(0xb3, 0x6b, 0x00),
    net_tx: Color32::from_rgb(0x08, 0x7e, 0xa4),
    ok: Color32::from_rgb(0x1a, 0x7f, 0x37),
    warning: Color32::from_rgb(0x9a, 0x67, 0x00),
    critical: Color32::from_rgb(0xcf, 0x22, 0x2e),
    muted: Color32::from_rgb(0x59, 0x63, 0x6e),
    border: Color32::from_rgb(0xd0, 0xd7, 0xde),
    surface: Color32::from_rgb(0xf6, 0xf8, 0xfa),
};

fn dark_visuals() -> Visuals {
    let mut v = Visuals::dark();
    v.panel_fill = Color32::from_rgb(0x11, 0x13, 0x17);
    v.window_fill = Color32::from_rgb(0x17, 0x1a, 0x1f);
    v.extreme_bg_color = Color32::from_rgb(0x0b, 0x0d, 0x10);
    v.faint_bg_color = Color32::from_rgb(0x16, 0x19, 0x1d);
    v.window_stroke = Stroke::new(1.0, DARK.border);
    v.selection.bg_fill = Color32::from_rgb(0x1d, 0x3b, 0x66);
    v.selection.stroke = Stroke::new(1.0, Color32::from_rgb(0xc9, 0xdd, 0xff));
    v.hyperlink_color = DARK.accent;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, DARK.border);
    v
}

fn light_visuals() -> Visuals {
    let mut v = Visuals::light();
    v.panel_fill = Color32::from_rgb(0xff, 0xff, 0xff);
    v.faint_bg_color = Color32::from_rgb(0xf6, 0xf8, 0xfa);
    v.window_stroke = Stroke::new(1.0, LIGHT.border);
    v.selection.bg_fill = Color32::from_rgb(0xdd, 0xe9, 0xfb);
    v.selection.stroke = Stroke::new(1.0, Color32::from_rgb(0x0a, 0x30, 0x69));
    v.hyperlink_color = LIGHT.accent;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LIGHT.border);
    v
}

fn tune(style: &mut egui::Style) {
    let radius = CornerRadius::same(3);
    style.visuals.window_corner_radius = radius;
    style.visuals.menu_corner_radius = radius;
    for w in [
        &mut style.visuals.widgets.noninteractive,
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        w.corner_radius = radius;
    }
    style.visuals.striped = true;

    style.spacing.item_spacing = egui::vec2(8.0, 5.0);
    style.spacing.button_padding = egui::vec2(8.0, 3.0);
    style.spacing.interact_size.y = 22.0;

    style.text_styles = [
        (TextStyle::Heading, FontId::new(17.0, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(13.0, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(13.0, FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(11.0, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace)),
    ]
    .into();
    // Short transitions keep panels and hover states feeling responsive.
    style.animation_time = 0.08;
}

/// Installs ProcX styles for both themes. Call once at start-up.
pub fn install(ctx: &egui::Context) {
    ctx.set_visuals_of(Theme::Dark, dark_visuals());
    ctx.set_visuals_of(Theme::Light, light_visuals());
    ctx.all_styles_mut(tune);
}

/// Switches the active theme.
pub fn apply(ctx: &egui::Context, preference: ThemePreference) {
    ctx.set_theme(match preference {
        ThemePreference::Dark => Theme::Dark,
        ThemePreference::Light => Theme::Light,
    });
}
