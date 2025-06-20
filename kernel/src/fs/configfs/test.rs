// SPDX-License-Identifier: MPL-2.0

use alloc::{
    string::ToString,
    sync::{Arc, Weak},
};
use core::{
    fmt::Debug,
    sync::atomic::{AtomicU32, Ordering},
};

use aster_systree::{
    impl_cast_methods_for_branch, impl_cast_methods_for_node, Error, Result, SysAttrSet,
    SysAttrSetBuilder, SysBranchNode, SysBranchNodeFields, SysMode, SysNode, SysNodeId,
    SysNodeType, SysNormalNodeFields, SysObj, SysStr,
};
use inherit_methods_macro::inherit_methods;
use ostd::{
    mm::{VmReader, VmWriter},
    prelude::ktest,
    Pod,
};
use spin::Once;

use crate::{
    fs::utils::{FileSystem, InodeMode, InodeType},
    time::clocks::init_for_ktest as time_init_for_ktest,
};

/// The mock process subsystem in configfs.
#[derive(Debug)]
struct MockProcessSystem {
    fields: SysBranchNodeFields<dyn SysObj>,
    weak_self: Weak<Self>,
}

impl MockProcessSystem {
    fn new() -> Arc<Self> {
        let name = SysStr::from("process_system");
        let attrs = SysAttrSet::new_empty();
        let fields = SysBranchNodeFields::new(name, attrs);

        Arc::new_cyclic(|weak_self| MockProcessSystem {
            fields,
            weak_self: weak_self.clone(),
        })
    }
}

#[inherit_methods(from = "self.fields")]
impl SysObj for MockProcessSystem {
    impl_cast_methods_for_branch!();

    fn id(&self) -> &SysNodeId;

    fn name(&self) -> &SysStr;

    fn set_parent_path(&self, path: SysStr);

    fn path(&self) -> SysStr;
}

impl SysNode for MockProcessSystem {
    fn node_attrs(&self) -> &SysAttrSet {
        self.fields.attr_set()
    }

    fn read_attr(&self, _name: &str, _writer: &mut VmWriter) -> Result<usize> {
        Err(Error::AttributeError)
    }

    fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> Result<usize> {
        Err(Error::AttributeError)
    }

    fn mode(&self) -> SysMode {
        SysMode::DEFAULT_RW_MODE
    }
}

#[inherit_methods(from = "self.fields")]
impl SysBranchNode for MockProcessSystem {
    fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&Arc<dyn SysObj>>));

    fn visit_children_with(&self, _min_id: u64, f: &mut dyn FnMut(&Arc<dyn SysObj>) -> Option<()>);

    fn child(&self, name: &str) -> Option<Arc<dyn SysObj>>;

    fn add_child(&self, new_child: Arc<dyn SysObj>) -> Result<()>;

    fn create_child(&self, name: &str) -> Result<Arc<dyn SysObj>> {
        let process = MockProcess::new(SysStr::from(name.to_string()));
        self.add_child(process.clone())?;
        Ok(process)
    }
}

/// A mock process in mock process subsystem.
///
/// This kind of `SysNode` can be created by executing `mkdir [process_name]`
/// in mock process subsystem.
#[derive(Debug)]
struct MockProcess {
    fields: SysBranchNodeFields<dyn SysObj>,
    priority: AtomicU32,
    weak_self: Weak<Self>,
}

impl MockProcess {
    fn new(name: SysStr) -> Arc<Self> {
        let mut builder = SysAttrSetBuilder::new();
        builder.add(SysStr::from("priority"), SysMode::DEFAULT_RW_ATTR_MODE);
        let attrs = builder.build().expect("Failed to build attribute set");
        let fields = SysBranchNodeFields::new(name, attrs);
        fields
            .add_child(MockThreads::new() as Arc<dyn SysObj>)
            .expect("Failed to add threads module");

        Arc::new_cyclic(|weak_self| MockProcess {
            fields,
            priority: AtomicU32::new(0),
            weak_self: weak_self.clone(),
        })
    }
}

#[inherit_methods(from = "self.fields")]
impl SysObj for MockProcess {
    impl_cast_methods_for_branch!();

    fn id(&self) -> &SysNodeId;

    fn name(&self) -> &SysStr;

    fn set_parent_path(&self, path: SysStr);

    fn path(&self) -> SysStr;
}

impl SysNode for MockProcess {
    fn node_attrs(&self) -> &SysAttrSet {
        self.fields.attr_set()
    }

    fn read_attr(&self, name: &str, writer: &mut VmWriter) -> Result<usize> {
        if name == "priority" {
            writer
                .write_val(&self.priority.load(Ordering::Relaxed))
                .unwrap();
            return Ok(size_of::<u32>());
        }

        Err(Error::AttributeError)
    }

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> Result<usize> {
        if name == "priority" {
            let new_priority = reader.read_val::<u32>().unwrap();
            self.priority.store(new_priority, Ordering::Relaxed);
            return Ok(size_of::<u32>());
        }

        Err(Error::AttributeError)
    }

    fn mode(&self) -> SysMode {
        SysMode::DEFAULT_RW_MODE
    }
}

#[inherit_methods(from = "self.fields")]
impl SysBranchNode for MockProcess {
    fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&Arc<dyn SysObj>>));

    fn visit_children_with(&self, _min_id: u64, f: &mut dyn FnMut(&Arc<dyn SysObj>) -> Option<()>);

    fn child(&self, name: &str) -> Option<Arc<dyn SysObj>>;

    fn add_child(&self, new_child: Arc<dyn SysObj>) -> Result<()>;
}

/// The threads module in a mock process.
#[derive(Debug)]
struct MockThreads {
    fields: SysBranchNodeFields<dyn SysObj>,
    weak_self: Weak<Self>,
}

impl MockThreads {
    fn new() -> Arc<Self> {
        let name = SysStr::from("threads");
        let attrs = SysAttrSet::new_empty();
        let fields = SysBranchNodeFields::new(name, attrs);

        Arc::new_cyclic(|weak_self| MockThreads {
            fields,
            weak_self: weak_self.clone(),
        })
    }
}

#[inherit_methods(from = "self.fields")]
impl SysObj for MockThreads {
    impl_cast_methods_for_branch!();

    fn id(&self) -> &SysNodeId;

    fn name(&self) -> &SysStr;

    fn set_parent_path(&self, path: SysStr);

    fn path(&self) -> SysStr;
}

impl SysNode for MockThreads {
    fn node_attrs(&self) -> &SysAttrSet {
        self.fields.attr_set()
    }

    fn read_attr(&self, _name: &str, _writer: &mut VmWriter) -> Result<usize> {
        Err(Error::AttributeError)
    }

    fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> Result<usize> {
        Err(Error::AttributeError)
    }

    fn mode(&self) -> SysMode {
        SysMode::DEFAULT_RW_MODE
    }
}

#[inherit_methods(from = "self.fields")]
impl SysBranchNode for MockThreads {
    fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&Arc<dyn SysObj>>));

    fn visit_children_with(&self, _min_id: u64, f: &mut dyn FnMut(&Arc<dyn SysObj>) -> Option<()>);

    fn child(&self, name: &str) -> Option<Arc<dyn SysObj>>;

    fn add_child(&self, new_child: Arc<dyn SysObj>) -> Result<()>;

    fn create_child(&self, name: &str) -> Result<Arc<dyn SysObj>> {
        let thread = MockThread::new(SysStr::from(name.to_string()));
        self.add_child(thread.clone())?;
        Ok(thread)
    }
}

/// A thread instance in the threads module of a mock process.
#[derive(Debug)]
struct MockThread {
    fields: SysNormalNodeFields,
    priority: AtomicU32,
    weak_self: Weak<Self>,
}

impl MockThread {
    fn new(name: SysStr) -> Arc<Self> {
        let mut builder = SysAttrSetBuilder::new();
        builder.add(SysStr::from("priority"), SysMode::DEFAULT_RW_ATTR_MODE);
        let attrs = builder.build().expect("Failed to build attribute set");
        let fields = SysNormalNodeFields::new(name, attrs);

        Arc::new_cyclic(|weak_self| MockThread {
            fields,
            priority: AtomicU32::new(0),
            weak_self: weak_self.clone(),
        })
    }
}

#[inherit_methods(from = "self.fields")]
impl SysObj for MockThread {
    impl_cast_methods_for_node!();

    fn id(&self) -> &SysNodeId;

    fn name(&self) -> &SysStr;

    fn set_parent_path(&self, path: SysStr);

    fn path(&self) -> SysStr;
}

impl SysNode for MockThread {
    fn node_attrs(&self) -> &SysAttrSet {
        self.fields.attr_set()
    }

    fn read_attr(&self, name: &str, writer: &mut VmWriter) -> Result<usize> {
        if name == "priority" {
            writer
                .write_val(&self.priority.load(Ordering::Relaxed))
                .unwrap();
            return Ok(size_of::<u32>());
        }

        Err(Error::AttributeError)
    }

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> Result<usize> {
        if name == "priority" {
            let new_priority = reader.read_val::<u32>().unwrap();
            self.priority.store(new_priority, Ordering::Relaxed);
            return Ok(size_of::<u32>());
        }

        Err(Error::AttributeError)
    }

    fn mode(&self) -> SysMode {
        SysMode::DEFAULT_RW_MODE
    }
}

// --- Test Setup ---

static MOCK_PROCESS_SUBSYETM: Once<Arc<MockProcessSystem>> = Once::new();

fn init_mock_process_subsystem() {
    if MOCK_PROCESS_SUBSYETM.is_completed() {
        return;
    }

    time_init_for_ktest();
    super::init_for_ktest();

    let mock_process_system = MockProcessSystem::new();
    MOCK_PROCESS_SUBSYETM.call_once(|| mock_process_system.clone());
    super::register_subsystem(mock_process_system);
}

#[ktest]
fn test_config_fs() {
    init_mock_process_subsystem();
    let config_fs = super::singleton();
    // path: /sys/kernel/config
    let root_inode = config_fs.root_inode();
    // path: /sys/kernel/config/process_system
    let mock_process_system = root_inode
        .lookup("process_system")
        .expect("lookup process_system failed");
    // path: /sys/kernel/config/process_system/process_1
    let process_1 = mock_process_system
        .create(
            "process_1",
            InodeType::Dir,
            InodeMode::from_bits_truncate(0o755),
        )
        .expect("creating child fails");
    // path: /sys/kernel/config/process_system/process_1/threads
    let threads = process_1.lookup("threads").expect("lookup threads failed");
    // path: /sys/kernel/config/process_system/process_1/threads/thread_0
    let thread = threads
        .create(
            "thread_0",
            InodeType::Dir,
            InodeMode::from_bits_truncate(0o755),
        )
        .expect("creating child fails");

    // R/W Attributes
    let process_priority = process_1
        .lookup("priority")
        .expect("lookup process priority failed");
    let thread_priority = thread
        .lookup("priority")
        .expect("lookup thread priority failed");

    let mut read_buffer: u32 = 10;
    let write_buffer: u32 = 10;
    // The original process priority is 0;
    assert!(process_priority
        .read_bytes_at(0, read_buffer.as_bytes_mut())
        .is_ok());
    assert_eq!(read_buffer, 0);
    // Set the process priority to 10;
    assert!(process_priority
        .write_bytes_at(0, write_buffer.as_bytes())
        .is_ok());
    assert!(process_priority
        .read_bytes_at(0, read_buffer.as_bytes_mut())
        .is_ok());
    assert_eq!(read_buffer, 10);
    // The original thread priority is 0;
    assert!(thread_priority
        .read_bytes_at(0, read_buffer.as_bytes_mut())
        .is_ok());
    assert_eq!(read_buffer, 0);
    // Set the thread priority to 10;
    assert!(thread_priority
        .write_bytes_at(0, write_buffer.as_bytes())
        .is_ok());
    assert!(thread_priority
        .read_bytes_at(0, read_buffer.as_bytes_mut())
        .is_ok());
    assert_eq!(read_buffer, 10);
}
