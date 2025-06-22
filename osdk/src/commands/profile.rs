// SPDX-License-Identifier: MPL-2.0

//! OSDK profile command implementation.
//!
//! The profile command is used to collect stack traces when running the target
//! kernel in QEMU. It attaches to the GDB server initiated with [`super::run`]
//! and collects the stack trace periodically. The collected data can be
//! further analyzed using tools like
//! [flame graph](https://github.com/brendangregg/FlameGraph).

use inferno::flamegraph;

use crate::{
    cli::{ProfileArgs, ProfileFormat},
    commands::util::bin_file_name,
    util::{get_kernel_crate, get_target_directory, new_command_checked_exists},
};
use regex::Regex;
use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, Write},
    path::PathBuf,
    process::Stdio,
    thread, time,
};

pub fn execute_profile_command(_profile: &str, args: &ProfileArgs) {
    if let Some(parse_input) = &args.parse {
        do_parse_stack_traces(parse_input, args);
    } else {
        do_collect_stack_traces(args);
    }
}

fn do_parse_stack_traces(target_file: &PathBuf, args: &ProfileArgs) {
    let out_args = &args.out_args;
    let in_file = File::open(target_file).expect("Failed to open input file");
    let profile: Profile =
        serde_json::from_reader(in_file).expect("Failed to parse the input JSON file");
    let out_file = File::create(out_args.output_path(Some(target_file)))
        .expect("Failed to create output file");

    let out_format = out_args.format();
    if matches!(out_format, ProfileFormat::Json) {
        println!("Warning: parsing JSON profile to the same format.");
        return;
    }
    profile.serialize_to(out_format, out_args.cpu_mask, out_file);
}

macro_rules! profile_round_delimiter {
    () => {
        "-<!OSDK_PROF_BT_ROUND!>-"
    };
}

fn do_collect_stack_traces(args: &ProfileArgs) {
    let file_path = get_target_directory()
        .join("osdk")
        .join(get_kernel_crate().name)
        .join(bin_file_name());

    let remote = &args.remote;
    let samples = &args.samples;
    let interval = &args.interval;

    let mut profile_buffer = ProfileBuffer::new();

    println!("Profiling \"{}\" at \"{}\".", file_path.display(), remote);
    // Use GDB to halt the remote, get stack traces, and resume
    let mut gdb_process = {
        let file_cmd = format!("file {}", file_path.display());
        let target_cmd = format!("target remote {}", remote);
        let backtrace_cmd_seq = vec![
            "-ex",
            "t a a bt -frame-arguments presence -frame-info short-location",
            "-ex",
            concat!("echo ", profile_round_delimiter!(), "\n"),
            "-ex",
            "continue",
        ];

        let mut gdb_args = vec![
            "-batch",
            "-ex",
            "set pagination 0",
            "-ex",
            &file_cmd,
            "-ex",
            &target_cmd,
        ];
        gdb_args.append(&mut vec![backtrace_cmd_seq; *samples].concat());

        new_command_checked_exists("gdb")
            .args(gdb_args)
            .stdout(Stdio::piped())
            .spawn()
            .expect("Failed to execute gdb")
    };

    let gdb_stdout = gdb_process.stdout.take().unwrap();
    let mut gdb_stdout_reader = std::io::BufReader::new(gdb_stdout);
    let mut gdb_stdout_buf = String::new();
    let mut gdb_output = String::new();
    use indicatif::{ProgressIterator, ProgressStyle};
    let style = ProgressStyle::default_bar().progress_chars("#>-");
    for _ in (0..*samples).progress_with_style(style) {
        loop {
            gdb_stdout_buf.clear();
            let _ = gdb_stdout_reader.read_line(&mut gdb_stdout_buf);
            gdb_output.push_str(&gdb_stdout_buf);
            if gdb_stdout_buf.contains(profile_round_delimiter!()) {
                break;
            }
        }
        for line in gdb_output.lines() {
            profile_buffer.append_raw_line(line);
        }
        gdb_output.clear();
        thread::sleep(time::Duration::from_secs_f64(*interval));
        let _ = new_command_checked_exists("kill")
            .args(["-INT", &format!("{}", gdb_process.id())])
            .output();
    }

    let out_args = &args.out_args;
    let out_path = out_args.output_path(None);
    println!(
        "{} profile samples collected. Writing the output to \"{}\".",
        profile_buffer.cur_profile.nr_stack_traces(),
        out_path.display()
    );

    let out_file = File::create(out_path).expect("Failed to create output file");
    profile_buffer
        .cur_profile
        .serialize_to(out_args.format(), out_args.cpu_mask, out_file);
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Profile {
    // Index 0: capture; Index 1: CPU ID; Index 2: stack frame
    stack_traces: Vec<HashMap<u32, Vec<String>>>,
}

impl Profile {
    fn serialize_to<W: Write>(&self, format: ProfileFormat, cpu_mask: u128, mut target: W) {
        match format {
            ProfileFormat::Folded => {
                let folded = self.fold(cpu_mask);

                // Write the folded traces to the target text writer.
                for (key, count) in folded {
                    writeln!(&mut target, "{} {}", key, count)
                        .expect("Failed to write folded output");
                }
            }
            ProfileFormat::Json => {
                let filtered = self.filter_cpu(cpu_mask);

                serde_json::to_writer(target, &filtered).expect("Failed to write JSON output");
            }
            ProfileFormat::FlameGraph => {
                let folded = self.fold(cpu_mask);

                // Generate the flame graph folded text lines.
                let lines = folded
                    .iter()
                    .map(|(key, count)| format!("{} {}", key, count))
                    .collect::<Vec<_>>();

                // Generate the flame graph to the target SVG writer.
                let mut opt = flamegraph::Options::default();
                flamegraph::from_lines(&mut opt, lines.iter().map(|s| s.as_str()), target).unwrap();
            }
        }
    }

    fn filter_cpu(&self, cpu_mask: u128) -> Profile {
        let filtered_traces = self
            .stack_traces
            .iter()
            .map(|capture| {
                capture
                    .iter()
                    .filter(|(cpu_id, _)| **cpu_id < 128 && cpu_mask & (1u128 << **cpu_id) != 0)
                    .map(|(cpu_id, stack)| (*cpu_id, stack.clone()))
                    .collect::<HashMap<_, _>>()
            })
            .collect::<Vec<_>>();

        Self {
            stack_traces: filtered_traces,
        }
    }

    fn fold(&self, cpu_mask: u128) -> HashMap<String, u32> {
        let mut folded = HashMap::new();

        for capture in &self.stack_traces {
            for (cpu_id, stack) in capture {
                if *cpu_id >= 128 || cpu_mask & (1u128 << *cpu_id) == 0 {
                    continue;
                }

                let folded_key = stack.iter().rev().cloned().collect::<Vec<_>>().join(";");
                *folded.entry(folded_key).or_insert(0) += 1;
            }
        }

        folded
    }

    fn nr_stack_traces(&self) -> usize {
        self.stack_traces.len()
    }
}

#[derive(Debug)]
struct ProfileBuffer {
    cur_profile: Profile,
    // Pre-compile regex patterns for cleaning the input.
    hex_in_pattern: Regex,
    impl_pattern: Regex,
    // The state
    cur_cpu: Option<u32>,
}

impl ProfileBuffer {
    fn new() -> Self {
        Self {
            cur_profile: Profile::default(),
            hex_in_pattern: Regex::new(r"0x[0-9a-f]+ in").unwrap(),
            impl_pattern: Regex::new(r"::\{.*?\}").unwrap(),
            cur_cpu: None,
        }
    }

    fn append_raw_line(&mut self, line: &str) {
        // Lines starting with '#' are stack frames
        if !line.starts_with('#') {
            // Otherwise it may initiate a new capture or a new CPU stack trace

            // Check if this is a new CPU trace (starts with `Thread` and contains `CPU#N`)
            if line.ends_with("[running])):") || line.ends_with("[halted ])):") {
                let cpu_id_idx = line.find("CPU#").unwrap();
                let cpu_id = line[cpu_id_idx + 4..]
                    .split_whitespace()
                    .next()
                    .unwrap()
                    .parse::<u32>()
                    .unwrap();
                self.cur_cpu = Some(cpu_id);

                // if the new CPU id is already in the stack traces, start a new capture
                match self.cur_profile.stack_traces.last() {
                    Some(capture) => {
                        if capture.contains_key(&cpu_id) {
                            self.cur_profile.stack_traces.push(HashMap::new());
                        }
                    }
                    None => {
                        self.cur_profile.stack_traces.push(HashMap::new());
                    }
                }
            }

            return;
        }

        // Clean the input line
        let mut processed = line.trim().to_string();

        // Remove everything between angle brackets '<...>'
        processed = Self::remove_generics(&processed);

        // Remove "::impl{}" and hex addresses
        processed = self.impl_pattern.replace_all(&processed, "").to_string();
        processed = self.hex_in_pattern.replace_all(&processed, "").to_string();

        // Remove unnecessary parts like "()" and "(...)"
        processed = processed.replace("(...)", "");
        processed = processed.replace("()", "");

        // Split the line by spaces and expect the second part to be the function name
        let parts: Vec<&str> = processed.split_whitespace().collect();
        if parts.len() > 1 {
            let func_name = parts[1].to_string();

            // Append the function name to the latest stack trace
            let current_capture = self.cur_profile.stack_traces.last_mut().unwrap();
            let cur_cpu = self.cur_cpu.unwrap();
            current_capture.entry(cur_cpu).or_default().push(func_name);
        }
    }

    fn remove_generics(line: &str) -> String {
        let mut result = String::new();
        let mut bracket_depth = 0;

        for c in line.chars() {
            match c {
                '<' => bracket_depth += 1,
                '>' => {
                    if bracket_depth > 0 {
                        bracket_depth -= 1;
                    }
                }
                _ => {
                    if bracket_depth == 0 {
                        result.push(c);
                    }
                }
            }
        }

        result
    }
}

#[cfg(test)]
#[test]
fn test_profile_parse_raw() {
    let test_case = r#"
0xffffffff880b0f6f in aster_nix::sched::priority_scheduler::{impl#4}::pick_next_current<ostd::task::Task> (self=0xffffffff88489808 <_ZN4ostd2mm14heap_allocator10HEAP_SPACE17h85a5340e6564f69dE.llvm.15305379556759765072+992480>) at src/sched/priority_scheduler.rs:156
156	        let next_entity = if !self.real_time_entities.is_empty() {

Thread 2 (Thread 1.2 (CPU#1 [running])):
#0  ostd::sync::spin::SpinLock<aster_nix::sched::priority_scheduler::PreemptRunQueue<ostd::task::Task>, ostd::sync::spin::PreemptDisabled>::acquire_lock<aster_nix::sched::priority_scheduler::PreemptRunQueue<ostd::task::Task>, ostd::sync::spin::PreemptDisabled> (...)
#1  ostd::sync::spin::SpinLock<aster_nix::sched::priority_scheduler::PreemptRunQueue<ostd::task::Task>, ostd::sync::spin::PreemptDisabled>::lock<aster_nix::sched::priority_scheduler::PreemptRunQueue<ostd::task::Task>, ostd::sync::spin::PreemptDisabled> (...)
#2  aster_nix::sched::priority_scheduler::{impl#1}::local_mut_rq_with<ostd::task::Task> (...)
#3  0xffffffff8826b205 in ostd::task::scheduler::reschedule<ostd::task::scheduler::yield_now::{closure_env#0}> (...)
#4  ostd::task::scheduler::yield_now ()
#5  0xffffffff880a92c5 in ostd::task::Task::yield_now ()
#6  aster_nix::thread::Thread::yield_now ()
#7  aster_nix::ap_init::ap_idle_thread ()
#8  core::ops::function::Fn::call<fn(), ()> ()
#9  0xffffffff880b341e in alloc::boxed::{impl#50}::call<(), (dyn core::ops::function::Fn<(), Output=()> + core::marker::Send + core::marker::Sync), alloc::alloc::Global> (...)
#10 aster_nix::thread::kernel_thread::create_new_kernel_task::{closure#0} ()
#11 0xffffffff882a3ea8 in alloc::boxed::{impl#50}::call<(), (dyn core::ops::function::Fn<(), Output=()> + core::marker::Send + core::marker::Sync), alloc::alloc::Global> (...)
#12 ostd::task::{impl#2}::build::kernel_task_entry ()
#13 0x0000000000000000 in ?? ()

Thread 1 (Thread 1.1 (CPU#0 [running])):
#0  aster_nix::sched::priority_scheduler::{impl#1}::local_mut_rq_with<ostd::task::Task> (...)
#1  0xffffffff8826b205 in ostd::task::scheduler::reschedule<ostd::task::scheduler::yield_now::{closure_env#0}> (...)
#2  ostd::task::scheduler::yield_now ()
#3  0xffffffff880a92c5 in ostd::task::Task::yield_now ()
#4  aster_nix::thread::Thread::yield_now ()
#5  aster_nix::ap_init::ap_idle_thread ()
#6  core::ops::function::Fn::call<fn(), ()> ()
#7  0xffffffff880b341e in alloc::boxed::{impl#50}::call<(), (dyn core::ops::function::Fn<(), Output=()> + core::marker::Send + core::marker::Sync), alloc::alloc::Global> (...)
#8  aster_nix::thread::kernel_thread::create_new_kernel_task::{closure#0} ()
#9  0xffffffff882a3ea8 in alloc::boxed::{impl#50}::call<(), (dyn core::ops::function::Fn<(), Output=()> + core::marker::Send + core::marker::Sync), alloc::alloc::Global> (...)
#10 ostd::task::{impl#2}::build::kernel_task_entry ()
#11 0x0000000000000000 in ?? ()
[Inferior 1 (process 1) detached]
0xffffffff880b0f6f in aster_nix::sched::priority_scheduler::{impl#4}::pick_next_current<ostd::task::Task> (self=0xffffffff88489808 <_ZN4ostd2mm14heap_allocator10HEAP_SPACE17h85a5340e6564f69dE.llvm.15305379556759765072+992480>) at src/sched/priority_scheduler.rs:156
156	        let next_entity = if !self.real_time_entities.is_empty() {

Thread 2 (Thread 1.2 (CPU#1 [running])):
#0  0xffffffff880b0f6f in aster_nix::sched::priority_scheduler::{impl#4}::pick_next_current<ostd::task::Task> (...)
#1  0xffffffff8826b3e0 in ostd::task::scheduler::yield_now::{closure#0} (...)
#2  ostd::task::scheduler::reschedule::{closure#0}<ostd::task::scheduler::yield_now::{closure_env#0}> (...)
#3  0xffffffff880b0cff in aster_nix::sched::priority_scheduler::{impl#1}::local_mut_rq_with<ostd::task::Task> (...)
#4  0xffffffff8826b205 in ostd::task::scheduler::reschedule<ostd::task::scheduler::yield_now::{closure_env#0}> (...)
#5  ostd::task::scheduler::yield_now ()
#6  0xffffffff880a92c5 in ostd::task::Task::yield_now ()
#7  aster_nix::thread::Thread::yield_now ()
#8  aster_nix::ap_init::ap_idle_thread ()
#9  core::ops::function::Fn::call<fn(), ()> ()
#10 0xffffffff880b341e in alloc::boxed::{impl#50}::call<(), (dyn core::ops::function::Fn<(), Output=()> + core::marker::Send + core::marker::Sync), alloc::alloc::Global> (...)
#11 aster_nix::thread::kernel_thread::create_new_kernel_task::{closure#0} ()
#12 0xffffffff882a3ea8 in alloc::boxed::{impl#50}::call<(), (dyn core::ops::function::Fn<(), Output=()> + core::marker::Send + core::marker::Sync), alloc::alloc::Global> (...)
#13 ostd::task::{impl#2}::build::kernel_task_entry ()
#14 0x0000000000000000 in ?? ()

Thread 1 (Thread 1.1 (CPU#0 [running])):
#0  ostd::arch::x86::interrupts_ack (...)
#1  0xffffffff8828d704 in ostd::trap::handler::call_irq_callback_functions (...)
#2  0xffffffff88268e48 in ostd::arch::x86::trap::trap_handler (...)
#3  0xffffffff88274db6 in __from_kernel ()
#4  0x0000000000000001 in ?? ()
#5  0x0000000000000001 in ?? ()
#6  0x00000000000001c4 in ?? ()
#7  0xffffffff882c8580 in ?? ()
#8  0x0000000000000002 in ?? ()
#9  0xffffffff88489808 in _ZN4ostd2mm14heap_allocator10HEAP_SPACE17h85a5340e6564f69dE.llvm.15305379556759765072 ()
#10 0x0000000000000000 in ?? ()
[Inferior 1 (process 1) detached]
"#;

    let mut buffer = ProfileBuffer::new();
    for line in test_case.lines() {
        buffer.append_raw_line(line);
    }

    let profile = &buffer.cur_profile;
    assert_eq!(profile.stack_traces.len(), 2);
    assert_eq!(profile.stack_traces[0].len(), 2);
    assert_eq!(profile.stack_traces[1].len(), 2);

    let stack00 = profile.stack_traces[0].get(&0).unwrap();
    assert_eq!(stack00.len(), 12);
    assert_eq!(
        stack00[0],
        "aster_nix::sched::priority_scheduler::local_mut_rq_with"
    );
    assert_eq!(stack00[11], "??");

    let stack01 = profile.stack_traces[0].get(&1).unwrap();
    assert_eq!(stack01.len(), 14);
    assert_eq!(stack01[9], "alloc::boxed::call");

    let stack10 = profile.stack_traces[1].get(&0).unwrap();
    assert_eq!(stack10.len(), 11);
    assert_eq!(
        stack10[9],
        "_ZN4ostd2mm14heap_allocator10HEAP_SPACE17h85a5340e6564f69dE.llvm.15305379556759765072"
    );

    let stack11 = profile.stack_traces[1].get(&1).unwrap();
    assert_eq!(stack11.len(), 15);
    assert_eq!(
        stack11[0],
        "aster_nix::sched::priority_scheduler::pick_next_current"
    );
    assert_eq!(stack11[14], "??");
}
