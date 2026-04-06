// Config management logic
pub mod watcher;

pub struct Config {
    pub max_tiles: usize,
    pub gap_outer: i32,
    pub gap_inner: i32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_tiles: 4,
            gap_outer: 10,
            gap_inner: 5,
        }
    }
}
