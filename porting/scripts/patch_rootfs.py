import re
p='/home/qute-wsl/Program/os-riscv-port/kernel/src/fs/rootfs.rs'
with open(p,'r') as f: s=f.read()
old="""struct BoxedReader<'a>(Box<dyn Read + 'a>);

impl<'a> BoxedReader<'a> {
    pub fn new(reader: Box<dyn Read + 'a>) -> Self {
        BoxedReader(reader)
    }
}"""
new="""struct BoxedReader<'a>(Box<dyn Read + 'a>);

impl<'a> BoxedReader<'a> {
    pub fn new(reader: Box<dyn Read + 'a>) -> Self {
        BoxedReader(reader)
    }
}

struct GzipReader<'a>(libflate::gzip::Decoder<&'a [u8]>);

impl<'a> Read for GzipReader<'a> {
    fn read(&mut self, buf: &mut [u8]) -> core2::io::Result<usize> {
        use no_std_io2::io::Read as _;
        self.0
            .read(buf)
            .map_err(|e| core2::io::Error::new(core2::io::ErrorKind::Other, e))
    }
}"""
if old in s:
    s=s.replace(old,new)
else:
    print('old not found')
old2="""                let gzip_decoder = GZipDecoder::new(initramfs_buf)
                    .map_err(|_| Error::with_message(Errno::EINVAL, "invalid gzip buffer"))?;
                BoxedReader::new(Box::new(gzip_decoder))"""
new2="""                let gzip_decoder = GZipDecoder::new(initramfs_buf)
                    .map_err(|_| Error::with_message(Errno::EINVAL, "invalid gzip buffer"))?;
                BoxedReader::new(Box::new(GzipReader(gzip_decoder)))"""
if old2 in s:
    s=s.replace(old2,new2)
else:
    print('old2 not found')
with open(p,'w') as f: f.write(s)
print('done')
