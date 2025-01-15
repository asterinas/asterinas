// SPDX-License-Identifier: MPL-2.0

use aster_virtio::device::gpu::GPU_DEVICE;

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_show_red(_ctx: &Context) -> Result<SyscallReturn> {
    println!("Red");
    let gpu_device = GPU_DEVICE.get().expect("GPU device not initialized");
    // gpu_device.lock().update_cursor(resource_id, scanout_id, pos_x, pos_y, hot_x, hot_y);
    let reso = gpu_device
        .lock()
        .resolution()
        .expect("Failed to get resolution");
    println!("Resolution: {}x{}", reso.0, reso.1);

    // show another image on the display
    gpu_device.lock().show_red().expect("error showing red");

    Ok(SyscallReturn::NoReturn)
}
