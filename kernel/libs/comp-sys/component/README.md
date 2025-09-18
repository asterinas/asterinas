# Component

## Overview
This crate is used for the initialization of the component system, which provides a priority initialization scheme based on the inventory crate.

## Initialization Stages
The component system supports three distinct initialization stages that execute in a specific order during system boot:
1. **Bootstrap**: The earliest stage, called after OSTD initialization is complete but before kernel subsystem initialization begins. This stage runs on the BSP (Bootstrap Processor) only, before SMP (Symmetric Multi-Processing) is enabled. Components in this stage can initialize core kernel services that other components depend on.
2. **Kthread**: The kernel thread stage, initialized after SMP is enabled and the first kernel thread is spawned. This stage runs in the context of the first kernel thread on the BSP.
3. **Process**: The process stage, initialized after the first user process is created. This stage runs in the context of the first user process, and prepares the system for user-space execution.

## Usage

### Register component

Registering a crate as component by marking a function in the lib.rs with `#[init_component]` macro. You can specify the initialization stage for your component:
- `#[init_component]` or `#[init_component(bootstrap)]` - registers for the **Bootstrap** stage
- `#[init_component(kthread)]` - registers for the **Kthread** stage
- `#[init_component(process)]` - registers for the **Process** stage

**Note**: Each crate can declare up to three initialization functions (one for each stage), but each function can only be associated with one specific stage. A given stage can only be declared once per crate.

The specific definition of the function can refer to the comments in the macro.

### Component initialization

Component system need to be initialized by calling `component::init_all` function and it needs  information about all components. Usually it is used with the `component::parse_metadata` macro.

## Example

```rust
// comp1/lib.rs
use std::sync::atomic::AtomicU16;
use std::sync::atomic::Ordering::Relaxed;

use component::init_component;

pub static INIT_COUNT : AtomicU16 = AtomicU16::new(0);

#[init_component]
fn comp1_init() -> Result<(), component::ComponentInitError> {
    assert_eq!(INIT_COUNT.load(Relaxed),0);
    INIT_COUNT.fetch_add(1,Relaxed);
    Ok(())
}

// src/main.rs
use std::sync::atomic::Ordering::Relaxed;

use component::{init_component, InitStage};
use comp1::INIT_COUNT;

#[init_component]
fn init() -> Result<(), component::ComponentInitError> {
    assert_eq!(INIT_COUNT.load(Relaxed),1);
    INIT_COUNT.fetch_add(1,Relaxed);
    Ok(())
}

fn main(){
    component::init_all(InitStage::Bootstrap, component::parse_metadata!()).unwrap();
    assert_eq!(INIT_COUNT.load(Relaxed),2);
}
```

## Notes

- Currently, initialization requires the presence of a `Components.toml` file, which stores some information about components and access control. The [tests](tests/kernel/Components.toml) provides a sample file of it. If the components declared inside `Components.toml` is inconsistent with the component found by `parse_metadata` macro (i.e. A crate depends on the component library but is not declared in `Components.toml`), then a compilation error will occur.

- The `parse_metadata` macro will generate the information of all components. But ultimately which functions are called still depends on which `#[init_component]` macros are extended. If you want to test a component. Then, other components with a lower priority than it or other unused high-priority components will not be initialized at runtime.
