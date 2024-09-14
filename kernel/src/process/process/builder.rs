// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

use super::{Pid, Process};
use crate::{
    fs::{file_table::FileTable, fs_resolver::FsResolver, utils::FileCreationMask},
    prelude::*,
    process::{
        posix_thread::{create_posix_task_from_executable, PosixThreadBuilder},
        process_vm::ProcessVm,
        rlimit::ResourceLimits,
        signal::sig_disposition::SigDispositions,
        Credentials,
    },
    sched::priority::Nice,
};

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
    file_table: Option<Arc<SpinLock<FileTable>>>,
    fs: Option<Arc<RwMutex<FsResolver>>>,
    umask: Option<Arc<RwLock<FileCreationMask>>>,
    resource_limits: Option<ResourceLimits>,
    sig_dispositions: Option<Arc<Mutex<SigDispositions>>>,
    credentials: Option<Credentials>,
    nice: Option<Nice>,
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
            credentials: None,
            nice: None,
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

    pub fn file_table(&mut self, file_table: Arc<SpinLock<FileTable>>) -> &mut Self {
        self.file_table = Some(file_table);
        self
    }

    pub fn fs(&mut self, fs: Arc<RwMutex<FsResolver>>) -> &mut Self {
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

    pub fn credentials(&mut self, credentials: Credentials) -> &mut Self {
        self.credentials = Some(credentials);
        self
    }

    pub fn nice(&mut self, nice: Nice) -> &mut Self {
        self.nice = Some(nice);
        self
    }

    fn check_build(&self) -> Result<()> {
        if self.main_thread_builder.is_some() {
            debug_assert!(self.parent.upgrade().is_some());
            debug_assert!(self.argv.is_none());
            debug_assert!(self.envp.is_none());
            debug_assert!(self.credentials.is_none());
        }

        if self.main_thread_builder.is_none() {
            debug_assert!(self.parent.upgrade().is_none());
            debug_assert!(self.argv.is_some());
            debug_assert!(self.envp.is_some());
            debug_assert!(self.credentials.is_some());
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
            credentials,
            nice,
        } = self;

        let process_vm = process_vm.or_else(|| Some(ProcessVm::alloc())).unwrap();

        let file_table = file_table
            .or_else(|| Some(Arc::new(SpinLock::new(FileTable::new_with_stdio()))))
            .unwrap();

        let fs = fs
            .or_else(|| Some(Arc::new(RwMutex::new(FsResolver::new()))))
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

        let nice = nice.or_else(|| Some(Nice::default())).unwrap();

        let process = {
            let threads = Vec::new();
            Process::new(
                pid,
                parent,
                threads,
                executable_path.to_string(),
                process_vm,
                fs,
                file_table,
                umask,
                resource_limits,
                nice,
                sig_dispositions,
            )
        };

        let task = if let Some(thread_builder) = main_thread_builder {
            let builder = thread_builder.process(Arc::downgrade(&process));
            builder.build()
        } else {
            create_posix_task_from_executable(
                pid,
                credentials.unwrap(),
                process.vm(),
                &process.fs().read(),
                executable_path,
                Arc::downgrade(&process),
                argv.unwrap(),
                envp.unwrap(),
            )?
        };

        process.tasks().lock().push(task);

        process.set_runnable();

        Ok(process)
    }
}
