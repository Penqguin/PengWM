use crate::config::{self, BarConfig};
use crate::connection;
use crate::theme::Theme;
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, CornerRadius, FontId, Painter, Pos2, Rect, Sense, Stroke,
    StrokeKind, Vec2,
};
use pengwm_core::command::{BarMessage, BarState, Command};
use pengwm_core::config::BarPosition;
use pengwm_core::tree::SplitDirection;
use std::sync::mpsc::Receiver;

pub struct BarApp {
    rx: Receiver<BarMessage>,
    config: BarConfig,
    theme: Theme,
    corner_radius: f32,
    state: Option<BarState>,
    visible: bool,
    last_geometry: Option<(Pos2, Vec2)>,
}

impl BarApp {
    pub fn new(
        rx: Receiver<BarMessage>,
        config: BarConfig,
        theme: Theme,
        corner_radius: f32,
    ) -> Self {
        let visible = config.visible;
        Self {
            rx,
            config,
            theme,
            corner_radius,
            state: None,
            visible,
            last_geometry: None,
        }
    }

    fn drain_messages(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                BarMessage::Show => {
                    self.visible = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.request_repaint();
                }
                BarMessage::Hide => {
                    self.visible = false;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                }
                BarMessage::Exit => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                BarMessage::Reload => {
                    let config = BarConfig::load();
                    self.config = config.clone();
                    self.theme = crate::theme::resolve(&config);
                    self.corner_radius = config::resolve_corner_radius(&config);
                    self.last_geometry = None;
                    ctx.request_repaint();
                }
                BarMessage::State(s) => {
                    self.state = Some(s);
                    ctx.request_repaint();
                }
            }
        }
    }

    /// Desired bar rect in global coordinates: the daemon-reserved rect when
    /// available, else a config-derived strip across the monitor.
    fn desired_geometry(&self, ctx: &egui::Context) -> Option<(Pos2, Vec2)> {
        if let Some(rect) = self.state.as_ref().and_then(|s| s.rect) {
            return Some((
                pos2(rect.x as f32, rect.y as f32),
                vec2(rect.width as f32, rect.height as f32),
            ));
        }
        // Fallback before the daemon has pushed a rect: compute the strip
        // against the physical monitor (global origin 0,0), not the window's
        // own rect — a self-referential origin makes bottom/right positions
        // land mid-screen on a freshly-created (centered) window.
        if let Some(monitor) = ctx.input(|i| i.viewport().monitor_size) {
            if monitor.x > 0.0 && monitor.y > 0.0 {
                let rect = pengwm_core::layout::bar_strip_rect(
                    (0, 0),
                    (monitor.x.round() as u32, monitor.y.round() as u32),
                    self.config.position,
                    self.config.thickness,
                );
                return Some((
                    pos2(rect.x as f32, rect.y as f32),
                    vec2(rect.width as f32, rect.height as f32),
                ));
            }
        }
        None
    }
}

impl eframe::App for BarApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // Fully transparent: eframe's default clear (semi-transparent dark)
        // paints the whole window as a square slab, hiding the rounded fill
        // behind it. With a transparent clear the rounded corners show the
        // desktop cleanly.
        egui::Color32::TRANSPARENT.to_normalized_gamma_f32()
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_messages(ctx);

        if let Some((pos, size)) = self.desired_geometry(ctx) {
            if self.last_geometry != Some((pos, size)) {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
                self.last_geometry = Some((pos, size));
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if !self.visible {
            return;
        }
        self.paint(ui, ui.max_rect());
    }
}

impl BarApp {
    fn paint(&mut self, ui: &mut egui::Ui, bar_rect: Rect) {
        let radius = self.corner_radius.max(0.0).round() as u8;
        let rounding = CornerRadius::from(radius);

        let painter = ui.painter();
        painter.rect_filled(bar_rect, rounding, self.theme.background);
        painter.rect_stroke(
            bar_rect.shrink(0.5),
            rounding,
            Stroke::new(1.0, self.theme.border),
            StrokeKind::Inside,
        );

        let horizontal = self.is_horizontal();
        let inner = bar_rect.shrink(6.0);
        let icon_size = vec2(14.0, 14.0);
        let icon_center = if horizontal {
            pos2(inner.left() + icon_size.x / 2.0, bar_rect.center().y)
        } else {
            pos2(bar_rect.center().x, inner.top() + icon_size.y / 2.0)
        };
        let icon_rect = Rect::from_center_size(icon_center, icon_size);
        self.paint_split_icon(painter, icon_rect);

        if let Some(state) = self.state.as_ref() {
            let start = if horizontal {
                icon_rect.right() + 10.0
            } else {
                icon_rect.bottom() + 10.0
            };
            paint_workspaces(&self.theme, state, ui, horizontal, start, bar_rect);
        }
    }

    fn is_horizontal(&self) -> bool {
        matches!(self.config.position, BarPosition::Top | BarPosition::Bottom)
    }

    fn paint_split_icon(&self, painter: &Painter, rect: Rect) {
        let color = self.theme.foreground.gamma_multiply(0.85);
        let rounding = CornerRadius::from(2);
        let gap = 2.0;
        match self.state.as_ref().and_then(|s| s.split_direction) {
            Some(SplitDirection::Vertical) => {
                let w = (rect.width() - gap) / 2.0;
                painter.rect_filled(
                    Rect::from_min_size(rect.min, vec2(w, rect.height())),
                    rounding,
                    color,
                );
                painter.rect_filled(
                    Rect::from_min_size(
                        pos2(rect.min.x + w + gap, rect.min.y),
                        vec2(w, rect.height()),
                    ),
                    rounding,
                    color,
                );
            }
            Some(SplitDirection::Horizontal) => {
                let h = (rect.height() - gap) / 2.0;
                painter.rect_filled(
                    Rect::from_min_size(rect.min, vec2(rect.width(), h)),
                    rounding,
                    color,
                );
                painter.rect_filled(
                    Rect::from_min_size(
                        pos2(rect.min.x, rect.min.y + h + gap),
                        vec2(rect.width(), h),
                    ),
                    rounding,
                    color,
                );
            }
            None => {
                painter.rect_filled(rect, rounding, color);
            }
        }
    }
}

const PILL_RADIUS: u8 = 4;
const PILL_SPACING: f32 = 8.0;
const PILL_PAD_X: f32 = 8.0;

fn paint_workspaces(
    theme: &Theme,
    state: &BarState,
    ui: &mut egui::Ui,
    horizontal: bool,
    start: f32,
    bar_rect: Rect,
) {
    let font = FontId::proportional(theme.font_size);
    let pill_h = (bar_rect.height() - 12.0).max(18.0);

    let mut cursor = start;
    for (i, ws) in state.workspaces.iter().enumerate() {
        let label = ws_label(&ws.name);
        let text_w = ui
            .painter()
            .layout_no_wrap(label.clone(), font.clone(), Color32::WHITE)
            .size()
            .x;
        let pill_w = text_w + PILL_PAD_X * 2.0;

        let pill_rect = if horizontal {
            Rect::from_min_size(
                pos2(cursor, bar_rect.center().y - pill_h / 2.0),
                vec2(pill_w, pill_h),
            )
        } else {
            let w = (pill_w + 4.0).max(24.0);
            Rect::from_center_size(
                pos2(bar_rect.center().x, cursor + pill_h / 2.0),
                vec2(w, pill_h),
            )
        };

        let response = ui.interact(pill_rect, ui.id().with(("ws-pill", i)), Sense::click());

        let (fill, text_color) = if ws.active {
            (theme.accent, theme.background)
        } else if response.hovered() {
            (theme.inactive.gamma_multiply(1.4), theme.foreground)
        } else {
            (theme.inactive, theme.foreground)
        };

        let painter = ui.painter();
        painter.rect_filled(pill_rect, CornerRadius::from(PILL_RADIUS), fill);
        painter.text(
            pill_rect.center(),
            Align2::CENTER_CENTER,
            label,
            font.clone(),
            text_color,
        );

        if response.clicked() {
            let id = i as u32 + 1;
            if let Err(e) = connection::send_command(&Command::Workspace { id }) {
                log::warn!("workspace click failed: {e}");
            }
        }

        if horizontal {
            cursor = pill_rect.right() + PILL_SPACING;
        } else {
            cursor = pill_rect.bottom() + PILL_SPACING;
        }
    }
}

fn ws_label(name: &str) -> String {
    name.strip_prefix("ws-").unwrap_or(name).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pengwm_core::command::BarWorkspace;

    #[test]
    fn workspace_label_strips_prefix() {
        assert_eq!(ws_label("ws-1"), "1");
        assert_eq!(ws_label("ws-12"), "12");
        assert_eq!(ws_label("main"), "main");
    }

    #[test]
    fn desired_geometry_prefers_daemon_rect() {
        let app = BarApp {
            rx: std::sync::mpsc::channel().1,
            config: BarConfig::default(),
            theme: crate::theme::tokyo_night(),
            corner_radius: 10.0,
            state: Some(BarState {
                workspaces: vec![BarWorkspace {
                    name: "ws-1".into(),
                    monitor_id: 1,
                    window_count: 2,
                    active: true,
                    windows: vec![],
                }],
                active_workspace: 0,
                split_direction: Some(SplitDirection::Vertical),
                rect: Some(pengwm_core::layout::Rect::new(0.0, 24.0, 1920.0, 32.0)),
            }),
            visible: true,
            last_geometry: None,
        };
        let ctx = egui::Context::default();
        let (pos, size) = app.desired_geometry(&ctx).unwrap();
        assert_eq!((pos.x, pos.y), (0.0, 24.0));
        assert_eq!((size.x, size.y), (1920.0, 32.0));
    }
}
