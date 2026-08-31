pub mod command;
pub mod config;
pub mod ipc;
pub mod layout;
pub mod tree;
pub mod workspace;

pub use command::{Command, DaemonResponse};
pub use config::{BarConfig, BarPosition, ColorOverrides};
pub use ipc::send_command;
pub use layout::{
    bar_strip_rect, calculate_layout, inset_rect, screen_local_to_global, window_at_point, Rect,
};
pub use tree::{Arena, Direction, Node, NodeData, NodeId, SplitDirection};
pub use workspace::Workspace;
