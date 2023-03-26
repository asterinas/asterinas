use jinux_frame::{
    cpu::CpuContext,
    user::UserSpace,
    vm::{VmIo, VmSpace},
};

use crate::{
    current_thread,
    fs::file_table::FileTable,
    fs::{fs_resolver::FsResolver, utils::FileCreationMask},
    prelude::*,
    process::{
        posix_thread::{
            builder::PosixThreadBuilder, name::ThreadName, posix_thread_ext::PosixThreadExt,
        },
        process_table,
    },
    rights::Full,
    thread::{allocate_tid, thread_table, Thread, Tid},
    util::write_val_to_user,
    vm::vmar::Vmar,
};

use super::{posix_thread::PosixThread, signal::sig_disposition::SigDispositions, Process};

bitflags! {
    pub struct CloneFlags: u32 {
        const CLONE_VM      = 0x00000100;       /* Set if VM shared between processes.  */
        const CLONE_FS      = 0x00000200;       /* Set if fs info shared between processes.  */
        const CLONE_FILES   = 0x00000400;       /* Set if open files shared between processes.  */
        const CLONE_SIGHAND = 0x00000800;       /* Set if signal handlers shared.  */
        const CLONE_PIDFD   = 0x00001000;       /* Set if a pidfd should be placed in parent.  */
        const CLONE_PTRACE  = 0x00002000;       /* Set if tracing continues on the child.  */
        const CLONE_VFORK   = 0x00004000;       /* Set if the parent wants the child to wake it up on mm_release.  */
        const CLONE_PARENT  = 0x00008000;       /* Set if we want to have the same parent as the cloner.  */
        const CLONE_THREAD  = 0x00010000;       /* Set to add to same thread group.  */
        const CLONE_NEWNS   = 0x00020000;       /* Set to create new namespace.  */
        const CLONE_SYSVSEM = 0x00040000;       /* Set to shared SVID SEM_UNDO semantics.  */
        const CLONE_SETTLS  = 0x00080000;       /* Set TLS info.  */
        const CLONE_PARENT_SETTID = 0x00100000; /* Store TID in userlevel buffer before MM copy.  */
        const CLONE_CHILD_CLEARTID = 0x00200000;/* Register exit futex and memory location to clear.  */
        const CLONE_DETACHED = 0x00400000;      /* Create clone detached.  */
        const CLONE_UNTRACED = 0x00800000;      /* Set if the tracing process can't force CLONE_PTRACE on this clone.  */
        const CLONE_CHILD_SETTID = 0x01000000;  /* Store TID in userlevel buffer in the child.  */
        const CLONE_NEWCGROUP   = 0x02000000;	/* New cgroup namespace.  */
        const CLONE_NEWUTS	= 0x04000000;	    /* New utsname group.  */
        const CLONE_NEWIPC	= 0x08000000;	    /* New ipcs.  */
        const CLONE_NEWUSER	= 0x10000000;	    /* New user namespace.  */
        const CLONE_NEWPID	= 0x20000000;	    /* New pid namespace.  */
        const CLONE_NEWNET	= 0x40000000;	    /* New network namespace.  */
        const CLONE_IO	= 0x80000000;	        /* Clone I/O context.  */
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CloneArgs {
    new_sp: u64,
    parent_tidptr: Vaddr,
    child_tidptr: Vaddr,
    tls: u64,
    clone_flags: CloneFlags,
}

impl CloneArgs {
    pub const fn default() -> Self {
        CloneArgs {
            new_sp: 0,
            parent_tidptr: 0,
            child_tidptr: 0,
            tls: 0,
            clone_flags: CloneFlags::empty(),
        }
    }

    pub const fn new(
        new_sp: u64,
        parent_tidptr: Vaddr,
        child_tidptr: Vaddr,
        tls: u64,
        clone_flags: CloneFlags,
    ) -> Self {
        CloneArgs {
            new_sp,
            parent_tidptr,
            child_tidptr,
            tls,
            clone_flags,
        }
    }
}

impl From<u64> for CloneFlags {
    fn from(flags: u64) -> Self {
        // We use the lower 32 bits
        let clone_flags = (flags & 0xffff_ffff) as u32;
        CloneFlags::from_bits_truncate(clone_flags)
    }
}

impl CloneFlags {
    fn check_unsupported_flags(&self) -> Result<()> {
        let supported_flags = CloneFlags::CLONE_VM
            | CloneFlags::CLONE_FS
            | CloneFlags::CLONE_FILES
            | CloneFlags::CLONE_SIGHAND
            | CloneFlags::CLONE_THREAD
            | CloneFlags::CLONE_SYSVSEM
            | CloneFlags::CLONE_SETTLS
            | CloneFlags::CLONE_PARENT_SETTID
            | CloneFlags::CLONE_CHILD_SETTID
            | CloneFlags::CLONE_CHILD_CLEARTID;
        let unsupported_flags = *self - supported_flags;
        if !unsupported_flags.is_empty() {
            panic!("contains unsupported clone flags: {:?}", unsupported_flags);
        }
        Ok(())
    }
}

/// Clone a child thread. Without schedule it to run.
pub fn clone_child(parent_context: CpuContext, clone_args: CloneArgs) -> Result<Tid> {
    clone_args.clone_flags.check_unsupported_flags()?;
    if clone_args.clone_flags.contains(CloneFlags::CLONE_THREAD) {
        let child_thread = clone_child_thread(parent_context, clone_args)?;
        let child_tid = child_thread.tid();
        debug!(
            "*********schedule child thread, current tid = {}, child pid = {}**********",
            current_thread!().tid(),
            child_tid
        );
        child_thread.run();
        debug!(
            "*********return to parent thread, current tid = {}, child pid = {}*********",
            current_thread!().tid(),
            child_tid
        );
        Ok(child_tid)
    } else {
        let child_process = clone_child_process(parent_context, clone_args)?;
        let child_pid = child_process.pid();
        debug!(
            "*********schedule child process, current pid = {}, child pid = {}**********",
            current!().pid(),
            child_pid
        );
        child_process.run();
        debug!(
            "*********return to parent process, current pid = {}, child pid = {}*********",
            current!().pid(),
            child_pid
        );
        Ok(child_pid)
    }
}

fn clone_child_thread(parent_context: CpuContext, clone_args: CloneArgs) -> Result<Arc<Thread>> {
    let clone_flags = clone_args.clone_flags;
    let current = current!();
    debug_assert!(clone_flags.contains(CloneFlags::CLONE_VM));
    debug_assert!(clone_flags.contains(CloneFlags::CLONE_FILES));
    debug_assert!(clone_flags.contains(CloneFlags::CLONE_SIGHAND));
    let child_root_vmar = current.root_vmar();
    let child_vm_space = child_root_vmar.vm_space().clone();
    let child_cpu_context = clone_cpu_context(
        parent_context,
        clone_args.new_sp,
        clone_args.tls,
        clone_flags,
    );
    let child_user_space = Arc::new(UserSpace::new(child_vm_space, child_cpu_context));
    clone_sysvsem(clone_flags)?;

    let child_tid = allocate_tid();
    // inherit sigmask from current thread
    let current_thread = current_thread!();
    let current_posix_thread = current_thread.as_posix_thread().unwrap();
    let sig_mask = current_posix_thread.sig_mask().lock().clone();
    let is_main_thread = child_tid == current.pid();
    let thread_builder = PosixThreadBuilder::new(child_tid, child_user_space)
        .process(Arc::downgrade(&current))
        .is_main_thread(is_main_thread);
    let child_thread = thread_builder.build();
    current.threads.lock().push(child_thread.clone());
    let child_posix_thread = child_thread.as_posix_thread().unwrap();
    clone_parent_settid(child_tid, clone_args.parent_tidptr, clone_flags)?;
    clone_child_cleartid(child_posix_thread, clone_args.child_tidptr, clone_flags)?;
    clone_child_settid(
        child_root_vmar,
        child_tid,
        clone_args.child_tidptr,
        clone_flags,
    )?;
    Ok(child_thread)
}

fn clone_child_process(parent_context: CpuContext, clone_args: CloneArgs) -> Result<Arc<Process>> {
    let current = current!();
    let parent = Arc::downgrade(&current);
    let clone_flags = clone_args.clone_flags;

    // clone vm
    let parent_root_vmar = current.root_vmar();
    let child_root_vmar = clone_vm(parent_root_vmar, clone_flags)?;
    let child_user_vm = current.user_vm().clone();

    // clone user space
    let child_cpu_context = clone_cpu_context(
        parent_context,
        clone_args.new_sp,
        clone_args.tls,
        clone_flags,
    );
    let child_vm_space = child_root_vmar.vm_space().clone();
    let child_user_space = Arc::new(UserSpace::new(child_vm_space, child_cpu_context));

    // clone file table
    let child_file_table = clone_files(current.file_table(), clone_flags);
    // clone fs
    let child_fs = clone_fs(current.fs(), clone_flags);
    // clone umask
    let parent_umask = current.umask.read().get();
    let child_umask = Arc::new(RwLock::new(FileCreationMask::new(parent_umask)));
    // clone sig dispositions
    let child_sig_dispositions = clone_sighand(current.sig_dispositions(), clone_flags);
    // clone system V semaphore
    clone_sysvsem(clone_flags)?;

    let child_elf_path = current.executable_path().read().clone();
    let child_thread_name = ThreadName::new_from_executable_path(&child_elf_path)?;

    // inherit parent's sig mask
    let current_thread = current_thread!();
    let posix_thread = current_thread.as_posix_thread().unwrap();
    let child_sig_mask = posix_thread.sig_mask().lock().clone();

    let child_tid = allocate_tid();
    let mut child_thread_builder = PosixThreadBuilder::new(child_tid, child_user_space)
        .thread_name(Some(child_thread_name))
        .sig_mask(child_sig_mask);

    let child = Arc::new_cyclic(|child_process_ref| {
        let weak_child_process = child_process_ref.clone();
        let child_pid = child_tid;
        child_thread_builder = child_thread_builder.process(weak_child_process);
        let child_thread = child_thread_builder.build();
        Process::new(
            child_pid,
            parent,
            vec![child_thread],
            child_elf_path,
            child_user_vm,
            child_root_vmar.clone(),
            Weak::new(),
            child_file_table,
            child_fs,
            child_umask,
            child_sig_dispositions,
        )
    });
    // Inherit parent's process group
    let parent_process_group = current.process_group().lock().upgrade().unwrap();
    parent_process_group.add_process(child.clone());
    child.set_process_group(Arc::downgrade(&parent_process_group));

    current!().add_child(child.clone());
    process_table::add_process(child.clone());

    let child_thread = thread_table::tid_to_thread(child_tid).unwrap();
    let child_posix_thread = child_thread.as_posix_thread().unwrap();
    clone_parent_settid(child_tid, clone_args.parent_tidptr, clone_flags)?;
    clone_child_cleartid(child_posix_thread, clone_args.child_tidptr, clone_flags)?;
    clone_child_settid(
        &child_root_vmar,
        child_tid,
        clone_args.child_tidptr,
        clone_flags,
    )?;
    Ok(child)
}

fn clone_child_cleartid(
    child_posix_thread: &PosixThread,
    child_tidptr: Vaddr,
    clone_flags: CloneFlags,
) -> Result<()> {
    if clone_flags.contains(CloneFlags::CLONE_CHILD_CLEARTID) {
        let mut clear_tid = child_posix_thread.clear_child_tid().lock();
        *clear_tid = child_tidptr;
    }
    Ok(())
}

fn clone_child_settid(
    child_root_vmar: &Vmar<Full>,
    child_tid: Tid,
    child_tidptr: Vaddr,
    clone_flags: CloneFlags,
) -> Result<()> {
    if clone_flags.contains(CloneFlags::CLONE_CHILD_SETTID) {
        child_root_vmar.write_val(child_tidptr, &child_tid)?;
    }
    Ok(())
}

fn clone_parent_settid(
    child_tid: Tid,
    parent_tidptr: Vaddr,
    clone_flags: CloneFlags,
) -> Result<()> {
    if clone_flags.contains(CloneFlags::CLONE_PARENT_SETTID) {
        write_val_to_user(parent_tidptr, &child_tid)?;
    }
    Ok(())
}

/// clone child vmar. If CLONE_VM is set, both threads share the same root vmar.
/// Otherwise, fork a new copy-on-write vmar.
fn clone_vm(
    parent_root_vmar: &Arc<Vmar<Full>>,
    clone_flags: CloneFlags,
) -> Result<Arc<Vmar<Full>>> {
    if clone_flags.contains(CloneFlags::CLONE_VM) {
        Ok(parent_root_vmar.clone())
    } else {
        Ok(Arc::new(parent_root_vmar.fork_vmar()?))
    }
}

fn clone_cpu_context(
    parent_context: CpuContext,
    new_sp: u64,
    tls: u64,
    clone_flags: CloneFlags,
) -> CpuContext {
    let mut child_context = parent_context.clone();
    // The return value of child thread is zero
    child_context.set_rax(0);

    if clone_flags.contains(CloneFlags::CLONE_VM) {
        // if parent and child shares the same address space, a new stack must be specified.
        debug_assert!(new_sp != 0);
    }
    if new_sp != 0 {
        child_context.set_rsp(new_sp as usize);
    }
    if clone_flags.contains(CloneFlags::CLONE_SETTLS) {
        // x86_64 specific: TLS is the fsbase register
        child_context.set_fsbase(tls as usize);
    }

    child_context
}

fn clone_fs(
    parent_fs: &Arc<RwLock<FsResolver>>,
    clone_flags: CloneFlags,
) -> Arc<RwLock<FsResolver>> {
    if clone_flags.contains(CloneFlags::CLONE_FS) {
        parent_fs.clone()
    } else {
        Arc::new(RwLock::new(parent_fs.read().clone()))
    }
}

fn clone_files(
    parent_file_table: &Arc<Mutex<FileTable>>,
    clone_flags: CloneFlags,
) -> Arc<Mutex<FileTable>> {
    // if CLONE_FILES is set, the child and parent shares the same file table
    // Otherwise, the child will deep copy a new file table.
    // FIXME: the clone may not be deep copy.
    if clone_flags.contains(CloneFlags::CLONE_FILES) {
        parent_file_table.clone()
    } else {
        Arc::new(Mutex::new(parent_file_table.lock().clone()))
    }
}

fn clone_sighand(
    parent_sig_dispositions: &Arc<Mutex<SigDispositions>>,
    clone_flags: CloneFlags,
) -> Arc<Mutex<SigDispositions>> {
    // similer to CLONE_FILES
    if clone_flags.contains(CloneFlags::CLONE_SIGHAND) {
        parent_sig_dispositions.clone()
    } else {
        Arc::new(Mutex::new(parent_sig_dispositions.lock().clone()))
    }
}

fn clone_sysvsem(clone_flags: CloneFlags) -> Result<()> {
    if clone_flags.contains(CloneFlags::CLONE_SYSVSEM) {
        warn!("CLONE_SYSVSEM is not supported now");
    }
    Ok(())
}

/// debug use. check clone vm space corrent.
fn debug_check_clone_vm_space(parent_vm_space: &VmSpace, child_vm_space: &VmSpace) {
    let mut buffer1 = vec![0u8; 0x78];
    let mut buffer2 = vec![0u8; 0x78];
    parent_vm_space
        .read_bytes(0x401000, &mut buffer1)
        .expect("read buffer1 failed");
    child_vm_space
        .read_bytes(0x401000, &mut buffer2)
        .expect("read buffer1 failed");
    for len in 0..buffer1.len() {
        assert_eq!(buffer1[len], buffer2[len]);
    }
    debug!("check clone vm space succeed.");
}
