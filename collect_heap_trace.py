
def main():
    qemu_log = open('qemu.log', 'r')
    # find the heap trace in it
    started = False
    trace = None
    for line in qemu_log:
        if "###DUMP_HEAP_PROFILE_END###" in line:
            break
        if started:
            trace = line.strip()
        if "###DUMP_HEAP_PROFILE_START###" in line:
            started = True
    import json
    trace = json.loads(trace)
    # a list of:
    # 
    # #[derive(serde::Serialize)]
    # struct ProfileEntry {
    #     alloc_size: usize,
    #     stack: Vec<usize>,
    #     is_stack_complete: bool,
    # }
    collapsed = []
    for entry in trace:
        alloc_size = entry['alloc_size']
        stack = entry['stack']
        folded = []
        for frame_pc in reversed(stack):
            # call addr2line to get the function name
            import subprocess
            p = subprocess.Popen(['addr2line', '-f', '-e', 'target/osdk/asterinas/asterinas-osdk-bin', '-a', hex(frame_pc)], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            out, err = p.communicate()
            out = out.decode('utf-8').split('\n')[1]
            from rust_demangler import demangle
            try:
                out = demangle(out)
            except:
                print(out, "can't be demangled")
            # remove all generic types
            langle_cnt = 0
            processed = ""
            for c in out:
                if c == '<':
                    langle_cnt += 1
                    continue
                if c == '>':
                    langle_cnt -= 1
                    continue
                if langle_cnt == 0:
                    processed += c
            # remove the last ::ahsdiunzlc part
            processed = "::".join(processed.split("::")[:-1])
            folded.append(processed)
        collapsed.append(";".join(folded) + f" {alloc_size}")
    with open('heap_trace.folded', 'w') as f:
        f.write('\n'.join(collapsed))

if __name__ == '__main__':
    main()
