#[derive(Debug, PartialEq, Clone, Copy)]
pub enum ThreadStatus {
    Init,
    Running,
    Exited,
    Stopped,
}

impl ThreadStatus {
    pub fn is_running(&self) -> bool {
        *self == ThreadStatus::Running
    }

    pub fn is_exited(&self) -> bool {
        *self == ThreadStatus::Exited
    }

    pub fn is_stopped(&self) -> bool {
        *self == ThreadStatus::Stopped
    }

    pub fn set_running(&mut self) {
        debug_assert!(!self.is_exited());
        *self = ThreadStatus::Running;
    }

    pub fn set_stopped(&mut self) {
        debug_assert!(!self.is_exited());
        *self = ThreadStatus::Stopped;
    }

    pub fn set_exited(&mut self) {
        *self = ThreadStatus::Exited;
    }
}
