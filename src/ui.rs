use eframe::egui::{
    self, pos2, vec2, Align2, Color32, CornerRadius, FontFamily, FontId, Rect, Response, Sense,
    Stroke, TextStyle, Ui,
};
use std::sync::Arc;

const TRACK_BOLD_FAMILY: &str = "SpotiFast Track Bold";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AppIcon {
    Search,
    Library,
    Heart,
    Download,
    Queue,
    Playlist,
    Shuffle,
    Previous,
    Next,
    Repeat,
    Close,
    Music,
    Play,
    Pause,
    Drive,
    More,
    Settings,
}

#[derive(Clone, Copy)]
pub struct SpotifyPalette {
    pub bg_base: Color32,
    pub bg_sidebar: Color32,
    pub bg_surface: Color32,
    pub bg_highlight: Color32,
    pub bg_active: Color32,
    pub bg_border: Color32,
    pub green_accent: Color32,
    pub green_hover: Color32,
    pub text_bright: Color32,
    pub text_subtle: Color32,
    pub text_faint: Color32,
}

impl Default for SpotifyPalette {
    fn default() -> Self {
        Self {
            bg_base: Color32::from_rgb(18, 18, 18),
            bg_sidebar: Color32::from_rgb(0, 0, 0),
            bg_surface: Color32::from_rgb(24, 24, 24),
            bg_highlight: Color32::from_rgb(36, 36, 36),
            bg_active: Color32::from_rgb(44, 44, 44),
            bg_border: Color32::from_rgb(42, 42, 42),
            green_accent: Color32::from_rgb(30, 215, 96),
            green_hover: Color32::from_rgb(74, 231, 124),
            text_bright: Color32::WHITE,
            text_subtle: Color32::from_rgb(179, 179, 179),
            text_faint: Color32::from_rgb(114, 114, 114),
        }
    }
}

pub fn apply_spotify_theme(ctx: &egui::Context, pal: &SpotifyPalette) {
    let mut fonts = egui::FontDefinitions::default();
    let mut track_font_fallbacks = Vec::new();
    if let Ok(font_bytes) = std::fs::read(r"C:\Windows\Fonts\segoeuib.ttf") {
        fonts.font_data.insert(
            "spotifast_track_bold".to_string(),
            Arc::new(egui::FontData::from_owned(font_bytes)),
        );
        track_font_fallbacks.push("spotifast_track_bold".to_string());
    }
    // Keep the app working if this Windows font isn't present.
    track_font_fallbacks.push("Ubuntu-Light".to_string());
    fonts.families.insert(
        FontFamily::Name(TRACK_BOLD_FAMILY.into()),
        track_font_fallbacks,
    );
    ctx.set_fonts(fonts);

    let mut visuals = egui::Visuals::dark();
    visuals.override_text_color = Some(pal.text_bright);
    visuals.panel_fill = pal.bg_base;
    visuals.window_fill = pal.bg_surface;
    visuals.extreme_bg_color = pal.bg_sidebar;
    visuals.faint_bg_color = pal.bg_surface;

    let radius = CornerRadius::same(8);
    visuals.widgets.noninteractive.bg_fill = pal.bg_surface;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, pal.bg_border);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, pal.text_bright);
    visuals.widgets.noninteractive.corner_radius = radius;

    visuals.widgets.inactive.bg_fill = pal.bg_surface;
    visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, pal.text_subtle);
    visuals.widgets.inactive.corner_radius = radius;

    visuals.widgets.hovered.bg_fill = pal.bg_highlight;
    visuals.widgets.hovered.bg_stroke = Stroke::NONE;
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, pal.text_bright);
    visuals.widgets.hovered.corner_radius = radius;

    visuals.widgets.active.bg_fill = pal.bg_active;
    visuals.widgets.active.bg_stroke = Stroke::NONE;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, pal.green_accent);
    visuals.widgets.active.corner_radius = radius;

    visuals.selection.bg_fill = pal.green_accent;
    visuals.selection.stroke = Stroke::new(1.0, pal.text_bright);

    ctx.set_visuals(visuals);

    ctx.style_mut_of(egui::Theme::Dark, |s| {
        s.spacing.item_spacing = vec2(10.0, 9.0);
        s.spacing.button_padding = vec2(14.0, 7.0);
        s.spacing.window_margin = egui::Margin::same(16);
        s.spacing.scroll = egui::style::ScrollStyle::floating();
        s.text_styles
            .insert(TextStyle::Heading, FontId::proportional(22.0));
        s.text_styles
            .insert(TextStyle::Body, FontId::proportional(14.0));
        s.text_styles
            .insert(TextStyle::Button, FontId::proportional(13.5));
        s.text_styles
            .insert(TextStyle::Small, FontId::proportional(12.0));
    });
}

pub fn track_bold_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(TRACK_BOLD_FAMILY.into()))
}

pub fn brand_header(ui: &mut Ui, pal: &SpotifyPalette) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 58.0), Sense::hover());
    let center_y = rect.center().y;

    // Monochrome Spotify mark, kept crisp with native vector strokes.
    let logo_center = pos2(rect.left() + 20.0, center_y - 2.0);
    let painter = ui.painter();
    painter.circle_filled(logo_center, 15.0, Color32::WHITE);

    // Three soundwave arcs in black.
    let arc_color = Color32::BLACK;
    let c = logo_center;
    // Top arc
    painter.line_segment(
        [pos2(c.x - 7.0, c.y - 4.0), pos2(c.x - 2.0, c.y - 6.0)],
        Stroke::new(2.2, arc_color),
    );
    painter.line_segment(
        [pos2(c.x - 2.0, c.y - 6.0), pos2(c.x + 4.0, c.y - 6.0)],
        Stroke::new(2.2, arc_color),
    );
    painter.line_segment(
        [pos2(c.x + 4.0, c.y - 6.0), pos2(c.x + 8.0, c.y - 3.5)],
        Stroke::new(2.2, arc_color),
    );

    // Middle arc
    painter.line_segment(
        [pos2(c.x - 6.0, c.y - 0.2), pos2(c.x - 1.5, c.y - 1.8)],
        Stroke::new(2.0, arc_color),
    );
    painter.line_segment(
        [pos2(c.x - 1.5, c.y - 1.8), pos2(c.x + 3.5, c.y - 1.8)],
        Stroke::new(2.0, arc_color),
    );
    painter.line_segment(
        [pos2(c.x + 3.5, c.y - 1.8), pos2(c.x + 7.0, c.y + 0.5)],
        Stroke::new(2.0, arc_color),
    );

    // Bottom arc
    painter.line_segment(
        [pos2(c.x - 5.0, c.y + 3.8), pos2(c.x - 1.0, c.y + 2.5)],
        Stroke::new(1.8, arc_color),
    );
    painter.line_segment(
        [pos2(c.x - 1.0, c.y + 2.5), pos2(c.x + 3.0, c.y + 2.5)],
        Stroke::new(1.8, arc_color),
    );
    painter.line_segment(
        [pos2(c.x + 3.0, c.y + 2.5), pos2(c.x + 6.0, c.y + 4.2)],
        Stroke::new(1.8, arc_color),
    );

    // Brand title text
    painter.text(
        pos2(rect.left() + 44.0, center_y - 5.0),
        Align2::LEFT_CENTER,
        "SpotiFast",
        FontId::proportional(19.0),
        pal.text_bright,
    );
    painter.text(
        pos2(rect.left() + 44.0, center_y + 13.0),
        Align2::LEFT_CENTER,
        "YOUR MUSIC LIBRARY",
        FontId::proportional(8.5),
        pal.text_faint,
    );
}

pub fn sidebar_nav_item(
    ui: &mut Ui,
    icon: AppIcon,
    label: &str,
    badge: Option<String>,
    selected: bool,
    pal: &SpotifyPalette,
) -> Response {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 40.0), Sense::click());
    let hovered = resp.hovered();

    let bg_color = if selected {
        Color32::from_rgb(38, 38, 38)
    } else if hovered {
        pal.bg_surface
    } else {
        Color32::TRANSPARENT
    };

    if bg_color != Color32::TRANSPARENT {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(6), bg_color);
    }

    if selected {
        // Vertical active accent bar on left edge
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min, vec2(3.5, rect.height())),
            CornerRadius::same(2),
            pal.green_accent,
        );
    }

    let text_color = if selected {
        pal.text_bright
    } else if hovered {
        pal.text_bright
    } else {
        pal.text_subtle
    };

    // Draw icon
    draw_app_icon(
        ui.painter(),
        pos2(rect.left() + 24.0, rect.center().y),
        icon,
        18.0,
        if selected {
            pal.green_accent
        } else {
            text_color
        },
    );

    // Draw label
    ui.painter().text(
        pos2(rect.left() + 42.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(if selected { 14.0 } else { 13.5 }),
        text_color,
    );

    // Draw optional badge pill on right
    if let Some(b) = badge {
        let badge_rect = Rect::from_min_size(
            pos2(rect.right() - 44.0, rect.center().y - 9.0),
            vec2(36.0, 18.0),
        );
        ui.painter().rect_filled(
            badge_rect,
            CornerRadius::same(9),
            Color32::from_rgb(34, 34, 34),
        );
        ui.painter().text(
            badge_rect.center(),
            Align2::CENTER_CENTER,
            b,
            FontId::proportional(11.0),
            pal.text_subtle,
        );
    }

    resp
}

pub fn spotify_pill_button(
    ui: &mut Ui,
    text: &str,
    is_green: bool,
    pal: &SpotifyPalette,
) -> Response {
    let font = FontId::proportional(13.0);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font, Color32::WHITE);
    let w = galley.size().x + 28.0;
    let h = 32.0;

    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    let hovered = resp.hovered();

    let (fill, text_color) = if is_green {
        (
            if hovered {
                pal.green_hover
            } else {
                pal.green_accent
            },
            Color32::BLACK,
        )
    } else {
        (
            if hovered {
                pal.bg_highlight
            } else {
                pal.bg_surface
            },
            pal.text_bright,
        )
    };

    let rounding = CornerRadius::same((h / 2.0).round() as u8);
    if !is_green {
        ui.painter().rect_filled(rect, rounding, pal.bg_border);
        ui.painter().rect_filled(rect.shrink(1.0), rounding, fill);
    } else {
        ui.painter().rect_filled(rect, rounding, fill);
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(13.0),
        text_color,
    );

    resp
}

pub fn spotify_icon_pill_button(
    ui: &mut Ui,
    icon: AppIcon,
    text: &str,
    is_green: bool,
    pal: &SpotifyPalette,
) -> Response {
    let font = FontId::proportional(13.0);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font, Color32::WHITE);
    let h = 32.0;
    let w = galley.size().x + 48.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    let hovered = resp.hovered();
    let (fill, icon_color, text_color) = if is_green {
        (
            if hovered {
                pal.green_hover
            } else {
                pal.green_accent
            },
            Color32::BLACK,
            Color32::BLACK,
        )
    } else {
        (
            if hovered {
                pal.bg_highlight
            } else {
                pal.bg_surface
            },
            pal.text_bright,
            pal.text_bright,
        )
    };
    let rounding = CornerRadius::same((h / 2.0).round() as u8);
    if !is_green {
        ui.painter().rect_filled(rect, rounding, pal.bg_border);
        ui.painter().rect_filled(rect.shrink(1.0), rounding, fill);
    } else {
        ui.painter().rect_filled(rect, rounding, fill);
    }
    draw_app_icon(
        ui.painter(),
        pos2(rect.left() + 17.0, rect.center().y),
        icon,
        16.0,
        icon_color,
    );
    ui.painter().text(
        pos2(rect.left() + 31.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(13.0),
        text_color,
    );
    resp
}

pub fn icon_button(ui: &mut Ui, icon: AppIcon, size: f32, color: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    if response.hovered() {
        ui.painter()
            .circle_filled(rect.center(), size * 0.5, Color32::from_gray(38));
    }
    draw_app_icon(ui.painter(), rect.center(), icon, size * 0.58, color);
    response
}

pub fn draw_app_icon(
    painter: &egui::Painter,
    center: egui::Pos2,
    icon: AppIcon,
    size: f32,
    color: Color32,
) {
    let scale = size / 24.0;
    let p = |x: f32, y: f32| pos2(center.x + x * scale, center.y + y * scale);
    let stroke = Stroke::new((1.9 * scale).max(1.0), color);
    let line = |points: &[(f32, f32)]| {
        painter.add(egui::Shape::line(
            points.iter().map(|&(x, y)| p(x, y)).collect(),
            stroke,
        ));
    };
    let triangle = |points: [(f32, f32); 3]| {
        painter.add(egui::Shape::convex_polygon(
            points.iter().map(|&(x, y)| p(x, y)).collect(),
            color,
            Stroke::NONE,
        ));
    };

    match icon {
        AppIcon::Search => {
            painter.circle_stroke(p(-2.0, -2.0), 7.0 * scale, stroke);
            painter.line_segment([p(3.0, 3.0), p(9.0, 9.0)], stroke);
        }
        AppIcon::Library => {
            painter.rect_stroke(
                Rect::from_min_max(p(-8.0, -8.0), p(8.0, 8.0)),
                CornerRadius::same(2),
                stroke,
                egui::StrokeKind::Inside,
            );
            line(&[(-3.0, -8.0), (-3.0, 8.0)]);
            line(&[(1.0, -4.0), (5.0, -4.0)]);
            line(&[(1.0, 0.0), (5.0, 0.0)]);
            line(&[(1.0, 4.0), (5.0, 4.0)]);
        }
        AppIcon::Heart => {
            let points = vec![
                p(0.0, 9.0),
                p(-7.0, 2.0),
                p(-9.0, -1.0),
                p(-9.0, -4.0),
                p(-7.0, -7.0),
                p(-4.0, -7.0),
                p(0.0, -3.0),
                p(4.0, -7.0),
                p(7.0, -7.0),
                p(9.0, -4.0),
                p(9.0, -1.0),
                p(7.0, 2.0),
            ];
            painter.add(egui::Shape::closed_line(points, stroke));
        }
        AppIcon::Download => {
            line(&[(0.0, -8.0), (0.0, 4.0)]);
            line(&[(-4.0, 0.0), (0.0, 4.0), (4.0, 0.0)]);
            line(&[(-7.0, 5.0), (-7.0, 8.0), (7.0, 8.0), (7.0, 5.0)]);
        }
        AppIcon::Queue => {
            line(&[(-7.0, -6.0), (5.0, -6.0)]);
            line(&[(-7.0, 0.0), (5.0, 0.0)]);
            line(&[(-7.0, 6.0), (1.0, 6.0)]);
            line(&[(7.0, 3.0), (7.0, 9.0)]);
            line(&[(4.0, 6.0), (10.0, 6.0)]);
        }
        AppIcon::Playlist => {
            line(&[(-8.0, -6.0), (8.0, -6.0)]);
            line(&[(-8.0, -1.0), (8.0, -1.0)]);
            line(&[(-8.0, 4.0), (3.0, 4.0)]);
            painter.circle_filled(p(7.0, 5.0), 2.0 * scale, color);
        }
        AppIcon::Shuffle => {
            line(&[(-9.0, -6.0), (-5.0, -6.0), (4.0, 6.0), (8.0, 6.0)]);
            line(&[(-9.0, 6.0), (-5.0, 6.0), (4.0, -6.0), (8.0, -6.0)]);
            triangle([(10.0, 6.0), (6.0, 3.0), (6.0, 9.0)]);
            triangle([(10.0, -6.0), (6.0, -9.0), (6.0, -3.0)]);
        }
        AppIcon::Previous => {
            line(&[(5.0, -8.0), (5.0, 8.0)]);
            triangle([(-7.0, 0.0), (2.0, -7.0), (2.0, 7.0)]);
        }
        AppIcon::Next => {
            line(&[(-5.0, -8.0), (-5.0, 8.0)]);
            triangle([(7.0, 0.0), (-2.0, -7.0), (-2.0, 7.0)]);
        }
        AppIcon::Repeat => {
            line(&[(-7.0, -3.0), (-4.0, -6.0), (5.0, -6.0), (8.0, -3.0)]);
            triangle([(9.0, -3.0), (5.0, -6.0), (8.0, -9.0)]);
            line(&[(7.0, 3.0), (4.0, 6.0), (-5.0, 6.0), (-8.0, 3.0)]);
            triangle([(-9.0, 3.0), (-5.0, 6.0), (-8.0, 9.0)]);
        }
        AppIcon::Close => {
            line(&[(-6.0, -6.0), (6.0, 6.0)]);
            line(&[(6.0, -6.0), (-6.0, 6.0)]);
        }
        AppIcon::Music => {
            line(&[
                (2.0, -8.0),
                (2.0, 4.0),
                (7.0, 2.0),
                (7.0, -10.0),
                (2.0, -8.0),
            ]);
            painter.circle_filled(p(-2.0, 6.0), 3.5 * scale, color);
            painter.circle_filled(p(3.0, 4.0), 3.5 * scale, color);
        }
        AppIcon::Play => triangle([(-4.0, -7.0), (-4.0, 7.0), (7.0, 0.0)]),
        AppIcon::Pause => {
            painter.rect_filled(
                Rect::from_min_max(p(-6.0, -7.0), p(-2.0, 7.0)),
                CornerRadius::same(1),
                color,
            );
            painter.rect_filled(
                Rect::from_min_max(p(2.0, -7.0), p(6.0, 7.0)),
                CornerRadius::same(1),
                color,
            );
        }
        AppIcon::Drive => {
            painter.rect_stroke(
                Rect::from_min_max(p(-9.0, -6.0), p(9.0, 6.0)),
                CornerRadius::same(2),
                stroke,
                egui::StrokeKind::Inside,
            );
            line(&[(-7.0, 2.0), (7.0, 2.0)]);
            painter.circle_filled(p(5.0, 4.0), 1.0 * scale, color);
        }
        AppIcon::More => {
            for y in [-6.0, 0.0, 6.0] {
                painter.circle_filled(p(0.0, y), 1.7 * scale, color);
            }
        }
        AppIcon::Settings => {
            let gear_radius = 9.0 * scale;
            painter.circle_stroke(center, gear_radius * 0.60, stroke);
            painter.circle_stroke(center, gear_radius * 0.22, stroke);
            for i in 0..8 {
                let angle = i as f32 * std::f32::consts::TAU / 8.0;
                let direction = egui::vec2(angle.cos(), angle.sin());
                painter.line_segment(
                    [
                        center + direction * (gear_radius * 0.72),
                        center + direction * gear_radius,
                    ],
                    stroke,
                );
            }
        }
    }
}

pub fn circle_play_button(
    ui: &mut Ui,
    is_playing: bool,
    size: f32,
    pal: &SpotifyPalette,
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let hovered = resp.hovered();
    let c = rect.center();
    let fill = if hovered {
        pal.green_hover
    } else {
        pal.green_accent
    };

    ui.painter().circle_filled(c, size / 2.0, fill);

    draw_app_icon(
        ui.painter(),
        c,
        if is_playing {
            AppIcon::Pause
        } else {
            AppIcon::Play
        },
        size * 0.7,
        Color32::BLACK,
    );

    resp
}

pub fn transport_button(
    ui: &mut Ui,
    icon: AppIcon,
    active: bool,
    pal: &SpotifyPalette,
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::click());
    let hovered = resp.hovered();

    if hovered {
        ui.painter()
            .circle_filled(rect.center(), 16.0, pal.bg_highlight);
    }

    let is_toggle = matches!(icon, AppIcon::Shuffle | AppIcon::Repeat);
    let color = if hovered {
        pal.green_hover
    } else if active || !is_toggle {
        pal.green_accent
    } else {
        pal.text_bright
    };
    draw_app_icon(ui.painter(), rect.center(), icon, 20.0, color);

    resp
}

pub fn draw_player_heart(
    painter: &egui::Painter,
    center: egui::Pos2,
    filled: bool,
    pal: &SpotifyPalette,
) {
    let outline = vec![
        pos2(center.x - 8.0, center.y - 2.0),
        pos2(center.x - 8.0, center.y - 4.0),
        pos2(center.x - 6.0, center.y - 6.0),
        pos2(center.x - 3.5, center.y - 6.0),
        pos2(center.x, center.y - 3.0),
        pos2(center.x + 3.5, center.y - 6.0),
        pos2(center.x + 6.0, center.y - 6.0),
        pos2(center.x + 8.0, center.y - 4.0),
        pos2(center.x + 8.0, center.y - 2.0),
        pos2(center.x + 7.0, center.y),
        pos2(center.x + 5.0, center.y + 2.0),
        pos2(center.x, center.y + 7.0),
        pos2(center.x - 5.0, center.y + 2.0),
        pos2(center.x - 7.0, center.y),
    ];

    if filled {
        let color = pal.green_accent;
        painter.add(egui::Shape::convex_polygon(
            vec![
                pos2(center.x - 7.5, center.y - 2.0),
                pos2(center.x + 7.5, center.y - 2.0),
                pos2(center.x, center.y + 7.0),
            ],
            color,
            Stroke::NONE,
        ));
        painter.circle_filled(pos2(center.x - 3.8, center.y - 2.2), 4.2, color);
        painter.circle_filled(pos2(center.x + 3.8, center.y - 2.2), 4.2, color);
        painter.add(egui::Shape::convex_polygon(
            vec![
                pos2(center.x - 1.8, center.y - 5.5),
                pos2(center.x + 1.8, center.y - 5.5),
                pos2(center.x, center.y - 2.8),
            ],
            pal.bg_surface,
            Stroke::NONE,
        ));
    }

    painter.add(egui::Shape::closed_line(
        outline,
        Stroke::new(1.8, pal.green_accent),
    ));
}

pub fn draw_speaker_icon(painter: &egui::Painter, center: egui::Pos2, muted: bool, color: Color32) {
    painter.add(egui::Shape::convex_polygon(
        vec![
            pos2(center.x - 8.0, center.y - 3.0),
            pos2(center.x - 4.0, center.y - 3.0),
            pos2(center.x + 1.0, center.y - 7.0),
            pos2(center.x + 1.0, center.y + 7.0),
            pos2(center.x - 4.0, center.y + 3.0),
            pos2(center.x - 8.0, center.y + 3.0),
        ],
        color,
        Stroke::NONE,
    ));

    if muted {
        painter.line_segment(
            [
                pos2(center.x + 4.0, center.y - 5.0),
                pos2(center.x + 10.0, center.y + 5.0),
            ],
            Stroke::new(1.8, color),
        );
        painter.line_segment(
            [
                pos2(center.x + 10.0, center.y - 5.0),
                pos2(center.x + 4.0, center.y + 5.0),
            ],
            Stroke::new(1.8, color),
        );
    } else {
        painter.add(egui::Shape::line(
            vec![
                pos2(center.x + 4.0, center.y - 4.0),
                pos2(center.x + 6.0, center.y),
                pos2(center.x + 4.0, center.y + 4.0),
            ],
            Stroke::new(1.6, color),
        ));
        painter.add(egui::Shape::line(
            vec![
                pos2(center.x + 7.0, center.y - 6.0),
                pos2(center.x + 10.0, center.y),
                pos2(center.x + 7.0, center.y + 6.0),
            ],
            Stroke::new(1.6, color),
        ));
    }
}

pub fn seek_scrubber(
    ui: &mut Ui,
    width: f32,
    current_secs: f64,
    total_secs: f64,
    pal: &SpotifyPalette,
) -> Option<f64> {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 16.0), Sense::click_and_drag());
    let track_rect = Rect::from_center_size(rect.center(), vec2(rect.width(), 4.0));
    let frac = if total_secs > 0.0 {
        (current_secs / total_secs).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };

    let active = resp.hovered() || resp.dragged();
    let fill_color = if active {
        pal.green_accent
    } else {
        pal.text_bright
    };
    let progress_w = track_rect.width() * frac;
    let thumb_x = track_rect.left() + progress_w;

    let painter = ui.painter();
    painter.rect_filled(
        track_rect,
        CornerRadius::same(2),
        Color32::from_rgb(70, 70, 70),
    );

    if progress_w > 0.0 {
        let played_rect = Rect::from_min_max(track_rect.min, pos2(thumb_x, track_rect.max.y));
        painter.rect_filled(played_rect, CornerRadius::same(2), fill_color);
    }

    if active {
        painter.circle_filled(pos2(thumb_x, track_rect.center().y), 6.0, Color32::WHITE);
    }

    if resp.dragged() || resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos().or_else(|| resp.hover_pos()) {
            let new_frac = ((pos.x - track_rect.left()) / track_rect.width()).clamp(0.0, 1.0);
            if total_secs > 0.0 {
                return Some(new_frac as f64 * total_secs);
            }
        }
    }
    None
}

pub fn volume_slider(ui: &mut Ui, width: f32, volume: f32, pal: &SpotifyPalette) -> Option<f32> {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 16.0), Sense::click_and_drag());
    let track_rect = Rect::from_center_size(rect.center(), vec2(rect.width(), 4.0));
    let frac = (volume / 100.0).clamp(0.0, 1.0);
    let active = resp.hovered() || resp.dragged();
    let fill_color = if active {
        pal.green_accent
    } else {
        pal.text_bright
    };
    let thumb_x = track_rect.left() + track_rect.width() * frac;

    let painter = ui.painter();
    painter.rect_filled(
        track_rect,
        CornerRadius::same(2),
        Color32::from_rgb(70, 70, 70),
    );

    if frac > 0.0 {
        let played_rect = Rect::from_min_max(track_rect.min, pos2(thumb_x, track_rect.max.y));
        painter.rect_filled(played_rect, CornerRadius::same(2), fill_color);
    }

    if active {
        painter.circle_filled(pos2(thumb_x, track_rect.center().y), 5.0, Color32::WHITE);
    }

    if resp.dragged() || resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos().or_else(|| resp.hover_pos()) {
            let new_frac = ((pos.x - track_rect.left()) / track_rect.width()).clamp(0.0, 1.0);
            return Some(new_frac * 100.0);
        }
    }
    None
}

pub fn format_time(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as u64;
    let mins = s / 60;
    let rem = s % 60;
    format!("{mins}:{rem:02}")
}

pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}
