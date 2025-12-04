# Syscall Flag Coverage

This section documents the flag coverage of Asterinas's implementation of Linux system calls.
It introduces [**System Call Matching Language (SCML)**](system-call-matching-language.md),
a lightweight domain‑specific language for
specifying allowed and disallowed patterns of system‑call invocations.

The rest of this section uses SCML
to accurately and concisely describe
both supported and unsupported functionality of system calls,
which are divided into the following categories:
* [Process and thread management](process-and-thread-management/)
* [Memory management](memory-management/)
* [File & directory operations](file-and-directory-operations/)
* [File systems & mount control](file-systems-and-mount-control/)
* [File descriptor & I/O control](file-descriptor-and-io-control/)
* [Inter-process communication](inter-process-communication/)
* [Networking & sockets](networking-and-sockets/)
* [Signals & timers](signals-and-timers/)
* [Namespaces, cgroups & security](namespaces-cgroups-and-security/)
* [System information & misc](system-information-and-misc/)
