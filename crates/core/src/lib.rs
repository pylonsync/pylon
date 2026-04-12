pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCode {
    Ok = 0,
    Usage = 64,
    Unavailable = 69,
}

impl ExitCode {
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

