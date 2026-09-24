use crate::layout::Rect;
use crate::workspace::Workspace;

pub(super) fn make_workspace() -> Workspace {
    Workspace::new("test".into(), 1, (0, 0), (1920, 1080))
}

pub(super) fn full_monitor_rect(ws: &Workspace) -> Rect {
    let (w, h) = ws.monitor_size();
    Rect::new(0.0, 0.0, w as f64, h as f64)
}
