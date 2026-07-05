path='/home/qute-wsl/Program/os-riscv-port/kernel/libs/atomic-integer-wrapper/src/lib.rs'
with open(path) as f: s=f.read()
old='''    let atomic_integer_type = if let Fields::Unnamed(ref fields_unnamed) = item.fields
        && fields_unnamed.unnamed.len() == 1
    {
        fields_unnamed.unnamed.first().unwrap().ty.clone()
    } else {'''
new='''    let atomic_integer_type = if let Fields::Unnamed(ref fields_unnamed) = item.fields {
        if fields_unnamed.unnamed.len() == 1 {
            fields_unnamed.unnamed.first().unwrap().ty.clone()
        } else {
            item.fields
                .span()
                .unwrap()
                .error("Expected a parenthesized struct like `struct AtomicFoo(AtomicU8)`")
                .emit();
            return TokenStream::new();
        }
    } else {'''
if old not in s:
    print('old pattern not found')
else:
    s=s.replace(old,new)
    # Remove the now-duplicate error block inside the else branch
    dup='''        item.fields
            .span()
            .unwrap()
            .error("Expected a parenthesized struct like `struct AtomicFoo(AtomicU8)`")
            .emit();
        return TokenStream::new();
    };'''
    s=s.replace(dup,'    };')
    with open(path,'w') as f: f.write(s)
    print('rewritten')
