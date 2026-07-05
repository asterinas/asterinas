import os, subprocess, textwrap, shutil
base = '/tmp/test_libflate'
shutil.rmtree(base, ignore_errors=True)
os.makedirs(base + '/src')
with open(base + '/Cargo.toml','w') as f:
    f.write(textwrap.dedent('''
        [package]
        name = "test_libflate"
        version = "0.1.0"
        edition = "2021"

        [dependencies]
        core2 = { git = "https://github.com/bbqsrc/core2", rev = "545e84bcb0f235b12e21351e0c69767958efe2a7", default-features = false, features = ["alloc"] }
        libflate = { version = "2.1.0", default-features = false }
    '''))
with open(base + '/src/lib.rs','w') as f:
    f.write(textwrap.dedent('''
        #![no_std]
        use core2::io::Read;
        use libflate::gzip::Decoder;

        pub fn f(buf: &[u8]) -> usize {
            let mut d = Decoder::new(buf).unwrap();
            let mut out = [0u8; 10];
            d.read(&mut out).unwrap()
        }
    '''))
print('done')
