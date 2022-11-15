//! This definition is from occlum
const KERNEL_NCCS: usize = 19;

type TcflagT = u32;
type CcT = u8;
type SpeedT = u32;

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct KernelTermios {
    pub c_iflags: TcflagT,
    pub c_oflags: TcflagT,
    pub c_cflags: TcflagT,
    pub c_lflags: TcflagT,
    pub c_line: CcT,
    pub c_cc: [CcT; KERNEL_NCCS],
}

impl KernelTermios {
    /// TODO: This fake result is from whitley
    pub fn fake_kernel_termios() -> Self {
        let mut termios = KernelTermios::new();
        termios.c_iflags = 0x6d02;
        termios.c_oflags = 0x5;
        termios.c_cflags = 0x4bf;
        termios.c_lflags = 0x8acb;
        termios.c_line = 0;
        termios.c_cc = [
            0x03, 0x1c, 0x7f, 0x15, 0x04, 0x00, 0x01, 0x00, 0x11, 0x13, 0x1a, 0xff, 0x12, 0x0f,
            0x17, 0x16, 0xff, 0x00, 0x00,
        ];
        termios
    }

    fn new() -> Self {
        KernelTermios {
            c_iflags: 0,
            c_oflags: 0,
            c_cflags: 0,
            c_lflags: 0,
            c_line: 0,
            c_cc: [0; KERNEL_NCCS],
        }
    }
}
