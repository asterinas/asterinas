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
            c_types::{CVtMode, ReleaseDisplayType},
            console::{VtConsole, VtMode},
            file::VtFile,
            manager::VT_MANAGER,
        },
    },
    fs::file::FileIo,
    prelude::*,
    util::ioctl::{RawIoctl, dispatch_ioctl},
};

/// The driver for VT (virtual terminal) devices.
pub struct VtDriver {
    index: VtIndex,
    console: VtConsole,
    ref_count: AtomicUsize,
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
                return console_set_font(&self.console, BitmapFont::new_basic8x8());
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
        console_set_font(&self.console, font)?;

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
        use super::ioctl_defs::*;

        dispatch_ioctl!(match raw_ioctl {
            cmd @ GetKeyboardType => {
                // Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/uapi/linux/kd.h#L36>.
                const KB_101: u8 = 0x02;

                cmd.write(&KB_101)?;
            }
            cmd @ SetGraphicsMode => {
                let mode = ConsoleMode::try_from(cmd.get())?;

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
                let mode = KeyboardMode::try_from(cmd.get())?;

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
                let res = vtm.with_lock(|m| {
                    m.get_available_vt_index()
                        .map(|index| index.get() as i32)
                        .unwrap_or(-1)
                });

                cmd.write(&res)?;
            }
            cmd @ GetVtMode => {
                // Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt_ioctl.c#L785-L798>

                let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");
                let c_vt_mode = vtm.with_lock(|_| {
                    let vt_console_inner = self.console.lock_inner();
                    (*vt_console_inner.vt_mode()).into()
                });

                cmd.write(&c_vt_mode)?;
            }
            cmd @ SetVtMode => {
                // Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt_ioctl.c#L762-L783>

                let c_vt_mode: CVtMode = cmd.read()?;

                let vt_mode: VtMode = c_vt_mode.try_into()?;

                let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");
                vtm.with_lock(|_| {
                    let mut vt_console_inner = self.console.lock_inner();
                    vt_console_inner.set_vt_mode(vt_mode);
                    vt_console_inner.set_process(Some(Arc::downgrade(&current!())));
                    vt_console_inner.set_wanted(None);
                });
            }
            cmd @ GetVtState => {
                let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");
                let state = vtm.with_lock(|m| m.get_global_vt_state());

                cmd.write(&state)?;
            }
            cmd @ ReleaseDisplay => {
                let reldisp_type = ReleaseDisplayType::try_from(cmd.get())?;

                let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");
                vtm.with_lock(|m| m.handle_release_display(self.index, reldisp_type))?;
            }
            cmd @ ActivateVt => {
                // Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt_ioctl.c#L835-L854>

                let vt_index = VtIndex::new(cmd.get() as u8)
                    .ok_or_else(|| Error::with_message(Errno::ENXIO, "the VT index is invalid"))?;

                let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");
                vtm.with_lock(|m| m.allocate_vt(vt_index))?;
                vtm.with_lock(|m| m.switch_vt(vt_index))?;
            }
            cmd @ WaitForVtActive => {
                // Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt_ioctl.c#L862-L870>

                let vt_index = VtIndex::new(cmd.get() as u8)
                    .ok_or_else(|| Error::with_message(Errno::ENXIO, "the VT index is invalid"))?;

                let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");
                vtm.wait_for_vt_active(vt_index)?;
            }
            cmd @ DisallocateVt => {
                // Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt_ioctl.c#L893-L906>

                let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");

                if cmd.get() == 0 {
                    vtm.with_lock(|m| m.disallocate_all_vts())?;
                } else {
                    let vt_index = VtIndex::new(cmd.get() as u8).ok_or_else(|| {
                        Error::with_message(Errno::ENXIO, "the VT index is invalid")
                    })?;

                    vtm.with_lock(|m| m.disallocate_vt(vt_index))?;
                }
            }
            _ => return Ok(false),
        });

        Ok(true)
    }
}

impl VtDriver {
    /// Creates a new VT driver with none vt console backend.
    pub(super) fn new(index: VtIndex) -> Self {
        Self {
            index,
            console: VtConsole::new(),
            ref_count: AtomicUsize::new(0),
        }
    }

    /// Gets the VT console associated with this driver.
    pub(super) fn vt_console(&self) -> &VtConsole {
        &self.console
    }

    /// Initializes the VT driver.
    ///
    /// This acttually includes operations below:
    /// - Increase the reference count to 1, which clones the linux
    ///   [`tty_port_init`](https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt.c#L3744)
    ///   key behavior.
    /// - Tries to switch the VT console to framebuffer backend if possible, which clones the linux
    ///   [`visual_init`](https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt.c#L3745)
    ///   key behavior.
    pub(super) fn initialize(&self) {
        self.ref_count.store(1, Ordering::Relaxed);
        self.console.try_switch_to_framebuffer_backend();
    }

    /// Checks if this VT driver is initialized by checking if the
    /// reference count is greater than 0.
    pub(super) fn is_initialized(&self) -> bool {
        self.ref_count.load(Ordering::Relaxed) > 0
    }

    /// Checks if this VT driver is currently in use by any file handle.
    pub(super) fn is_in_use(&self) -> bool {
        self.ref_count.load(Ordering::Relaxed) > 1
    }

    /// The open callback for this VT driver.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt.c#L3590-L3631>
    pub(super) fn open_callback(&self) -> Result<()> {
        let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");
        vtm.with_lock(|m| {
            m.allocate_vt(self.index)?;
            self.inc_ref_count();
            Ok(())
        })
    }

    /// The close callback for this VT driver.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt.c#L3640-L3659>
    pub(super) fn close_callback(&self) {
        self.dec_ref_count();
    }

    /// Increases the reference count of this VT driver by 1.
    fn inc_ref_count(&self) {
        self.ref_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Decreases the reference count of this VT driver by 1.
    pub(super) fn dec_ref_count(&self) {
        if self
            .ref_count
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                if count == 0 { None } else { Some(count - 1) }
            })
            .is_err()
        {
            log::warn!("VtDriver `ref_count` is already zero");
        }
    }
}
