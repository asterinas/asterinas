# System Call Matching Language (SCML)

SCML specifies matching patterns for system‑call invocations.
Asterinas developers can easily write SCML rules to describe supported patterns.
Likewise, users and developers can intuitively read these rules 
to understand which system calls and features are available.

SCML is designed to integrate seamlessly with
[strace](https://man7.org/linux/man-pages/man1/strace.1.html),
the standard Linux system‑call tracer.
Strace emits each invocation in a C‑style syntax;
given a set of SCML rules,
a tool can automatically determine
whether a strace log entry conforms to the supported patterns.
This paves the way for an SCML‑based analyzer
that reports unsupported calls in any application's trace.

## Strace: A Quick Example

To illustrate, run strace on a simple "Hello, World!" program:

```bash
$ strace ./hello_world
```

A typical trace might look like this:

```shell
execve("./hello_world", ["./hello_world"], 0xffffffd3f710 /* 4 vars */) = 0
brk(NULL)                               = 0xaaaabdc1b000
mmap(NULL, 8192, PROT_READ|PROT_WRITE, MAP_PRIVATE|MAP_ANONYMOUS, -1, 0) = 0xffff890f4000
openat(AT_FDCWD, "/lib/aarch64-linux-gnu/libc.so.6", O_RDONLY|O_CLOEXEC) = 3
read(3, "\177ELF\2\1\1\3\0\0\0\0\0\0\0\0\3\0\267\0\1\0\0\0\360\206\2\0\0\0\0\0"..., 832) = 832
fstat(3, {st_mode=S_IFREG|0755, st_size=1722920, ...}) = 0
…
write(1, "Hello, World!\n", 14)         = 14
exit_group(0)                           = ?
```

Key points of this output:

* System calls are rendered as `name(arg1, …, argN)`.
* Flags appear as `FLAG1|FLAG2|…|FLAGN`.
* Structs use `{field1=value1, …}`.
* Arrays are shown as `[value1, …]`.

SCML's syntax draws directly from these conventions.

## SCML by Example

SCML is intentionally simple:
most Linux system‑call semantics hinge on bitflags.
SCML rules act as templates:
you define a rule once,
and a human or an analyzer uses it to check if a syscall invocation matches it or not.

Imagine you're developing a Linux-compatible OS (like Asterinas)
that supports just a restricted subset of syscalls and their options.
We will use SCML to describe the restricted functionality.

### Matching Rules for System Calls

For example,
your OS supports the [`open`](https://man7.org/linux/man-pages/man2/openat.2.html) system call 
with one or more of the four flags: `O_RDONLY`, `O_WRONLY`, `O_RDWR`, and `O_CLOEXEC`:
This constraint can be expressed in the following system call matching rule.

```c
open(path, flags = O_RDONLY | O_WRONLY | O_RDWR | O_CLOEXEC);
```

To allow file creation,
you add another matching rule that 
includes the `O_CREAT` flag and requires a `mode` argument:

```c
open(path, flags = O_CREAT | O_RDONLY | O_WRONLY | O_RDWR | O_CLOEXEC, mode);
```

To support the `O_PATH` flag
(only valid with `O_CLOEXEC`, not with  `O_RDONLY`, `O_WRONLY`, or `O_RDWR`),
you add a third matching rule:

```c
open(path, flags = O_PATH | O_CLOEXEC);
```

SCML rules constrain only the flagged arguments;
other parameters (like `path` and `mode`) accept any value.

In many system calls, the number of arguments may vary depending on the flags provided.
To accommodate this, SCML allows you to use the `..` wildcard in the parameter list.
This indicates that any additional arguments are accepted, regardless of their value or count.

For example:

```c
open(path, flags = O_RDONLY | O_WRONLY | O_RDWR | O_CLOEXEC, ..);
```

Here, the `..` wildcard makes the rule flexible enough to match invocations of `open` with extra parameters,
such as when the `O_CREAT` flag is present and a `mode` argument is required.
This approach makes it easy to write concise rules that only constrain the arguments of interest,
while allowing other parameters to vary as needed.

### C-Style Comments

SCML also supports C‑style comments:

```c
// All matching rules for the open syscall.
// A supported invocation of the open syscall must match at least one of the rules.
open(path, flags = O_RDONLY | O_WRONLY | O_RDWR | O_CLOEXEC);
open(path, flags = O_CREAT | O_RDONLY | O_WRONLY | O_RDWR | O_CLOEXEC, mode);
open(path, flags = O_PATH | O_CLOEXEC);
```

### Matching Rules for Bitflags

Above, we embedded flag combinations directly within individual system‑call rules,
which can lead to duplication and make maintenance harder.
SCML allows you to define named bitflag rules that
can be reused across multiple rules.
This reduces repetition and centralizes your flag definitions.
For example:

```c
// Define a reusable bitflags rule
access_mode = O_RDONLY | O_WRONLY | O_RDWR;

open(path, flags = <access_mode> | O_CLOEXEC);
open(path, flags = O_CREAT | <access_mode> | O_CLOEXEC, mode);
open(path, flags = O_PATH | O_CLOEXEC);
```

### Matching Rules for Structs

SCML can match flags inside struct fields.
Consider [`sigaction`](https://man7.org/linux/man-pages/man2/sigaction.2.html):

```c
struct sigaction = {
    sa_flags = SA_NOCLDSTOP | SA_NOCLDWAIT,
    ..
};
```

Here, `..` is a wildcard for remaining fields that we do not care.

Then, we can write a system call rule that
refers to the struct rule using the `<struct_rule>` syntax.

```c
sigaction(signum, act = <sigaction>, oldact = <sigaction>);
```

Instead of defining a separate struct rule,
you can also inline the struct pattern directly in the parameter list.
This is convenient when the struct pattern is only used once
or when you want to express different constraints for the same struct type in different contexts.

For example, the following rule inlines the struct pattern for `capget`:

```c
capget(
    hdrp = {
        version = _LINUX_CAPABILITY_VERSION_3,
        ..
    },
    datap
);
```

### Matching Rules for Arrays

SCML can describe how to match flags embedded inside the struct values of an array.
This is the case of the [`poll`](https://man7.org/linux/man-pages/man2/poll.2.html) system call.
It takes an array of values of `struct pollfd`,
whose `event` and `revents` fields are bitflags.

```c
// Support all but the POLLPRI flags
events = POLLIN | POLLOUT | POLLRDHUP | POLLERR | POLLHUP | POLLNVAL;

struct pollfd = {
    events  = <events>,
    revents = <events>,
    ..
};

poll(fds = [ <pollfd> ], nfds, timeout);
```

Notice how SCML denotes an array with the `[ <struct_rule> ]` syntax.

### Special Built-in Matching Rules

Bitflags-based matching rules described above are expressive enough to
capture most patterns of interesting system call arguments.
But some system call arguments cannot be characterized with bitflags.
To address such cases, SCML introduces two special built-in matching rules:
`<PATH>` and `<INTEGER>`

#### The file path matching rule

The `<PATH>` matching rule is used to
denote a system call argument of a C-string file path.
For example, the matching rules for the `open` system call
can be enhanced with `<PATH>` as follows:

```c
access_mode = O_RDONLY | O_WRONLY | O_RDWR;

open(path = <PATH>, flags = <access_mode> | O_CLOEXEC);
open(path = <PATH>, flags = O_CREAT | <access_mode> | O_CLOEXEC, mode);
open(path = <PATH>, flags = O_PATH | O_CLOEXEC);
```

File paths provide a new dimension to determine whether a system call is supported or not.
Linux has multiple pseudo file systems such as
DevTmpFS, ProcFS, SysFS, CgroupFS, and ConfigFS,
mounted at well-known locations.
A Linux-compatible OS such as Asterinas may only support a sub-tree of an pseudo FS.
Knowing which system call arguments refer to file paths,
a tool may be built to automatically issue warnings
when unsupported file paths are accessed by system calls.

#### The integer matching rule

The `<INTEGER>` matching rule can match any integer system call argument
such as `1234`, `-100`, `0xdeadbeef`, and `0o666`.
It can be used as a fallback rule
when a system call takes an argument of either bitflags or integer.

```c
timer_create(
    clockid =
        // Static clock IDs represented as bitflags
        CLOCK_PROCESS_CPUTIME_ID | CLOCK_THREAD_CPUTIME_ID | CLOCK_REALTIME | CLOCK_MONOTONIC | CLOCK_BOOTTIME |
        // Dynamic clock IDs (per-process or per-thread clock IDs)
        // represented as an integer value.
        <INTEGER>,
    sevp,
    timerid
);
```

### Advanced Usage

Just like you can write multiple rules of the same system call,
you may define multiple rules for the same struct:

```c
// Rules for control message header
struct cmsghdr = {
    cmsg_level = SOL_SOCKET,
    cmsg_type  = SO_TIMESTAMP_OLD | SCM_RIGHTS | SCM_CREDENTIALS,
    ..
};
struct cmsghdr = {
    cmsg_level = SOL_IP,
    cmsg_type  = IP_TTL,
    ..
};
```

A `cmsghdr` value matches if it satisfies any one rule.

Struct rules may also be nested:

```c
// Rule for message header, which refers to the rules for control message header
struct msghdr = {
    msg_control = [ <cmsghdr> ],
    ..
};

recvmsg(socket, message = <msghdr>, flags);
```

SCML supports arrays with nested structures and heterogeneous element types,
as encountered in system calls like `recvmsg` where netlink message payloads
follow the TLV (Type-Length-Value) format.
Arrays can contain multiple elements of varying types:
inline struct patterns (`{ ... }`), nested arrays (`[ ... ]`),
or references to named rules (`<identifier>`).
This flexibility allows SCML to represent hierarchical data structures
as they appear in strace output.

For example, when receiving a netlink message about adding a network address:

```c
struct iovec = {
    iov_base = [
        [
            {
                nlmsg_type = RTM_NEWADDR,
                ..
            },
            [
                [ { nla_type = IFA_CACHEINFO, .. } ]
            ]
        ]
    ],
    ..
};

recvmsg(
    sockfd,
    msg = {
        msg_iov = [ <iovec> ],
        ..
    },
    flags
);
```

This example demonstrates receiving a netlink message of type
`RTM_NEWADDR` containing nested attributes with cache information
(`IFA_CACHEINFO`). The nested array structure illustrates how SCML
handles heterogeneous arrays where elements can be both structs and
nested arrays, reflecting the hierarchical TLV encoding typical of netlink.

## Formal Syntax

Below is the formal syntax of SCML,
expressed in Extended Backus–Naur Form (EBNF).
Non‑terminals are in angle brackets, terminals in quotes.

```
<scml>           ::= { <rule> }
<rule>           ::= <syscall-rule> ';' 
                   | <struct-rule> ';'
                   | <bitflags-rule> ';'

<syscall-rule>   ::= <identifier> '(' [ <param-list> ] ')'
<param-list>     ::= '..'
                   | <param> { ',' <param> } [ ',' '..' ]
<param>          ::= <identifier> '=' <flag-pattern>
                   | <identifier> '=' <struct-pattern>
                   | <identifier> '=' <array-pattern>
                   | <identifier>

<flag-pattern>   ::= <flag-part> { '|' <flag-part> }
<flag-part>      ::= <identifier>
                   | '<' <identifier> '>'

<array-pattern>  ::= '[' <array-element> { ',' <array-element> } ']'
<array-element>  ::= '<' <identifier> '>'
                   | <struct-pattern>
                   | <array-pattern>

<struct-rule>    ::= 'struct' <identifier> '=' <struct-pattern>
<struct-pattern> ::= '{' <field-list> [ ',' '..' ] '}'
<field-list>     ::= <field> { ',' <field> }
<field>          ::= <identifier>
                   | <identifier> '=' <flag-pattern>
                   | <identifier> '=' <struct-pattern>
                   | <identifier> '=' <array-pattern>

<bitflags-rule>  ::= <identifier> '=' <flag-pattern>

<identifier>     ::= letter { letter | digit | '_' }

comment          ::= '//' { any-char }
```
