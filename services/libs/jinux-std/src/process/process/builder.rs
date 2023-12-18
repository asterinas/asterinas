use crate::fs::file_table::FileTable;
use crate::fs::fs_resolver::FsResolver;
use crate::fs::utils::FileCreationMask;
use crate::process::posix_thread::{PosixThreadBuilder, PosixThreadExt};
use crate::process::process_vm::ProcessVm;
use crate::process::rlimit::ResourceLimits;
use crate::process::signal::sig_disposition::SigDispositions;
use crate::thread::Thread;

use super::{Pid, Process};
use crate::prelude::*;

pub struct ProcessBuilder<'a> {
    // Essential parts
    pid: Pid,
    executable_path: &'a str,
    parent: Weak<Process>,

    // Optional parts
    main_thread_builder: Option<PosixThreadBuilder>,
    argv: Option<Vec<CString>>,
    envp: Option<Vec<CString>>,
    process_vm: Option<ProcessVm>,
    file_table: Option<Arc<Mutex<FileTable>>>,
    fs: Option<Arc<RwLock<FsResolver>>>,
    umask: Option<Arc<RwLock<FileCreationMask>>>,
    resource_limits: Option<ResourceLimits>,
    sig_dispositions: Option<Arc<Mutex<SigDispositions>>>,
}

impl<'a> ProcessBuilder<'a> {
    pub fn new(pid: Pid, executable_path: &'a str, parent: Weak<Process>) -> Self {
        ProcessBuilder {
            pid,
            executable_path,
            parent,
            main_thread_builder: None,
            argv: None,
            envp: None,
            process_vm: None,
            file_table: None,
            fs: None,
            umask: None,
            resource_limits: None,
            sig_dispositions: None,
        }
    }

    pub fn main_thread_builder(&mut self, builder: PosixThreadBuilder) -> &mut Self {
        self.main_thread_builder = Some(builder);
        self
    }

    pub fn process_vm(&mut self, process_vm: ProcessVm) -> &mut Self {
        self.process_vm = Some(process_vm);
        self
    }

    pub fn file_table(&mut self, file_table: Arc<Mutex<FileTable>>) -> &mut Self {
        self.file_table = Some(file_table);
        self
    }

    pub fn fs(&mut self, fs: Arc<RwLock<FsResolver>>) -> &mut Self {
        self.fs = Some(fs);
        self
    }

    pub fn umask(&mut self, umask: Arc<RwLock<FileCreationMask>>) -> &mut Self {
        self.umask = Some(umask);
        self
    }

    pub fn resource_limits(&mut self, resource_limits: ResourceLimits) -> &mut Self {
        self.resource_limits = Some(resource_limits);
        self
    }

    pub fn sig_dispositions(&mut self, sig_dispositions: Arc<Mutex<SigDispositions>>) -> &mut Self {
        self.sig_dispositions = Some(sig_dispositions);
        self
    }

    pub fn argv(&mut self, argv: Vec<CString>) -> &mut Self {
        self.argv = Some(argv);
        self
    }

    pub fn envp(&mut self, envp: Vec<CString>) -> &mut Self {
        self.envp = Some(envp);
        self
    }

    fn check_build(&self) -> Result<()> {
        if self.main_thread_builder.is_some() {
            debug_assert!(self.parent.upgrade().is_some());
            debug_assert!(self.argv.is_none());
            debug_assert!(self.envp.is_none());
        }

        if self.main_thread_builder.is_none() {
            debug_assert!(self.parent.upgrade().is_none());
            debug_assert!(self.argv.is_some());
            debug_assert!(self.envp.is_some());
        }

        Ok(())
    }

    pub fn build(self) -> Result<Arc<Process>> {
        self.check_build()?;
        let Self {
            pid,
            executable_path,
            parent,
            main_thread_builder,
            argv,
            envp,
            process_vm,
            file_table,
            fs,
            umask,
            resource_limits,
            sig_dispositions,
        } = self;

        let process_vm = process_vm.or_else(|| Some(ProcessVm::alloc())).unwrap();

        let file_table = file_table
            .or_else(|| Some(Arc::new(Mutex::new(FileTable::new_with_stdio()))))
            .unwrap();

        let fs = fs
            .or_else(|| Some(Arc::new(RwLock::new(FsResolver::new()))))
            .unwrap();

        let umask = umask
            .or_else(|| Some(Arc::new(RwLock::new(FileCreationMask::default()))))
            .unwrap();

        let resource_limits = resource_limits
            .or_else(|| Some(ResourceLimits::default()))
            .unwrap();

        let sig_dispositions = sig_dispositions
            .or_else(|| Some(Arc::new(Mutex::new(SigDispositions::new()))))
            .unwrap();

        let process = {
            let threads = Vec::new();
            Arc::new(Process::new(
                pid,
                parent,
                threads,
                executable_path.to_string(),
                process_vm,
                file_table,
                fs,
                umask,
                sig_dispositions,
                resource_limits,
            ))
        };

        let thread = if let Some(thread_builder) = main_thread_builder {
            let builder = thread_builder.process(Arc::downgrade(&process));
            builder.build()
        } else {
            Thread::new_posix_thread_from_executable(
                pid,
                process.vm(),
                &process.fs().read(),
                executable_path,
                Arc::downgrade(&process),
                argv.unwrap(),
                envp.unwrap(),
            )?
        };

        process.threads().lock().push(thread);

        process.set_runnable();

        Ok(process)
    }
}
