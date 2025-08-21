# Limitations on System Calls

This section documents known limitations of Asterinas's implementation of Linux system calls.
It introduce [**System Call Matching Language (SCML)**](system-call-matching-language.md),
a lightweight domain‑specific language for
specifying allowed and disallowed patterns of system‑call invocations.

The rest of this section uses SCML
to accurately and concisely describe
both supported and unsupported functionality of system calls,
which are divided into the following categories:
* [Process and thread management](process-and-thread-management.md)
* [Memory management](memory-management.md)
* [File & directory operations](file-and-directory-operations.md)
* [File systems & mount control](file-systems-and-mount-control.md)
* [File descriptor & I/O control](file-descriptor-and-io-control.md)
* [Inter-process communication](inter-process-communication.md)
* [Networking & sockets](networking-and-sockets.md)
* [Signals & timers](signals-and-timers.md)
* [Namespaces, cgroups & security](namespaces-cgroups-and-security.md)
* [System information & misc](system-information-and-misc.md)
