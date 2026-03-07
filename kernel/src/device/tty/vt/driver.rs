// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, format, sync::Arc};

use aster_console::{
    AnyConsoleDevice,
    mode::{ConsoleMode, KeyboardMode},
};
use ostd::sync::SpinLock;

use crate::{
    device::tty::{
        Tty, TtyDriver,
        termio::CTermios,
        vt::{
            VtIndex,
            console::{VtConsole, VtConsoleProxy, VtMode},
            file::VtFile,
        },
    },
    fs::inode_handle::FileIo,
    prelude::*,
    util::ioctl::{RawIoctl, dispatch_ioctl},
};

/// The driver for VT (virtual terminal) devices.
pub struct VtDriver {
    /// A stable console proxy.
    ///
    /// The actual backend can be upgraded from Dummy to Real when the vt is opened and
    /// the framebuffer is available; and can be downgraded back to Dummy when the vt is closed.
    console: VtConsoleProxy,
    /// The number of open file handles to this VT.
    open_count: SpinLock<usize>,
}

impl TtyDriver for VtDriver {
    // Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/linux/major.h#L18>.
    const DEVICE_MAJOR_ID: u32 = 4;

    fn devtmpfs_path(&self, index: u32) -> Option<String> {
        Some(format!("tty{}", index))
    }

    fn open(tty: Arc<Tty<Self>>) -> Result<Box<dyn FileIo>> {
        log::debug!("VtDriver: opening VT{}", tty.index());
        Ok(Box::new(VtFile::new(tty)?))
    }

    fn push_output(&self, chs: &[u8]) -> Result<usize> {
        self.console.send(chs);
        Ok(chs.len())
    }

    fn echo_callback(&self) -> impl FnMut(&[u8]) + '_ {
        |chs| self.console.send(chs)
    }

    fn can_push(&self) -> bool {
        true
    }

    fn notify_input(&self) {}

    fn console(&self) -> Option<&dyn AnyConsoleDevice> {
        Some(&self.console)
    }

    fn on_termios_change(&self, _old_termios: &CTermios, _new_termios: &CTermios) {}

    fn ioctl(&self, _tty: &Tty<Self>, raw_ioctl: RawIoctl) -> Result<Option<i32>> {
        use super::{ioctl_defs::*, manager::VIRTUAL_TERMINAL_MANAGER};

        let with_vt_console = |f: &mut dyn FnMut(&VtConsole) -> Result<Option<i32>>| {
            if let Some(vt_console) = self.vt_console() {
                f(&vt_console)
            } else {
                log::warn!("VtDriver: ioctl called on VT without real console");
                Ok(None)
            }
        };

        let vtm = VIRTUAL_TERMINAL_MANAGER
            .get()
            .expect("VirtualTerminalManager is not initialized");

        dispatch_ioctl! {match raw_ioctl {
            cmd @ GetAvailableVt => {
                let res = vtm
                    .get_available_vt_index()
                    .map(|index| index.get() as i32)
                    .unwrap_or(-1);

                cmd.write(&res)?
            }
            cmd @ GetVtState => {
                let state = vtm.get_global_vt_state();

                cmd.write(&state)?
            }
            cmd @ GetVtMode => {
                return with_vt_console(&mut |vt_console| {
                    let vt_mode = vt_console.vt_mode();
                    let c_vt_mode: CVtMode = vt_mode.into();
                    cmd.write(&c_vt_mode)?;
                    Ok(Some(0))
                });
            }
            cmd @ SetVtMode => {
                return with_vt_console(&mut |vt_console| {
                    let c_vt_mode: CVtMode = cmd.read()?;
                    let vt_mode: VtMode = c_vt_mode.try_into()?;

                    vt_console.set_vt_mode(vt_mode);
                    vt_console.set_pid(Some(current!().pid()));
                    Ok(Some(0))
                });
            }
            cmd @ ActivateVt => {
                let vt_index = VtIndex::new(cmd.get() as u8)
                    .ok_or_else(|| Error::with_message(Errno::ENXIO, "invalid VT index"))?;

                vtm.switch_vt(vt_index)?;
            }
            cmd @ WaitForVtActive => {
                let vt_index = VtIndex::new(cmd.get() as u8)
                    .ok_or_else(|| Error::with_message(Errno::ENXIO, "invalid VT index"))?;

                vtm.wait_for_vt_active(vt_index)?;
            }
            cmd @ SetGraphicsMode => {
                return with_vt_console(&mut |vt_console| {
                    let mode = ConsoleMode::try_from(cmd.get())
                        .map_err(|_| Error::with_message(Errno::EINVAL, "invalid console mode"))?;
                    let _ =  vt_console.set_mode(mode);
                    Ok(Some(0))
                });
            }
            cmd @ GetGraphicsMode => {
                return with_vt_console(&mut |vt_console| {
                    let mode = vt_console.mode().unwrap_or(ConsoleMode::Text);
                    cmd.write(&(mode as i32))?;
                    Ok(Some(0))
                });
            }
            cmd @ SetKeyboardMode => {
             return with_vt_console(&mut |vt_console| {
                let mode = KeyboardMode::try_from(cmd.get())
                    .map_err(|_| Error::with_message(Errno::EINVAL, "invalid keyboard mode"))?;
                let _ =  vt_console.set_keyboard_mode(mode);
                Ok(Some(0))
             });
            }
            cmd @ GetKeyboardMode => {
                return with_vt_console(&mut |vt_console| {
                    let mode = vt_console.keyboard_mode().unwrap_or(KeyboardMode::Xlate);
                    cmd.write(&(mode as i32))?;
                    Ok(Some(0))
                });
            }
            cmd @ ReleaseDisplay => {
                let release_display_type = ReleaseDisplayType::try_from(cmd.get()).
                    map_err(|_| Error::with_message(Errno::EINVAL, "invalid release display type"))?;
                vtm.handle_reldisp(release_display_type)?
            }
            _ => {
                log::debug!("VtDriver: unhandled ioctl cmd {:#x}", raw_ioctl.cmd());
                return Ok(None);
            }
        }};

        Ok(Some(0))
    }
}

impl VtDriver {
    pub(in crate::device::tty::vt) fn new() -> Self {
        Self {
            console: VtConsoleProxy::new_dummy(),
            open_count: SpinLock::new(0),
        }
    }

    /// Get a reference to the real VT console if currently in Real backend.
    pub(in crate::device::tty::vt) fn vt_console(&self) -> Option<Arc<VtConsole>> {
        self.console.vt_console()
    }

    /// Try to upgrade the backend to a real VT console and activate it.
    pub(in crate::device::tty::vt) fn try_upgrade_to_real_and_activate(&self) {
        self.console.try_upgrade_to_real();
        if let Some(vt_console) = self.vt_console() {
            vt_console.activate();
        }
    }

    /// Increments the open count.
    pub(in crate::device::tty::vt) fn inc_open(&self) {
        let mut guard = self.open_count.lock();
        if *guard == 0 && self.vt_console().is_none() {
            self.console.try_upgrade_to_real();
        }
        *guard += 1;
    }

    /// Decrements the open count.
    pub(in crate::device::tty::vt) fn dec_open(&self) {
        let mut guard = self.open_count.lock();
        if *guard == 0 {
            log::warn!("VtDriver: dec_open called when open_count is already zero");
            return;
        }
        *guard -= 1;
        if *guard == 0 {
            self.console.try_downgrade_to_dummy();
        }
    }

    /// Returns whether the VT is currently open.
    pub(in crate::device::tty::vt) fn is_open(&self) -> bool {
        *self.open_count.lock() > 0
    }
}
