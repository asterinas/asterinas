// SPDX-License-Identifier: MPL-2.0

use core::{sync::atomic::AtomicU64, time::Duration};

use aster_systree::EmptyNode;
use aster_util::printer::VmPrinter;
use spin::Once;

use crate::{
    fs::{
        file::{AccessMode, InodeMode, InodeType, PerOpenFileOps, StatusFlags, mkmod},
        pseudofs::AnonDeviceId,
        utils::{DirentVisitor, NAME_MAX},
        vfs::{
            file_system::{FileSystem, FsEventSubscriberStats, SuperBlock},
            inode::{
                Extension, FileOps, Inode, Metadata, MknodType, RenameMode, RevalidationPolicy,
                SymbolicLink,
            },
            path::{is_dot, is_dotdot},
            registry::{FsCreationCtx, FsProperties, FsType},
        },
    },
    prelude::*,
    process::{
        Gid, Uid, UserNamespace, credentials::capabilities::CapSet, posix_thread::AsPosixThread,
    },
    security::{self, AppArmorPolicyOperation},
};

const SECURITYFS_MAGIC: u64 = 0x73636673;
const SECURITYFS_BLOCK_SIZE: usize = 4096;
const ROOT_INO: u64 = 1;
const MAX_BINARY_POLICY_LEN: usize = 1024 * 1024;
const MAX_REMOVE_TEXT_LEN: usize = 4096;

pub(super) fn init() {
    let security_kernel_sysnode = EmptyNode::new("security".into());
    super::sysfs::register_kernel_sysnode(security_kernel_sysnode).unwrap();

    crate::fs::vfs::registry::register(&SecurityFsType).unwrap();
}

struct SecurityFs {
    _anon_device_id: AnonDeviceId,
    sb: SuperBlock,
    root: Arc<dyn Inode>,
    inode_allocator: AtomicU64,
    fs_event_subscriber_stats: FsEventSubscriberStats,
}

impl SecurityFs {
    fn singleton() -> &'static Arc<Self> {
        static SINGLETON: Once<Arc<SecurityFs>> = Once::new();

        SINGLETON.call_once(Self::new)
    }

    fn new() -> Arc<Self> {
        let anon_device_id =
            AnonDeviceId::acquire().expect("no device ID is available for securityfs");
        let sb = SuperBlock::new(
            SECURITYFS_MAGIC,
            SECURITYFS_BLOCK_SIZE,
            NAME_MAX,
            anon_device_id.id(),
        );

        Arc::new_cyclic(|weak_fs| Self {
            _anon_device_id: anon_device_id,
            sb: sb.clone(),
            root: SecurityFsInode::new_root(weak_fs.clone(), &sb),
            inode_allocator: AtomicU64::new(ROOT_INO + 1),
            fs_event_subscriber_stats: FsEventSubscriberStats::new(),
        })
    }

    fn alloc_id(&self) -> u64 {
        self.inode_allocator
            .fetch_add(1, core::sync::atomic::Ordering::Relaxed)
    }
}

impl FileSystem for SecurityFs {
    fn name(&self) -> &'static str {
        "securityfs"
    }

    fn sync(&self) -> Result<()> {
        Ok(())
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        self.root.clone()
    }

    fn sb(&self) -> SuperBlock {
        self.sb.clone()
    }

    fn fs_event_subscriber_stats(&self) -> &FsEventSubscriberStats {
        &self.fs_event_subscriber_stats
    }
}

struct SecurityFsType;

impl FsType for SecurityFsType {
    fn name(&self) -> &'static str {
        "securityfs"
    }

    fn properties(&self) -> FsProperties {
        FsProperties::empty()
    }

    fn create(&self, _fs_creation_ctx: &FsCreationCtx) -> Result<Arc<dyn FileSystem>> {
        Ok(SecurityFs::singleton().clone())
    }

    fn sysnode(&self) -> Option<Arc<dyn aster_systree::SysNode>> {
        None
    }
}

struct SecurityFsInode {
    kind: SecurityFsNodeKind,
    metadata: RwLock<Metadata>,
    extension: Extension,
    fs: Weak<dyn FileSystem>,
    parent: Option<Weak<dyn Inode>>,
    this: Weak<SecurityFsInode>,
}

impl SecurityFsInode {
    fn new_root(fs: Weak<SecurityFs>, sb: &SuperBlock) -> Arc<dyn Inode> {
        let fs: Weak<dyn FileSystem> = fs;
        let metadata = Metadata::new_dir(
            ROOT_INO,
            SecurityFsNodeKind::Root.mode(),
            SECURITYFS_BLOCK_SIZE,
            sb.container_dev_id,
        );

        Arc::new_cyclic(|this| Self {
            kind: SecurityFsNodeKind::Root,
            metadata: RwLock::new(metadata),
            extension: Extension::new(),
            fs: fs.clone(),
            parent: None,
            this: this.clone(),
        })
    }

    fn new_child(kind: SecurityFsNodeKind, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        let fs = parent.upgrade().unwrap().fs();
        let securityfs = fs.downcast_ref::<SecurityFs>().unwrap();
        let metadata = match kind.type_() {
            InodeType::Dir => Metadata::new_dir(
                securityfs.alloc_id(),
                kind.mode(),
                SECURITYFS_BLOCK_SIZE,
                securityfs.sb().container_dev_id,
            ),
            InodeType::File => Metadata::new_file(
                securityfs.alloc_id(),
                kind.mode(),
                SECURITYFS_BLOCK_SIZE,
                securityfs.sb().container_dev_id,
            ),
            _ => unreachable!("securityfs only creates directories and regular files"),
        };

        Arc::new_cyclic(|this| Self {
            kind,
            metadata: RwLock::new(metadata),
            extension: Extension::new(),
            fs: Arc::downgrade(&fs),
            parent: Some(parent),
            this: this.clone(),
        })
    }

    fn this(&self) -> Arc<dyn Inode> {
        self.this.upgrade().unwrap()
    }

    fn parent(&self) -> Option<Arc<dyn Inode>> {
        self.parent.as_ref().and_then(Weak::upgrade)
    }

    fn entries(&self) -> Vec<SecurityFsEntry> {
        match self.kind {
            SecurityFsNodeKind::Root => {
                if security::is_apparmor_enabled() {
                    vec![SecurityFsEntry::new(
                        "apparmor",
                        SecurityFsNodeKind::AppArmorDir,
                    )]
                } else {
                    Vec::new()
                }
            }
            SecurityFsNodeKind::AppArmorDir => vec![
                SecurityFsEntry::new("profiles", SecurityFsNodeKind::Profiles),
                SecurityFsEntry::new(".load", SecurityFsNodeKind::Load),
                SecurityFsEntry::new(".replace", SecurityFsNodeKind::Replace),
                SecurityFsEntry::new(".remove", SecurityFsNodeKind::Remove),
                SecurityFsEntry::new("features", SecurityFsNodeKind::FeaturesDir),
            ],
            SecurityFsNodeKind::FeaturesDir => vec![
                SecurityFsEntry::new("abi", SecurityFsNodeKind::FeatureAbi),
                SecurityFsEntry::new("policy", SecurityFsNodeKind::FeaturePolicyDir),
                SecurityFsEntry::new("file", SecurityFsNodeKind::FeatureFileDir),
                SecurityFsEntry::new("domain", SecurityFsNodeKind::FeatureDomainDir),
                SecurityFsEntry::new("caps", SecurityFsNodeKind::FeatureCapsDir),
            ],
            SecurityFsNodeKind::FeaturePolicyDir => vec![
                SecurityFsEntry::new("versions", SecurityFsNodeKind::FeaturePolicyVersionsDir),
                SecurityFsEntry::new("set_load", SecurityFsNodeKind::FeaturePolicySetLoad),
                SecurityFsEntry::new(
                    "permstable32",
                    SecurityFsNodeKind::FeaturePolicyPermstable32,
                ),
                SecurityFsEntry::new(
                    "permstable32_version",
                    SecurityFsNodeKind::FeaturePolicyPermstable32Version,
                ),
            ],
            SecurityFsNodeKind::FeaturePolicyVersionsDir => vec![
                SecurityFsEntry::new("v5", SecurityFsNodeKind::FeaturePolicyVersionV5),
                SecurityFsEntry::new("v6", SecurityFsNodeKind::FeaturePolicyVersionV6),
                SecurityFsEntry::new("v7", SecurityFsNodeKind::FeaturePolicyVersionV7),
                SecurityFsEntry::new("v8", SecurityFsNodeKind::FeaturePolicyVersionV8),
                SecurityFsEntry::new("v9", SecurityFsNodeKind::FeaturePolicyVersionV9),
            ],
            SecurityFsNodeKind::FeatureFileDir => {
                vec![SecurityFsEntry::new(
                    "mask",
                    SecurityFsNodeKind::FeatureFileMask,
                )]
            }
            SecurityFsNodeKind::FeatureCapsDir => {
                vec![SecurityFsEntry::new(
                    "mask",
                    SecurityFsNodeKind::FeatureCapsMask,
                )]
            }
            SecurityFsNodeKind::FeatureDomainDir => vec![
                SecurityFsEntry::new(
                    "change_profile",
                    SecurityFsNodeKind::FeatureDomainChangeProfile,
                ),
                SecurityFsEntry::new(
                    "change_onexec",
                    SecurityFsNodeKind::FeatureDomainChangeOnexec,
                ),
                SecurityFsEntry::new("version", SecurityFsNodeKind::FeatureDomainVersion),
            ],
            _ => Vec::new(),
        }
    }

    fn lookup_entry(&self, name: &str) -> Option<SecurityFsEntry> {
        self.entries().into_iter().find(|entry| entry.name == name)
    }
}

impl FileOps for SecurityFsInode {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        if self.kind.type_() == InodeType::Dir {
            return_errno_with_message!(Errno::EISDIR, "securityfs directories are not readable");
        }

        let mut printer = VmPrinter::new_skip(writer, offset);
        match self.kind {
            SecurityFsNodeKind::Profiles => {
                for (profile_name, mode) in security::apparmor_profile_summaries()? {
                    writeln!(printer, "{} {}", profile_name.as_str(), mode.as_str())?;
                }
            }
            SecurityFsNodeKind::FeatureAbi => {
                writeln!(printer, "asterinas-apparmor-linux-filedfa-v1")?;
                writeln!(
                    printer,
                    "root_namespace={}",
                    security::apparmor_root_namespace_name()?
                )?;
                writeln!(printer, "policy_abi=linux-v5-v9-subset")?;
                writeln!(printer, "file_audit=yes")?;
                writeln!(printer, "file_quiet=yes")?;
                writeln!(printer, "capability_audit=yes")?;
                writeln!(printer, "complain=yes")?;
            }
            SecurityFsNodeKind::FeaturePolicySetLoad
            | SecurityFsNodeKind::FeaturePolicyVersionV5
            | SecurityFsNodeKind::FeaturePolicyVersionV6
            | SecurityFsNodeKind::FeaturePolicyVersionV7
            | SecurityFsNodeKind::FeaturePolicyVersionV8
            | SecurityFsNodeKind::FeaturePolicyVersionV9
            | SecurityFsNodeKind::FeatureDomainChangeProfile
            | SecurityFsNodeKind::FeatureDomainChangeOnexec => {
                writeln!(printer, "yes")?;
            }
            SecurityFsNodeKind::FeaturePolicyPermstable32 => {
                writeln!(printer, "allow deny audit quiet xindex")?;
            }
            SecurityFsNodeKind::FeaturePolicyPermstable32Version => {
                writeln!(printer, "0x000002")?;
            }
            SecurityFsNodeKind::FeatureFileMask => {
                writeln!(
                    printer,
                    "create read write exec append delete rename setattr mmap_exec link"
                )?;
            }
            SecurityFsNodeKind::FeatureCapsMask => {
                writeln!(
                    printer,
                    "chown dac_override dac_read_search fowner fsetid kill setgid setuid setpcap linux_immutable net_bind_service net_broadcast net_admin net_raw ipc_lock ipc_owner sys_module sys_rawio sys_chroot sys_ptrace sys_pacct sys_admin sys_boot sys_nice sys_resource sys_time sys_tty_config mknod lease audit_write audit_control setfcap mac_override mac_admin syslog wake_alarm block_suspend audit_read perfmon bpf checkpoint_restore"
                )?;
            }
            SecurityFsNodeKind::FeatureDomainVersion => {
                writeln!(printer, "1.2")?;
            }
            SecurityFsNodeKind::Load | SecurityFsNodeKind::Replace | SecurityFsNodeKind::Remove => {
                return_errno_with_message!(Errno::EPERM, "the securityfs file is not readable");
            }
            _ => return_errno_with_message!(Errno::EINVAL, "invalid securityfs read target"),
        }

        Ok(printer.bytes_written())
    }

    fn write_at(
        &self,
        _offset: usize,
        reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        if self.kind.type_() == InodeType::Dir {
            return_errno_with_message!(Errno::EISDIR, "securityfs directories are not writable");
        }

        match self.kind {
            SecurityFsNodeKind::Load | SecurityFsNodeKind::Replace => {
                require_mac_admin()?;
                let (policy, read_bytes) = read_bytes_from(reader, MAX_BINARY_POLICY_LEN)?;
                security::load_apparmor_binary_policy(&policy, AppArmorPolicyOperation::Replace)?;
                Ok(read_bytes)
            }
            SecurityFsNodeKind::Remove => {
                require_mac_admin()?;
                let (policy, read_bytes) = read_bytes_from(reader, MAX_BINARY_POLICY_LEN)?;
                if security::has_apparmor_binary_policy_magic(&policy) {
                    security::load_apparmor_binary_policy(
                        &policy,
                        AppArmorPolicyOperation::Remove,
                    )?;
                } else {
                    let profile_name = parse_remove_profile_name(&policy)?;
                    security::remove_apparmor_profile_by_name(profile_name)?;
                }
                Ok(read_bytes)
            }
            _ => return_errno_with_message!(Errno::EPERM, "the securityfs file is not writable"),
        }
    }

    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        if self.kind.type_() != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "securityfs inode is not a directory");
        }

        let try_readdir =
            |iterate_offset: &mut usize, visitor: &mut dyn DirentVisitor| -> Result<()> {
                let this_inode = self.this();
                let parent_inode = self.parent().unwrap_or_else(|| this_inode.clone());
                for (name, inode, next_offset) in
                    [(".", this_inode, 1usize), ("..", parent_inode, 2usize)]
                {
                    if next_offset <= *iterate_offset {
                        continue;
                    }

                    visitor.visit(name, inode.ino(), inode.type_(), next_offset)?;
                    *iterate_offset = next_offset;
                }

                for (index, entry) in self.entries().into_iter().enumerate() {
                    let next_offset = 3 + index;
                    if next_offset <= *iterate_offset {
                        continue;
                    }

                    visitor.visit(entry.name, 2, entry.kind.type_(), next_offset)?;
                    *iterate_offset = next_offset;
                }

                Ok(())
            };

        let mut iterate_offset = offset;
        match try_readdir(&mut iterate_offset, visitor) {
            Err(error) if iterate_offset == offset => Err(error),
            _ => Ok(iterate_offset - offset),
        }
    }
}

impl Inode for SecurityFsInode {
    fn size(&self) -> usize {
        self.metadata.read().size
    }

    fn resize(&self, _new_size: usize) -> Result<()> {
        Ok(())
    }

    fn metadata(&self) -> Metadata {
        *self.metadata.read()
    }

    fn ino(&self) -> u64 {
        self.metadata.read().ino
    }

    fn type_(&self) -> InodeType {
        self.kind.type_()
    }

    fn mode(&self) -> Result<InodeMode> {
        Ok(self.metadata.read().mode)
    }

    fn set_mode(&self, mode: InodeMode) -> Result<()> {
        self.metadata.write().mode = mode;
        Ok(())
    }

    fn owner(&self) -> Result<Uid> {
        Ok(self.metadata.read().uid)
    }

    fn set_owner(&self, uid: Uid) -> Result<()> {
        self.metadata.write().uid = uid;
        Ok(())
    }

    fn group(&self) -> Result<Gid> {
        Ok(self.metadata.read().gid)
    }

    fn set_group(&self, gid: Gid) -> Result<()> {
        self.metadata.write().gid = gid;
        Ok(())
    }

    fn atime(&self) -> Duration {
        self.metadata.read().last_access_at
    }

    fn set_atime(&self, time: Duration) {
        self.metadata.write().last_access_at = time;
    }

    fn mtime(&self) -> Duration {
        self.metadata.read().last_modify_at
    }

    fn set_mtime(&self, time: Duration) {
        self.metadata.write().last_modify_at = time;
    }

    fn ctime(&self) -> Duration {
        self.metadata.read().last_meta_change_at
    }

    fn set_ctime(&self, time: Duration) {
        self.metadata.write().last_meta_change_at = time;
    }

    fn open(
        &self,
        _access_mode: AccessMode,
        _status_flags: StatusFlags,
    ) -> Option<Result<Box<dyn PerOpenFileOps>>> {
        None
    }

    fn create(&self, _name: &str, _type_: InodeType, _mode: InodeMode) -> Result<Arc<dyn Inode>> {
        return_errno_with_message!(Errno::EPERM, "securityfs does not support create");
    }

    fn mknod(&self, _name: &str, _mode: InodeMode, _type_: MknodType) -> Result<Arc<dyn Inode>> {
        return_errno_with_message!(Errno::EPERM, "securityfs does not support mknod");
    }

    fn link(&self, _old: &Arc<dyn Inode>, _name: &str) -> Result<()> {
        return_errno_with_message!(Errno::EPERM, "securityfs does not support link");
    }

    fn unlink(&self, _name: &str) -> Result<()> {
        return_errno_with_message!(Errno::EPERM, "securityfs does not support unlink");
    }

    fn rmdir(&self, _name: &str) -> Result<()> {
        return_errno_with_message!(Errno::EPERM, "securityfs does not support rmdir");
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        if is_dot(name) {
            return Ok(self.this());
        }
        if is_dotdot(name) {
            return Ok(self.parent().unwrap_or_else(|| self.this()));
        }

        let Some(entry) = self.lookup_entry(name) else {
            return_errno_with_message!(Errno::ENOENT, "the securityfs file does not exist");
        };

        let this: Arc<dyn Inode> = self.this();
        Ok(SecurityFsInode::new_child(
            entry.kind,
            Arc::downgrade(&this),
        ))
    }

    fn rename(
        &self,
        _old_name: &str,
        _target: &Arc<dyn Inode>,
        _new_name: &str,
        _mode: RenameMode,
    ) -> Result<()> {
        return_errno_with_message!(Errno::EPERM, "securityfs does not support rename");
    }

    fn read_link(&self) -> Result<SymbolicLink> {
        Err(Error::new(Errno::EINVAL))
    }

    fn write_link(&self, _target: &str) -> Result<()> {
        Err(Error::new(Errno::EINVAL))
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        self.fs.upgrade().unwrap()
    }

    fn revalidation_policy(&self) -> RevalidationPolicy {
        match self.kind {
            SecurityFsNodeKind::Root => {
                RevalidationPolicy::REVALIDATE_EXISTS | RevalidationPolicy::REVALIDATE_ABSENT
            }
            _ => RevalidationPolicy::empty(),
        }
    }

    fn revalidate_exists(&self, name: &str, _child: &dyn Inode) -> bool {
        self.lookup_entry(name).is_some()
    }

    fn revalidate_absent(&self, name: &str) -> bool {
        self.lookup_entry(name).is_none()
    }

    fn seek_end(&self) -> Option<usize> {
        (self.kind.type_() == InodeType::Dir).then_some(0)
    }

    fn extension(&self) -> &Extension {
        &self.extension
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SecurityFsNodeKind {
    Root,
    AppArmorDir,
    Profiles,
    Load,
    Replace,
    Remove,
    FeaturesDir,
    FeatureAbi,
    FeaturePolicyDir,
    FeaturePolicyVersionsDir,
    FeaturePolicySetLoad,
    FeaturePolicyPermstable32,
    FeaturePolicyPermstable32Version,
    FeaturePolicyVersionV5,
    FeaturePolicyVersionV6,
    FeaturePolicyVersionV7,
    FeaturePolicyVersionV8,
    FeaturePolicyVersionV9,
    FeatureFileDir,
    FeatureFileMask,
    FeatureCapsDir,
    FeatureCapsMask,
    FeatureDomainDir,
    FeatureDomainChangeProfile,
    FeatureDomainChangeOnexec,
    FeatureDomainVersion,
}

impl SecurityFsNodeKind {
    fn type_(self) -> InodeType {
        match self {
            Self::Root
            | Self::AppArmorDir
            | Self::FeaturesDir
            | Self::FeaturePolicyDir
            | Self::FeaturePolicyVersionsDir
            | Self::FeatureFileDir
            | Self::FeatureCapsDir
            | Self::FeatureDomainDir => InodeType::Dir,
            Self::Profiles
            | Self::Load
            | Self::Replace
            | Self::Remove
            | Self::FeatureAbi
            | Self::FeaturePolicySetLoad
            | Self::FeaturePolicyPermstable32
            | Self::FeaturePolicyPermstable32Version
            | Self::FeaturePolicyVersionV5
            | Self::FeaturePolicyVersionV6
            | Self::FeaturePolicyVersionV7
            | Self::FeaturePolicyVersionV8
            | Self::FeaturePolicyVersionV9
            | Self::FeatureFileMask
            | Self::FeatureCapsMask
            | Self::FeatureDomainChangeProfile
            | Self::FeatureDomainChangeOnexec
            | Self::FeatureDomainVersion => InodeType::File,
        }
    }

    fn mode(self) -> InodeMode {
        match self {
            Self::Root
            | Self::AppArmorDir
            | Self::FeaturesDir
            | Self::FeaturePolicyDir
            | Self::FeaturePolicyVersionsDir
            | Self::FeatureFileDir
            | Self::FeatureCapsDir
            | Self::FeatureDomainDir => mkmod!(a+rx),
            Self::Profiles
            | Self::FeatureAbi
            | Self::FeaturePolicySetLoad
            | Self::FeaturePolicyPermstable32
            | Self::FeaturePolicyPermstable32Version
            | Self::FeaturePolicyVersionV5
            | Self::FeaturePolicyVersionV6
            | Self::FeaturePolicyVersionV7
            | Self::FeaturePolicyVersionV8
            | Self::FeaturePolicyVersionV9
            | Self::FeatureFileMask
            | Self::FeatureCapsMask
            | Self::FeatureDomainChangeProfile
            | Self::FeatureDomainChangeOnexec
            | Self::FeatureDomainVersion => mkmod!(a+r),
            Self::Load | Self::Replace | Self::Remove => mkmod!(u+w),
        }
    }
}

struct SecurityFsEntry {
    name: &'static str,
    kind: SecurityFsNodeKind,
}

impl SecurityFsEntry {
    fn new(name: &'static str, kind: SecurityFsNodeKind) -> Self {
        Self { name, kind }
    }
}

fn require_mac_admin() -> Result<()> {
    let current_thread = current_thread!();
    let Some(posix_thread) = current_thread.as_posix_thread() else {
        return_errno_with_message!(Errno::ESRCH, "the current thread is not a POSIX thread");
    };

    security::capable(
        UserNamespace::get_init_singleton().as_ref(),
        CapSet::MAC_ADMIN,
        posix_thread,
    )
}

fn read_bytes_from(reader: &mut VmReader, max_len: usize) -> Result<(Vec<u8>, usize)> {
    let len = reader.remain();
    if len > max_len {
        return_errno_with_message!(Errno::E2BIG, "the AppArmor policy payload is too large");
    }

    let mut bytes = vec![0; len];
    let mut writer = VmWriter::from(bytes.as_mut_slice()).to_fallible();
    let read_bytes = reader
        .read_fallible(&mut writer)
        .map_err(|(error, _read_bytes)| error)?;
    bytes.truncate(read_bytes);

    Ok((bytes, read_bytes))
}

fn parse_remove_profile_name(bytes: &[u8]) -> Result<&str> {
    if bytes.len() > MAX_REMOVE_TEXT_LEN {
        return_errno_with_message!(Errno::E2BIG, "the AppArmor profile name is too large");
    }

    let profile_name = core::str::from_utf8(bytes)
        .map_err(|_| Error::with_message(Errno::EINVAL, "the profile name is not UTF-8"))?
        .trim();
    if profile_name.is_empty() {
        return_errno_with_message!(Errno::EINVAL, "the AppArmor profile name is empty");
    }

    Ok(profile_name)
}
