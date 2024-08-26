// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;

bitflags! {
    /// Flags used for send/recv.
    /// The definition is from https://elixir.bootlin.com/linux/v6.0.9/source/include/linux/socket.h
    #[repr(C)]
    #[derive(Pod)]
    pub struct SendRecvFlags: i32 {
        const MSG_OOB = 1;
        const MSG_PEEK	= 2;
        const MSG_DONTROUTE	= 4;
        // const MSG_TRYHARD  =   4;       /* Synonym for MSG_DONTROUTE for DECnet */
        const MSG_CTRUNC	= 8;
        const MSG_PROBE	= 0x10;	/* Do not send. Only probe path f.e. for MTU */
        const MSG_TRUNC	= 0x20;
        const MSG_DONTWAIT	= 0x40;	/* Nonblocking io		 */
        const MSG_EOR       = 0x80;	/* End of record */
        const MSG_WAITALL	= 0x100;	/* Wait for a full request */
        const MSG_FIN        = 0x200;
        const MSG_SYN		= 0x400;
        const MSG_CONFIRM	= 0x800;	/* Confirm path validity */
        const MSG_RST		= 0x1000;
        const MSG_ERRQUEUE	= 0x2000;	/* Fetch message from error queue */
        const MSG_NOSIGNAL	= 0x4000;	/* Do not generate SIGPIPE */
        const MSG_MORE	= 0x8000;	/* Sender will send more */
        const MSG_WAITFORONE	= 0x10000;	/* recvmmsg(): block until 1+ packets avail */
        const MSG_SENDPAGE_NOPOLICY = 0x10000; /* sendpage() internal : do no apply policy */
        const MSG_SENDPAGE_NOTLAST = 0x20000; /* sendpage() internal : not the last page */
        const MSG_BATCH	= 0x40000; /* sendmmsg(): more messages coming */
        // const MSG_EOF         MSG_FIN
        const MSG_NO_SHARED_FRAGS = 0x80000; /* sendpage() internal : page frags are not shared */
        const MSG_SENDPAGE_DECRYPTED	= 0x100000; /* sendpage() internal : page may carry plain text and require encryption */
    }
}

impl SendRecvFlags {
    fn supported_flags() -> Self {
        SendRecvFlags::empty()
    }

    pub fn is_all_supported(&self) -> bool {
        let supported_flags = Self::supported_flags();
        supported_flags.contains(*self)
    }
}
