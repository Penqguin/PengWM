use tokio::sync::mpsc;
use crate::state::StateManager;

pub enum DaemonEvent {
    WindowCreated(pengwm_core::tree::WindowId),
    WindowDestroyed(pengwm_core::tree::WindowId),
    WindowFocused(pengwm_core::tree::WindowId),

    AppLaunched(i32),
    AppTerminated(i32),
    AppActivated(i32),

    MonitorAdded(u32),
    MonitorRemoved(u32),
    MonitorResized(u32),

    Command(pengwm_core::command::DaemonCommand, mpsc::Sender<pengwm_core::command::DaemonResponse>),

    Keybind(pengwm_core::command::DaemonCommand),
}

pub struct EventLoop {
    rx: mpsc::Receiver<DaemonEvent>,
    state: StateManager,
}

impl EventLoop {
    pub fn new() -> (Self, mpsc::Sender<DaemonEvent>) {
        let (tx, rx) = mpsc::channel(256);
        let state = StateManager::new(tx.clone());
        (Self { rx, state }, tx)
    }

    pub async fn run(&mut self) {
        loop {
            match self.rx.recv().await {
                Some(event) => self.dispatch(event),
                None => {
                    log::error!("Event loop channel closed");
                    break;
                }
            }
        }
    }

    fn dispatch(&mut self, event: DaemonEvent) {
        match event {
            DaemonEvent::WindowCreated(id) => self.state.on_window_created(id),
            DaemonEvent::WindowDestroyed(id) => self.state.on_window_destroyed(id),
            DaemonEvent::WindowFocused(id) => self.state.on_window_focused(id),
            DaemonEvent::AppLaunched(pid) => self.state.on_app_launched(pid),
            DaemonEvent::AppTerminated(pid) => self.state.on_app_terminated(pid),
            DaemonEvent::AppActivated(pid) => self.state.on_app_activated(pid),
            DaemonEvent::MonitorAdded(id) => self.state.on_monitor_added(id),
            DaemonEvent::MonitorRemoved(id) => self.state.on_monitor_removed(id),
            DaemonEvent::MonitorResized(id) => self.state.on_monitor_resized(id),
            DaemonEvent::Command(cmd, rtx) => self.state.on_command(cmd, rtx),
            DaemonEvent::Keybind(cmd) => {
                let (rtx, _) = mpsc::channel(1);
                self.state.on_command(cmd, rtx);
            }
        }
    }
}
