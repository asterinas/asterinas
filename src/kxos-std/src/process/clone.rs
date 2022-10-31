use kxos_frame::{
    cpu::CpuContext,
    user::UserSpace,
    vm::{VmIo, VmSpace},
};

use crate::{
    prelude::*,
    process::{new_pid, signal::sig_queues::SigQueues, table, task::create_new_task},
};

use super::Process;

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
    new_sp: Vaddr,
    parent_tidptr: Vaddr,
    child_tidptr: Vaddr,
    tls: usize,
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
        new_sp: Vaddr,
        parent_tidptr: Vaddr,
        child_tidptr: Vaddr,
        tls: usize,
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
    fn contains_unsupported_flags(&self) -> bool {
        self.intersects(!(CloneFlags::CLONE_CHILD_SETTID | CloneFlags::CLONE_CHILD_CLEARTID))
    }
}

/// Clone a child process. Without schedule it to run.
pub fn clone_child(parent_context: CpuContext, clone_args: CloneArgs) -> Result<Arc<Process>> {
    let child_pid = new_pid();
    let current = Process::current();

    // child process vm space
    // FIXME: COPY ON WRITE can be used here
    let parent_vm_space = current
        .vm_space()
        .expect("User task should always have vm space");
    let child_vm_space = parent_vm_space.clone();
    debug_check_clone_vm_space(parent_vm_space, &child_vm_space);

    let child_file_name = match current.filename() {
        None => None,
        Some(filename) => Some(filename.clone()),
    };

    // child process user_vm
    let child_user_vm = match current.user_vm() {
        None => None,
        Some(user_vm) => Some(user_vm.clone()),
    };

    // child process cpu context
    let mut child_cpu_context = parent_context.clone();
    debug!("parent context: {:x?}", parent_context);
    debug!("parent gp_regs: {:x?}", child_cpu_context.gp_regs);
    child_cpu_context.gp_regs.rax = 0; // Set return value of child process

    let child_user_space = Arc::new(UserSpace::new(child_vm_space, child_cpu_context));
    debug!("before spawn child task");
    debug!("current pid: {}", current.pid());
    debug!("child process pid: {}", child_pid);
    debug!("rip = 0x{:x}", child_cpu_context.gp_regs.rip);

    // inherit parent's sig disposition
    let child_sig_dispositions = current.sig_dispositions().lock().clone();
    // sig queue is set empty
    let child_sig_queues = SigQueues::new();
    // inherit parent's sig mask
    let child_sig_mask = current.sig_mask().lock().clone();

    let child = Arc::new_cyclic(|child_process_ref| {
        let weak_child_process = child_process_ref.clone();
        let child_task = create_new_task(child_user_space.clone(), weak_child_process);
        Process::new(
            child_pid,
            child_task,
            child_file_name,
            child_user_vm,
            Some(child_user_space),
            None,
            child_sig_dispositions,
            child_sig_queues,
            child_sig_mask,
        )
    });
    // Inherit parent's process group
    let parent_process_group = current
        .process_group()
        .lock()
        .as_ref()
        .map(|ppgrp| ppgrp.upgrade())
        .flatten()
        .unwrap();
    parent_process_group.add_process(child.clone());
    child.set_process_group(Arc::downgrade(&parent_process_group));

    Process::current().add_child(child.clone());
    table::add_process(child_pid, child.clone());
    deal_with_clone_args(clone_args, &child)?;
    Ok(child)
}

fn deal_with_clone_args(clone_args: CloneArgs, child_process: &Arc<Process>) -> Result<()> {
    let clone_flags = clone_args.clone_flags;
    if clone_flags.contains_unsupported_flags() {
        panic!("Found unsupported clone flags: {:?}", clone_flags);
    }
    if clone_flags.contains(CloneFlags::CLONE_CHILD_CLEARTID) {
        clone_child_clear_tid(child_process)?;
    }
    if clone_flags.contains(CloneFlags::CLONE_CHILD_SETTID) {
        clone_child_set_tid(child_process, clone_args)?;
    }
    Ok(())
}

fn clone_child_clear_tid(child_process: &Arc<Process>) -> Result<()> {
    warn!("clone_child_clear_tid does nothing now");
    Ok(())
}

fn clone_child_set_tid(child_process: &Arc<Process>, clone_args: CloneArgs) -> Result<()> {
    debug!("clone child set tid");
    let child_pid = child_process.pid();
    let child_vm = child_process
        .vm_space()
        .ok_or_else(|| Error::new(Errno::ECHILD))?;
    child_vm.write_val(clone_args.child_tidptr, &child_pid)?;
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
