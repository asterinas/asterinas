// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;

use aster_device::uevent::Uevent;

/// Encodes an event as a raw uevent datagram without a netlink header.
///
/// The layout is `action@devpath\0`, then `ACTION`, `DEVPATH`, `SUBSYSTEM`,
/// the device entries, and `SEQNUM`, each terminated by NUL.
pub(super) fn serialize_uevent(event: &Uevent) -> Vec<u8> {
    let action = event.action().as_str();
    let devpath = event.devpath();
    let subsystem = event.subsystem();
    let mut seqnum_buf = [0u8; 20];
    let seqnum = format_seqnum(event.seqnum(), &mut seqnum_buf);

    let vars_len = event.vars().total_bytes();
    let capacity = action.len()
        + devpath.len()
        + 2
        + b"ACTION=".len()
        + action.len()
        + 1
        + b"DEVPATH=".len()
        + devpath.len()
        + 1
        + b"SUBSYSTEM=".len()
        + subsystem.len()
        + 1
        + vars_len
        + b"SEQNUM=".len()
        + seqnum.len()
        + 1;
    let mut raw = Vec::with_capacity(capacity);

    raw.extend_from_slice(action.as_bytes());
    raw.push(b'@');
    raw.extend_from_slice(devpath.as_bytes());
    raw.push(0);

    append_var(&mut raw, "ACTION", action);
    append_var(&mut raw, "DEVPATH", devpath);
    append_var(&mut raw, "SUBSYSTEM", subsystem);
    for (key, value) in event.vars().iter() {
        append_var(&mut raw, key, value);
    }
    append_var(&mut raw, "SEQNUM", seqnum);
    raw
}

fn append_var(raw: &mut Vec<u8>, key: &str, value: &str) {
    raw.extend_from_slice(key.as_bytes());
    raw.push(b'=');
    raw.extend_from_slice(value.as_bytes());
    raw.push(0);
}

fn format_seqnum(mut seqnum: u64, buf: &mut [u8; 20]) -> &str {
    let mut start = buf.len();
    loop {
        start -= 1;
        buf[start] = b'0' + (seqnum % 10) as u8;
        seqnum /= 10;
        if seqnum == 0 {
            break;
        }
    }
    core::str::from_utf8(&buf[start..]).expect("decimal digits are valid UTF-8")
}
