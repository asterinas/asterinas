// SPDX-License-Identifier: MPL-2.0

use lending_iterator::LendingIterator;

use super::{error::*, CpioDecoder, FileType};

#[test]
fn test_decoder() {
    use std::process::{Command, Stdio};

    let manifest_path = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let manifest_path = std::path::Path::new(manifest_path.as_str());

    // Prepare the cpio buffer
    let buffer = {
        let mut find_process = Command::new("find")
            .arg(manifest_path.as_os_str())
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

    let mut decoder = CpioDecoder::new(buffer.as_slice());
    // 1st entry must be the root entry
    let entry = {
        let entry_result = decoder.next().unwrap();
        entry_result.unwrap()
    };
    assert_eq!(entry.name(), manifest_path.as_os_str());
    assert!(entry.metadata().file_type() == FileType::Dir);
    assert!(entry.metadata().ino() > 0);

    // Other entries
    while let Some(decode_result) = decoder.next() {
        let mut entry = decode_result.unwrap();
        assert!(entry.metadata().ino() > 0);
        if entry.name() == manifest_path.join("src").as_os_str() {
            assert!(entry.metadata().file_type() == FileType::Dir);
            assert!(entry.metadata().ino() > 0);
        } else if entry.name() == manifest_path.join("src").join("lib.rs").as_os_str()
            || entry.name() == manifest_path.join("src").join("test.rs").as_os_str()
            || entry.name() == manifest_path.join("src").join("error.rs").as_os_str()
            || entry.name() == manifest_path.join("Cargo.toml").as_os_str()
        {
            assert!(entry.metadata().file_type() == FileType::File);
            assert!(entry.metadata().size() > 0);
            let mut buffer: Vec<u8> = Vec::new();
            assert!(entry.read_all(&mut buffer).is_ok());
        } else {
            panic!("unexpected entry: {:?}", entry.name());
        }
    }
}

#[test]
fn test_short_buffer() {
    let short_buffer: Vec<u8> = Vec::new();
    let mut decoder = CpioDecoder::new(short_buffer.as_slice());
    let entry_result = decoder.next().unwrap();
    assert!(entry_result.is_err());
    assert!(entry_result.err() == Some(Error::BufferShortError));
}

#[test]
fn test_invalid_buffer() {
    let buffer: &[u8] = b"invalidmagic.invalidmagic.invalidmagic.invalidmagic.invalidmagic.invalidmagic.invalidmagic.invalidmagic.invalidmagic.invalidmagic";
    let mut decoder = CpioDecoder::new(buffer);
    let entry_result = decoder.next().unwrap();
    assert!(entry_result.is_err());
    assert!(entry_result.err() == Some(Error::MagicError));
}
