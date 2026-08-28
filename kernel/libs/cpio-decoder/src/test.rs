// SPDX-License-Identifier: MPL-2.0

use lending_iterator::LendingIterator;

use super::{CpioDecoder, FileType, error::*};

struct FailingWriter;

impl std::io::Write for FailingWriter {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("injected write failure"))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn decoder() {
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
    {
        let entry = decoder.next().unwrap().unwrap();
        assert_eq!(entry.name(), manifest_path.as_os_str());
        assert!(entry.metadata().file_type() == FileType::Dir);
        assert!(entry.metadata().ino() > 0);
    }

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
fn dropping_unread_file_advances_to_next_entry() {
    let first_contents = b"unread contents";
    let second_contents = b"next file contents";
    let regular_file_mode = FileType::File as u32 | 0o644;
    let mut buffer = Vec::new();
    append_newc_entry(&mut buffer, 1, regular_file_mode, "first", first_contents);
    append_newc_entry(&mut buffer, 2, regular_file_mode, "second", second_contents);
    append_newc_entry(&mut buffer, 0, 0, "TRAILER!!!", b"");

    let mut decoder = CpioDecoder::new(buffer.as_slice());
    {
        let first_entry = decoder.next().unwrap().unwrap();
        assert_eq!(first_entry.name(), "first");
        assert_eq!(
            first_entry.metadata().size(),
            u32::try_from(first_contents.len()).unwrap()
        );
    }

    let mut second_entry = decoder.next().unwrap().unwrap();
    assert_eq!(second_entry.name(), "second");
    let mut decoded_contents = Vec::new();
    second_entry.read_all(&mut decoded_contents).unwrap();
    assert_eq!(decoded_contents, second_contents);
}

#[test]
fn writer_error_keeps_next_entry_aligned() {
    let first_contents = b"contents consumed before the write failure";
    let second_contents = b"next file contents";
    let regular_file_mode = FileType::File as u32 | 0o644;
    let mut buffer = Vec::new();
    append_newc_entry(&mut buffer, 1, regular_file_mode, "first", first_contents);
    append_newc_entry(&mut buffer, 2, regular_file_mode, "second", second_contents);
    append_newc_entry(&mut buffer, 0, 0, "TRAILER!!!", b"");

    let mut decoder = CpioDecoder::new(buffer.as_slice());
    {
        let mut first_entry = decoder.next().unwrap().unwrap();
        assert_eq!(first_entry.read_all(FailingWriter), Err(Error::IoError));
    }

    let mut second_entry = decoder.next().unwrap().unwrap();
    assert_eq!(second_entry.name(), "second");
    let mut decoded_contents = Vec::new();
    second_entry.read_all(&mut decoded_contents).unwrap();
    assert_eq!(decoded_contents, second_contents);
}

#[test]
fn short_buffer() {
    let short_buffer: Vec<u8> = Vec::new();
    let mut decoder = CpioDecoder::new(short_buffer.as_slice());
    let entry_result = decoder.next().unwrap();
    assert!(entry_result.is_err());
    assert!(entry_result.err() == Some(Error::BufferShortError));
}

#[test]
fn invalid_buffer() {
    let buffer: &[u8] = b"invalidmagic.invalidmagic.invalidmagic.invalidmagic.invalidmagic.invalidmagic.invalidmagic.invalidmagic.invalidmagic.invalidmagic";
    let mut decoder = CpioDecoder::new(buffer);
    let entry_result = decoder.next().unwrap();
    assert!(entry_result.is_err());
    assert!(entry_result.err() == Some(Error::MagicError));
}

fn append_newc_entry(buffer: &mut Vec<u8>, inode: u32, mode: u32, name: &str, contents: &[u8]) {
    let zero = 0_u32;
    let link_count = 1_u32;
    let file_size = u32::try_from(contents.len()).unwrap();
    let name_size = u32::try_from(name.len() + 1).unwrap();
    let header = format!(
        "070701{inode:08x}{mode:08x}{zero:08x}{zero:08x}{link_count:08x}{zero:08x}\
         {file_size:08x}{zero:08x}{zero:08x}{zero:08x}{zero:08x}{name_size:08x}{zero:08x}"
    );

    buffer.extend_from_slice(header.as_bytes());
    buffer.extend_from_slice(name.as_bytes());
    buffer.push(0);
    pad_to_newc_alignment(buffer);
    buffer.extend_from_slice(contents);
    pad_to_newc_alignment(buffer);
}

fn pad_to_newc_alignment(buffer: &mut Vec<u8>) {
    const NEWC_ALIGNMENT: usize = 4;

    buffer.resize(buffer.len().next_multiple_of(NEWC_ALIGNMENT), 0);
}
