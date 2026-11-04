use crate::workspace::Workspace;

pub(super) fn make_workspace() -> Workspace {
    Workspace::new("test".into(), 1, (0, 0), (1920, 1080))
}
