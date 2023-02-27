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

    let decoder = CpioDecoder::new(&buffer);
    assert!(decoder.entries().count() > 3);
    for (idx, entry) in decoder.entries().enumerate() {
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
            assert!(entry.name() == "src/lib.rs");
            assert!(entry.metadata().file_type() == FileType::File);
            assert!(entry.metadata().ino() > 0);
        }
    }
}
