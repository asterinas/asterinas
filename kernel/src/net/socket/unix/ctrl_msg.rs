// SPDX-License-Identifier: MPL-2.0

use core::fmt;

use aster_rights::ReadOp;
use ostd::task::Task;

use super::{cred::SocketCred, CUserCred, UnixStreamSocket};
use crate::{
    fs::{
        file_handle::FileLike,
        file_table::{get_file_fast, FdFlags},
    },
    net::socket::util::{CControlHeader, ControlMessage},
    prelude::*,
    process::{credentials::capabilities::CapSet, posix_thread::AsPosixThread},
    util::net::CSocketOptionLevel,
};

#[derive(Debug)]
pub struct UnixControlMessage(Message);

#[derive(Debug)]
enum Message {
    Files(FileMessage),
    Cred(CredMessage),
}

impl UnixControlMessage {
    pub fn read_from(header: &CControlHeader, reader: &mut VmReader) -> Result<Option<Self>> {
        debug_assert_eq!(header.level(), Some(CSocketOptionLevel::SOL_SOCKET));

        let Ok(type_) = CControlType::try_from(header.type_()) else {
            warn!("unsupported control message type in {:?}", header);
            reader.skip(header.payload_len());
            return Ok(None);
        };

        match type_ {
            CControlType::SCM_RIGHTS => {
                let msg = FileMessage::read_from(header, reader)?;
                Ok(Some(Self(Message::Files(msg))))
            }
            CControlType::SCM_CREDENTIALS => {
                let msg = CredMessage::read_from(header, reader)?;
                Ok(Some(Self(Message::Cred(msg))))
            }
            _ => {
                warn!("unsupported control message type in {:?}", header);
                reader.skip(header.payload_len());
                Ok(None)
            }
        }
    }

    pub fn write_to(&self, writer: &mut VmWriter) -> Result<CControlHeader> {
        match &self.0 {
            Message::Files(msg) => msg.write_to(writer),
            Message::Cred(msg) => msg.write_to(writer),
        }
    }
}

struct FileMessage {
    files: Vec<Arc<dyn FileLike>>,
}

impl fmt::Debug for FileMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileMessage")
            .field("len", &self.files.len())
            .finish_non_exhaustive()
    }
}

/// The maximum number of the file descriptors in the control messages.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.15/source/include/net/scm.h#L18>.
const MAX_NR_FILES: usize = 253;

impl FileMessage {
    fn read_from(header: &CControlHeader, reader: &mut VmReader) -> Result<Self> {
        let payload_len = header.payload_len();
        if payload_len % size_of::<i32>() != 0 {
            return_errno_with_message!(Errno::EINVAL, "the SCM_RIGHTS message is invalid");
        }
        let nfiles = payload_len / size_of::<i32>();

        // "Attempting to send an array larger than this limit causes sendmsg(2) to fail with the
        // error EINVAL." -- Reference: <https://man7.org/linux/man-pages/man7/unix.7.html>.
        if nfiles > MAX_NR_FILES {
            return_errno_with_message!(Errno::EINVAL, "the SCM_RIGHTS message is too large");
        }
        // TODO: "[the ETOOMANYREFS error] occurs if the number of "in-flight" file descriptors
        // exceeds the RLIMIT_NOFILE resource limit and the caller does not have the
        // CAP_SYS_RESOURCE capability."

        let mut files = Vec::with_capacity(nfiles);

        let current = Task::current().unwrap();
        let mut file_table = current.as_thread_local().unwrap().borrow_file_table_mut();
        for _ in 0..nfiles {
            let fd = reader.read_val::<i32>()?;
            let file = get_file_fast!(&mut file_table, fd).into_owned();
            files.push(file);
        }

        Ok(FileMessage { files })
    }

    fn write_to(&self, writer: &mut VmWriter) -> Result<CControlHeader> {
        let nfiles = self
            .files
            .len()
            .min(CControlHeader::payload_len_from_total(writer.avail())? / size_of::<i32>());
        if nfiles == 0 {
            return_errno_with_message!(Errno::EINVAL, "the control message buffer is too small");
        }
        if nfiles < self.files.len() {
            warn!("setting MSG_CTRUNC is not supported");
        }

        let header = CControlHeader::new(
            CSocketOptionLevel::SOL_SOCKET,
            CControlType::SCM_RIGHTS as i32,
            nfiles * size_of::<i32>(),
        );
        writer.write_val::<CControlHeader>(&header)?;

        let current = Task::current().unwrap();
        let file_table = current.as_thread_local().unwrap().borrow_file_table();
        for file in self.files[..nfiles].iter() {
            // TODO: Deal with the `O_CLOEXEC` flag.
            let fd = file_table
                .unwrap()
                .write()
                .insert(file.clone(), FdFlags::empty());
            // Perhaps we should remove the inserted files from the file table if we cannot write
            // the file descriptor back to user space? However, even Linux cannot handle every
            // corner case (https://elixir.bootlin.com/linux/v6.15.2/source/net/core/scm.c#L357).
            writer.write_val::<i32>(&fd)?;
        }

        Ok(header)
    }
}

#[derive(Debug)]
struct CredMessage {
    cred: CUserCred,
}

impl CredMessage {
    fn read_from(header: &CControlHeader, reader: &mut VmReader) -> Result<Self> {
        if header.payload_len() != size_of::<CUserCred>() {
            return_errno_with_message!(Errno::EINVAL, "the SCM_CREDENTIALS message is invalid");
        }

        let cred = reader.read_val()?;

        Ok(Self { cred })
    }

    fn write_to(&self, writer: &mut VmWriter) -> Result<CControlHeader> {
        let payload_len =
            size_of::<CUserCred>().min(CControlHeader::payload_len_from_total(writer.avail())?);
        if payload_len != size_of::<CUserCred>() {
            warn!("setting MSG_CTRUNC is not supported");
        }

        let header = CControlHeader::new(
            CSocketOptionLevel::SOL_SOCKET,
            CControlType::SCM_CREDENTIALS as i32,
            payload_len,
        );
        writer.write_val(&header)?;
        writer.write_fallible(&mut VmReader::from(self.cred.as_bytes()))?;

        Ok(header)
    }
}

/// Control message types.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/linux/socket.h#L178>.
#[repr(i32)]
#[derive(Debug, Clone, Copy, TryFromInt, PartialEq, Eq)]
#[expect(non_camel_case_types)]
enum CControlType {
    SCM_RIGHTS = 1,
    SCM_CREDENTIALS = 2,
    SCM_SECURITY = 3,
    SCM_PIDFD = 4,
}

/// Auxiliary data associated with UNIX messages.
///
/// In UNIX sockets, one can send payload bytes with multiple control messages. If these control
/// messages need to be sent to a remote endpoint, they are packaged in this type and transmitted.
///
/// We use this type instead of transmitting control messages directly to the remote endpoint
/// because control messages of the same type (e.g., files) can be merged and missing control
/// messages of certain types (e.g., credentials) can be supplied automatically according to socket
/// option settings.
#[derive(Default)]
pub(super) struct AuxiliaryData {
    files: Vec<Arc<dyn FileLike>>,
    cred: Option<SocketCred>,
}

impl AuxiliaryData {
    /// Builds the auxiliary data from the control messages.
    pub(super) fn from_control(ctrl_msgs: Vec<ControlMessage>) -> Result<Self> {
        let mut files = Vec::new();
        let mut cred = None;

        for ctrl_msg in ctrl_msgs.into_iter() {
            let ControlMessage::Unix(unix_ctrl_msg) = ctrl_msg;
            // TODO: What should we do if there are control messages of other protocols?

            match unix_ctrl_msg.0 {
                Message::Files(FileMessage {
                    files: mut msg_files,
                }) => {
                    if msg_files.len() > MAX_NR_FILES - files.len() {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "the SCM_RIGHTS message is too large"
                        );
                    }
                    files.append(&mut msg_files);
                }
                Message::Cred(CredMessage { cred: msg_cred }) => {
                    let cur_cred = SocketCred::<ReadOp>::new_current();
                    if cur_cred.to_real_c_cred() != msg_cred {
                        // FIXME: Allow this if we're root or have the CAP_SYS_ADMIN capability.
                        return_errno_with_message!(
                            Errno::EPERM,
                            "setting others' credentials is not allowed"
                        );
                    }
                    cred = Some(cur_cred);
                }
            }
        }

        // FIXME: Sending UNIX sockets over UNIX sockets can easily lead to circular references and
        // memory leaks. Linux uses a complex garbage collection algorithm to address these issues.
        // See also <https://elixir.bootlin.com/linux/v6.15/source/net/unix/garbage.c#L592>.
        if files
            .iter()
            .any(|file| (&**file as &dyn Any).is::<UnixStreamSocket>())
        {
            warn!("UNIX sockets in SCM_RIGHTS messages can leak kernel resource");

            let credentials = current_thread!().as_posix_thread().unwrap().credentials();
            if !credentials.euid().is_root()
                && !credentials.effective_capset().contains(CapSet::SYS_ADMIN)
            {
                return_errno_with_message!(
                    Errno::EPERM,
                    "UNIX sockets in SCM_RIGHTS messages can leak kernel resource"
                )
            }
        }

        Ok(Self { files, cred })
    }

    /// Fills the current credentials if there are no credentials.
    pub(super) fn fill_cred(&mut self) {
        if self.cred.is_none() {
            self.cred = Some(SocketCred::<ReadOp>::new_current());
        }
    }

    /// Generates the control messages from the auxiliary data.
    pub(super) fn generate_control(&mut self, is_pass_cred: bool) -> Vec<ControlMessage> {
        let mut ctrl_msgs = Vec::new();

        let Self { files, cred } = self;

        if is_pass_cred {
            let unix_ctrl_msg = UnixControlMessage(Message::Cred(CredMessage {
                cred: cred
                    .as_ref()
                    .map(SocketCred::to_real_c_cred)
                    .unwrap_or_else(CUserCred::new_overflow),
            }));
            ctrl_msgs.push(ControlMessage::Unix(unix_ctrl_msg));
        }

        if !files.is_empty() {
            let unix_ctrl_msg = UnixControlMessage(Message::Files(FileMessage {
                files: core::mem::take(files),
            }));
            ctrl_msgs.push(ControlMessage::Unix(unix_ctrl_msg));
        }

        ctrl_msgs
    }

    /// Returns whether the auxiliary data contains nothing.
    pub(super) fn is_empty(&self) -> bool {
        self.files.is_empty() && self.cred.is_none()
    }

    /// Returns whether the auxiliary data can be treated as a subset of the other one.
    ///
    /// In stream sockets, we can receive more bytes at once if the current auxiliary data is a
    /// subset of the subsequent auxiliary data.
    pub(super) fn is_subset_of(&self, other: &Self, is_pass_cred: bool) -> bool {
        if !self.files.is_empty() {
            return false;
        }

        if is_pass_cred
            && self.cred.as_ref().map(SocketCred::to_real_c_cred)
                != other.cred.as_ref().map(SocketCred::to_real_c_cred)
        {
            return false;
        }

        true
    }
}
