use super::error::*;
use super::{CpioDecoder, FileType};

#[test]
fn test_decoder() {
    use std::process::{Command, Stdio};

    // Prepare the cpio buffer
    let buffer = {
        let mut find_process = Command::new("find")
            .arg(".")
            .stdout(Stdio::piped())
            .spawn()
            .expect("find command is not started");
        let ecode = find_process.wait().expect("failed to execute find");
        assert!(ecode.success());
        let find_stdout = find_process.stdout.take().unwrap();
        let output = Command::new("cpio")
            .stdin(find_stdout)
            .args(["-o", "-H", "newc"])
            .output()
            .expect("failed to execute cpio");
        assert!(output.status.success());
        output.stdout
    };

    assert!(CpioDecoder::new(buffer.as_slice()).decode_entries().count() > 3);
    let mut decoder = CpioDecoder::new(buffer.as_slice());
    for (idx, entry_result) in decoder.decode_entries().enumerate() {
        let entry = entry_result.unwrap();
        if idx == 0 {
            assert!(entry.name() == ".");
            assert!(entry.metadata().file_type() == FileType::Dir);
            assert!(entry.metadata().ino() > 0);
        }
        if idx == 1 {
            assert!(entry.name() == "src");
            assert!(entry.metadata().file_type() == FileType::Dir);
            assert!(entry.metadata().ino() > 0);
        }
        if idx == 2 {
            assert!(
                entry.name() == "src/lib.rs"
                    || entry.name() == "src/test.rs"
                    || entry.name() == "src/error.rs"
            );
            assert!(entry.metadata().file_type() == FileType::File);
            assert!(entry.metadata().ino() > 0);
        }
    }
}

#[test]
fn test_short_buffer() {
    let short_buffer: Vec<u8> = Vec::new();
    let mut decoder = CpioDecoder::new(short_buffer.as_slice());
    for entry_result in decoder.decode_entries() {
        assert!(entry_result.is_err());
        assert!(entry_result.err() == Some(Error::BufferShortError));
    }
}

#[test]
fn test_invalid_buffer() {
    let buffer: &[u8] = b"invalidmagic.invalidmagic.invalidmagic.invalidmagic.invalidmagic.invalidmagic.invalidmagic.invalidmagic.invalidmagic.invalidmagic";
    let mut decoder = CpioDecoder::new(buffer);
    for entry_result in decoder.decode_entries() {
        assert!(entry_result.is_err());
        assert!(entry_result.err() == Some(Error::MagicError));
    }
}
