use std::{
    fs::OpenOptions,
    io::{Seek, SeekFrom, Write},
    path::PathBuf,
    process::Command,
};

pub const MACHINE_ARGS: &[&str] = &[
    "-machine",
    "microvm,pit=on,pic=off,rtc=on",
    "-nodefaults",
    "-no-user-config",
];

pub const DEVICE_ARGS: &[&str] = &[
    "-device",
    "virtio-blk-device,drive=x0",
    "-device",
    "virtio-keyboard-device",
    "-device",
    "virtio-net-device,netdev=net01",
    "-device",
    "virtio-serial-device",
    "-device",
    "virtconsole,chardev=mux",
];

pub fn create_bootdev_image(path: PathBuf) -> PathBuf {
    let dir = path.parent().unwrap();
    let name = path.file_name().unwrap().to_str().unwrap().to_string();
    let elf_path = dir.join(name.clone()).to_str().unwrap().to_string();
    let strip_elf_path = dir
        .join(name.clone() + ".stripped.elf")
        .to_str()
        .unwrap()
        .to_string();

    // We use rust-strip to reduce the kernel image size.
    let status = Command::new("rust-strip")
        .arg(&elf_path)
        .arg("-o")
        .arg(&strip_elf_path)
        .status();

    match status {
        Ok(status) => {
            if !status.success() {
                panic!("Failed to strip kernel elf.");
            }
        }
        Err(err) => match err.kind() {
            std::io::ErrorKind::NotFound => panic!(
                "Not find rust-strip command, 
                try `cargo install cargo-binutils` and then rerun."
            ),
            _ => panic!("Strip kernel elf failed, err:{:#?}", err),
        },
    }

    // Because QEMU denies a x86_64 multiboot ELF file (GRUB2 accept it, btw),
    // modify `em_machine` to pretend to be an x86 (32-bit) ELF image,
    //
    // https://github.com/qemu/qemu/blob/950c4e6c94b15cd0d8b63891dddd7a8dbf458e6a/hw/i386/multiboot.c#L197
    // Set EM_386 (0x0003) to em_machine.
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&strip_elf_path)
        .unwrap();

    let bytes: [u8; 2] = [0x03, 0x00];

    file.seek(SeekFrom::Start(18)).unwrap();
    file.write_all(&bytes).unwrap();
    file.flush().unwrap();

    strip_elf_path.into()
}
