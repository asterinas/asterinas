# `kernel/`

Guidelines that apply only to code under `kernel/`.

### Keep the Linux Compatibility docs current (`linux-compat-docs`) {#linux-compat-docs}

When a change under `kernel/` changes the supported system calls, flags, arguments,
file systems, sockets, boot protocols, or kernel parameters,
update the [Linux Compatibility](../../../../kernel/linux-compatibility/) section in the same PR,
so the documented coverage keeps matching the code.
This includes adding support, removing support, or restricting supported argument combinations.

- For a **system call**:
  update the matching [Syscall Flag Coverage](../../../../kernel/linux-compatibility/syscall-flag-coverage/) page.
  Update its [SCML](../../../../kernel/linux-compatibility/syscall-flag-coverage/system-call-matching-language.md) (`.scml`) coverage file
  only when the supported invocation patterns change,
  such as when adding a syscall, flag, ioctl command, or socket option.
- For a **kernel parameter**:
  update the [Kernel Parameters](../../../../kernel/linux-compatibility/kernel-parameters.md) page.
- For supported **file systems**, **sockets**, or **boot protocols**:
  update the corresponding [File systems](../../../../kernel/linux-compatibility/#file-systems),
  [Sockets](../../../../kernel/linux-compatibility/#sockets),
  or [Boot protocols](../../../../kernel/linux-compatibility/#boot-protocols) section.

Do not update the Linux Compatibility pages when a change fixes a Linux compatibility bug
without changing Asterinas's claimed coverage of Linux features.
