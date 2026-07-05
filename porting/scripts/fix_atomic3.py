p = '/home/qute-wsl/Program/os-riscv-port/kernel/libs/atomic-integer-wrapper/src/lib.rs'
with open(p,'r') as f: s=f.read()
old = """    } else {
    };
    let from_integer = if try_from.value {"""
new = """    } else {
        item.fields
            .span()
            .unwrap()
            .error("Expected a parenthesized struct like `struct AtomicFoo(AtomicU8)`")
            .emit();
        return TokenStream::new();
    };
    let from_integer = if try_from.value {"""
if old in s:
    s=s.replace(old,new)
    with open(p,'w') as f: f.write(s)
    print('fixed')
else:
    print('pattern not found')
