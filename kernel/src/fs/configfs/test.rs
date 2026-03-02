// SPDX-License-Identifier: MPL-2.0

//! Testing configfs by adding to it a top-level directory called `demo_set`,
//! whose structure is illustrated as follows:
//!
//! ```
//! configfs/
//!     demo_set/
//!         demo_foo/
//!              attr_a
//!              attr_b
//!         demo_bar/
//!              attr_a
//!              attr_b
//! ```
//!
//! The `demo_set` is initially empty. One can create directories to trigger creating an in-kernel object
//! that represents a new demo. Each demo has two attributes called `attr_a` and `attr_b`.

use alloc::{string::ToString, sync::Arc};
use core::{
    fmt::Debug,
    sync::atomic::{AtomicU32, Ordering},
};

use aster_systree::{
    BranchNodeFields, Error, NormalNodeFields, Result, SysAttrSet, SysAttrSetBuilder, SysObj,
    SysPerms, SysStr, inherit_sys_branch_node, inherit_sys_leaf_node,
};
use inherit_methods_macro::inherit_methods;
use ostd::{
    mm::{VmReader, VmWriter},
    prelude::ktest,
};
use ostd_pod::IntoBytes;
use spin::Once;

use crate::{
    fs::utils::{FileSystem, InodeType, mkmod},
    time::clocks::init_for_ktest as time_init_for_ktest,
};

/// A demo subsystem for testing configfs functionality.
#[derive(Debug)]
struct DemoSet {
    fields: BranchNodeFields<DemoObject, Self>,
}

#[inherit_methods(from = "self.fields")]
impl DemoSet {
    fn new() -> Arc<Self> {
        let name = SysStr::from("demo_set");
        let attrs = SysAttrSet::new_empty();

        Arc::new_cyclic(|weak_self| {
            let fields = BranchNodeFields::new(name, attrs, weak_self.clone());
            DemoSet { fields }
        })
    }

    fn add_child(&self, new_child: Arc<DemoObject>) -> Result<()>;
}

inherit_sys_branch_node!(DemoSet, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }

    fn create_child(&self, name: &str) -> Result<Arc<dyn SysObj>> {
        let demo_obj = DemoObject::new(SysStr::from(name.to_string()));
        self.add_child(demo_obj.clone())?;
        Ok(demo_obj)
    }
});

/// A demo object that can be created dynamically in the configfs.
///
/// Each demo object has two configurable attributes: `attr_a` and `attr_b`.
#[derive(Debug)]
struct DemoObject {
    fields: NormalNodeFields<Self>,
    attr_a: AtomicU32,
    attr_b: AtomicU32,
}

impl DemoObject {
    fn new(name: SysStr) -> Arc<Self> {
        let mut builder = SysAttrSetBuilder::new();

        builder.add(SysStr::from("attr_a"), SysPerms::DEFAULT_RW_ATTR_PERMS);
        builder.add(SysStr::from("attr_b"), SysPerms::DEFAULT_RW_ATTR_PERMS);

        let attrs = builder.build().expect("Failed to build attribute set");

        Arc::new_cyclic(|weak_self| {
            let fields = NormalNodeFields::new(name, attrs, weak_self.clone());
            DemoObject {
                fields,
                attr_a: AtomicU32::new(0),
                attr_b: AtomicU32::new(0),
            }
        })
    }
}

inherit_sys_leaf_node!(DemoObject, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }

    fn read_attr_at(&self, name: &str, _offset: usize, writer: &mut VmWriter) -> Result<usize> {
        match name {
            "attr_a" => {
                let value = self.attr_a.load(Ordering::Relaxed);
                writer.write_val(&value).unwrap();
                Ok(size_of::<u32>())
            }
            "attr_b" => {
                let value = self.attr_b.load(Ordering::Relaxed);
                writer.write_val(&value).unwrap();
                Ok(size_of::<u32>())
            }
            _ => Err(Error::AttributeError),
        }
    }

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> Result<usize> {
        match name {
            "attr_a" => {
                let value = reader.read_val::<u32>().unwrap();
                self.attr_a.store(value, Ordering::Relaxed);
                Ok(size_of::<u32>())
            }
            "attr_b" => {
                let value = reader.read_val::<u32>().unwrap();
                self.attr_b.store(value, Ordering::Relaxed);
                Ok(size_of::<u32>())
            }
            _ => Err(Error::AttributeError),
        }
    }
});

// --- Test Setup ---

static DEMO_SET_SUBSYSTEM: Once<Arc<DemoSet>> = Once::new();

fn init_demo_subsystem() {
    DEMO_SET_SUBSYSTEM.call_once(|| {
        time_init_for_ktest();
        super::init_for_ktest();

        let demo_set = DemoSet::new();
        super::register_subsystem(demo_set.clone()).unwrap();

        demo_set
    });
}

#[ktest]
fn config_fs() {
    init_demo_subsystem();
    let config_fs = super::fs::ConfigFs::singleton();

    // Access the root of configfs: /sys/kernel/config
    let root_inode = config_fs.root_inode();

    // --- Navigate to demo_set directory ---
    // path: /sys/kernel/config/demo_set
    let demo_set_inode = root_inode
        .lookup("demo_set")
        .expect("lookup demo_set failed");

    // --- Create demo objects ---
    // path: /sys/kernel/config/demo_set/demo_foo
    let demo_foo = demo_set_inode
        .create("demo_foo", InodeType::Dir, mkmod!(a+rx, u+w))
        .expect("creating demo 'demo_foo' fails");

    // path: /sys/kernel/config/demo_set/demo_bar
    let demo_bar = demo_set_inode
        .create("demo_bar", InodeType::Dir, mkmod!(a+rx, u+w))
        .expect("creating demo 'demo_bar' fails");

    // --- Test attribute access for demo_foo ---
    let attr_a_foo = demo_foo.lookup("attr_a").expect("lookup attr_a failed");
    let attr_b_foo = demo_foo.lookup("attr_b").expect("lookup attr_b failed");

    let mut read_buffer: u32 = 0;

    // Test attr_a read/write on demo_foo
    assert!(
        attr_a_foo
            .read_bytes_at(0, read_buffer.as_mut_bytes())
            .is_ok()
    );
    assert_eq!(read_buffer, 0);

    let write_value_a: u32 = 42;
    assert!(
        attr_a_foo
            .write_bytes_at(0, write_value_a.as_bytes())
            .is_ok()
    );
    assert!(
        attr_a_foo
            .read_bytes_at(0, read_buffer.as_mut_bytes())
            .is_ok()
    );
    assert_eq!(read_buffer, 42);

    // Test attr_b read/write on demo_foo
    assert!(
        attr_b_foo
            .read_bytes_at(0, read_buffer.as_mut_bytes())
            .is_ok()
    );
    assert_eq!(read_buffer, 0);

    let write_value_b: u32 = 100;
    assert!(
        attr_b_foo
            .write_bytes_at(0, write_value_b.as_bytes())
            .is_ok()
    );
    assert!(
        attr_b_foo
            .read_bytes_at(0, read_buffer.as_mut_bytes())
            .is_ok()
    );
    assert_eq!(read_buffer, 100);

    // --- Test attribute access for demo_bar ---
    let attr_a_bar = demo_bar.lookup("attr_a").expect("lookup attr_a failed");

    // Verify that demo_bar has independent state from demo_foo
    assert!(
        attr_a_bar
            .read_bytes_at(0, read_buffer.as_mut_bytes())
            .is_ok()
    );
    assert_eq!(read_buffer, 0); // Should be 0, not 42 like demo_foo

    let write_value_bar: u32 = 200;
    assert!(
        attr_a_bar
            .write_bytes_at(0, write_value_bar.as_bytes())
            .is_ok()
    );
    assert!(
        attr_a_bar
            .read_bytes_at(0, read_buffer.as_mut_bytes())
            .is_ok()
    );
    assert_eq!(read_buffer, 200);

    // Verify demo_foo's attr_a is still 42
    assert!(
        attr_a_foo
            .read_bytes_at(0, read_buffer.as_mut_bytes())
            .is_ok()
    );
    assert_eq!(read_buffer, 42);
}
