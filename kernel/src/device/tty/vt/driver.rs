// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, format, sync::Arc};
use core::sync::atomic::{AtomicUsize, Ordering};

use aster_console::{
    AnyConsoleDevice,
    font::BitmapFont,
    mode::{ConsoleMode, KeyboardMode},
};
use ostd::mm::VmIo;

use crate::{
    current_userspace,
    device::tty::{
        CFontOp, Tty, TtyDriver,
        termio::CTermios,
        vt::{
            VtIndex,
            console::{VtConsole, VtMode, VtModeType},
            file::VtFile,
        },
    },
    fs::file::FileIo,
    prelude::*,
    util::ioctl::{RawIoctl, dispatch_ioctl},
};

/// The driver for VT (virtual terminal) devices.
pub struct VtDriver {
    console: Arc<VtConsole>,
    /// The number of open file handles to this VT.
    open_count: AtomicUsize,
}

impl VtDriver {
    fn handle_set_font(&self, font_op: &CFontOp) -> Result<()> {
        let CFontOp {
            op,
            flags: _,
            width,
            height,
            charcount,
            data,
            ..
        } = font_op;

        let vpitch = match *op {
            CFontOp::OP_SET => CFontOp::NONTALL_VPITCH,
            CFontOp::OP_SET_TALL => font_op.height,
            CFontOp::OP_SET_DEFAULT => {
                return console_set_font(self.console.as_ref(), BitmapFont::new_basic8x8());
            }
            _ => return_errno_with_message!(Errno::EINVAL, "the font operation is invalid"),
        };

        if *width == 0
            || *height == 0
            || *width > CFontOp::MAX_WIDTH
            || *height > CFontOp::MAX_HEIGHT
            || *charcount > CFontOp::MAX_CHARCOUNT
            || *height > vpitch
        {
            return_errno_with_message!(Errno::EINVAL, "the font is invalid or too large");
        }

        let font_size = width.div_ceil(u8::BITS) * vpitch * charcount;
        let mut font_data = vec![0; font_size as usize];
        current_userspace!().read_bytes(*data as Vaddr, &mut font_data[..])?;

        // In Linux, the most significant bit represents the first pixel, but `BitmapFont` requires
        // the least significant bit to represent the first pixel. So now we reverse the bits.
        font_data
            .iter_mut()
            .for_each(|byte| *byte = byte.reverse_bits());

        let font = BitmapFont::new_with_vpitch(
            *width as usize,
            *height as usize,
            vpitch as usize,
            font_data,
        );
        console_set_font(self.console.as_ref(), font)?;

        Ok(())
    }
}

fn console_set_font(console: &dyn AnyConsoleDevice, font: BitmapFont) -> Result<()> {
    use aster_console::ConsoleSetFontError;

    match console.set_font(font) {
        Ok(()) => Ok(()),
        Err(ConsoleSetFontError::InappropriateDevice) => {
            return_errno_with_message!(Errno::ENOTTY, "the console has no support for font setting")
        }
        Err(ConsoleSetFontError::InvalidFont) => {
            return_errno_with_message!(Errno::EINVAL, "the font is invalid for the console")
        }
    }
}

impl TtyDriver for VtDriver {
    // Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/linux/major.h#L18>.
    const DEVICE_MAJOR_ID: u32 = 4;

    fn devtmpfs_path(&self, index: u32) -> Option<String> {
        Some(format!("tty{}", index))
    }

    fn open(tty: Arc<Tty<Self>>) -> Result<Box<dyn FileIo>> {
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

    fn on_termios_change(&self, _old_termios: &CTermios, _new_termios: &CTermios) {}

    fn ioctl(&self, _tty: &Tty<Self>, raw_ioctl: RawIoctl) -> Result<bool>
    where
        Self: Sized,
    {
        use super::{ioctl_defs::*, manager::VT_MANAGER};

        dispatch_ioctl!(match raw_ioctl {
            cmd @ GetKeyboardType => {
                // Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/uapi/linux/kd.h#L36>.
                const KB_101: u8 = 0x02;

                cmd.write(&KB_101)?;
            }
            cmd @ SetGraphicsMode => {
                let mode = ConsoleMode::try_from(cmd.get())
                    .map_err(|_| Error::with_message(Errno::EINVAL, "invalid console mode"))?;

                if !self.console.set_mode(mode) {
                    return_errno_with_message!(Errno::EINVAL, "the console mode is not supported");
                }
            }
            cmd @ GetGraphicsMode => {
                let mode = self.console.mode().unwrap_or(ConsoleMode::Text);

                cmd.write(&(mode as i32))?;
            }
            cmd @ GetKeyboardMode => {
                let mode = self.console.keyboard_mode().unwrap_or(KeyboardMode::Xlate);

                cmd.write(&(mode as i32))?;
            }
            cmd @ SetKeyboardMode => {
                let mode = KeyboardMode::try_from(cmd.get())
                    .map_err(|_| Error::with_message(Errno::EINVAL, "invalid keyboard mode"))?;

                if !self.console.set_keyboard_mode(mode) {
                    return_errno_with_message!(Errno::EINVAL, "the keyboard mode is not supported");
                }
            }
            cmd @ SetOrGetFont => {
                let font_op = cmd.read()?;

                self.handle_set_font(&font_op)?;
            }
            cmd @ GetAvailableVt => {
                let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");
                let res = vtm
                    .get_available_vt_index()
                    .map(|index| index.get() as i32)
                    .unwrap_or(-1);

                cmd.write(&res)?;
            }
            cmd @ GetVtMode => {
                let vt_console_inner = self.console.inner();
                let vt_mode = vt_console_inner.vt_mode();
                let c_vt_mode: CVtMode = (*vt_mode).into();

                cmd.write(&c_vt_mode)?;
            }
            cmd @ SetVtMode => {
                let c_vt_mode: CVtMode = cmd.read()?;

                let vt_mode: VtMode = c_vt_mode.try_into()?;
                let mut vt_console_inner = self.console.inner();
                vt_console_inner.set_vt_mode(vt_mode);
                vt_console_inner.set_pid(Some(current!().pid()));
                vt_console_inner.set_wanted(None);
            }
            cmd @ GetVtState => {
                let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");
                let state = vtm.get_global_vt_state();

                cmd.write(&state)?;
            }
            cmd @ ReleaseDisplay => {
                // Reference: <https://elixir.bootlin.com/linux/v6.13/source/drivers/tty/vt/vt_ioctl.c#L555>

                let reldisp_type = ReleaseDisplayType::try_from(cmd.get()).map_err(|_| {
                    Error::with_message(Errno::EINVAL, "invalid release display type")
                })?;

                let mut vt_console_inner = self.console.inner();
                let mode_type = vt_console_inner.vt_mode().mode_type;
                if mode_type != VtModeType::Process {
                    return_errno_with_message!(Errno::EINVAL, "the VT is not in process mode");
                }

                let wanted = vt_console_inner.wanted();
                if wanted.is_none() {
                    return if reldisp_type == ReleaseDisplayType::AckAcquire {
                        Ok(true)
                    } else {
                        return_errno_with_message!(Errno::EINVAL, "no VT switch is pending");
                    };
                }

                let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");

                if reldisp_type == ReleaseDisplayType::DenyRelease {
                    vt_console_inner.set_wanted(None);
                    vtm.cancel_switch_vt();
                    return Ok(true);
                }

                let target = wanted.unwrap();
                vt_console_inner.set_wanted(None);

                vtm.allocate_vt(target)?;
                vtm.complete_switch_vt(target)?;
            }
            cmd @ ActivateVt => {
                let vt_index = VtIndex::new(cmd.get() as u8)
                    .ok_or_else(|| Error::with_message(Errno::ENXIO, "invalid VT index"))?;

                let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");
                vtm.allocate_vt(vt_index)?;
                vtm.switch_vt(vt_index)?;
            }
            cmd @ WaitForVtActive => {
                let vt_index = VtIndex::new(cmd.get() as u8)
                    .ok_or_else(|| Error::with_message(Errno::ENXIO, "invalid VT index"))?;

                let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");
                vtm.wait_for_vt_active(vt_index)?;
            }
            cmd @ DisallocateVt => {
                let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");

                if cmd.get() == 0 {
                    vtm.disallocate_all_vts()?;
                } else {
                    let vt_index = VtIndex::new(cmd.get() as u8)
                        .ok_or_else(|| Error::with_message(Errno::ENXIO, "invalid VT index"))?;

                    vtm.disallocate_vt(vt_index)?;
                }
            }
            _ => return Ok(false),
        });

        Ok(true)
    }
}

impl VtDriver {
    /// Creates a new VT driver with none vt console backend.
    pub(in crate::device::tty::vt) fn new() -> Self {
        Self {
            console: Arc::new(VtConsole::new()),
            open_count: AtomicUsize::new(0),
        }
    }

    /// Gets the VT console associated with this driver.
    pub(in crate::device::tty::vt) fn vt_console(&self) -> Arc<VtConsole> {
        self.console.clone()
    }

    /// Tries to switch to the framebuffer backend and activate the VT console.
    pub(in crate::device::tty::vt) fn try_switch_to_framebuffer_backend_and_activate(&self) {
        self.console.try_switch_to_framebuffer_backend();
        self.console.activate();
    }

    /// Increments the open count.
    pub(in crate::device::tty::vt) fn inc_open_count(&self) {
        let prev_count = self.open_count.fetch_add(1, Ordering::Relaxed);
        if prev_count == 0 {
            self.console.try_switch_to_framebuffer_backend();
        }
    }

    /// Decrements the open count.
    pub(in crate::device::tty::vt) fn dec_open_count(&self) {
        if self
            .open_count
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                if count == 0 { None } else { Some(count - 1) }
            })
            .is_err()
        {
            log::warn!("VtDriver open count is already zero");
        }
    }

    /// Returns whether the VT is currently open.
    pub(in crate::device::tty::vt) fn is_open(&self) -> bool {
        self.open_count.load(Ordering::Relaxed) > 0
    }
}
