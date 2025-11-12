# Networking & Sockets

<!--
Put system calls such as

socket, socketpair, bind, listen, accept, connect, getsockname, getpeername, 
sendto, recvfrom, sendmsg, recvmsg, shutdown, setsockopt, getsockopt, 
sendmmsg, recvmmsg, accept4, recvmsg, and socketcall
under this category.
-->

## Socket Creation

### `socket`

Supported functionality in SCML:

```c
{{#include socket.scml}}
```

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/socket.2.html).

### `socketpair`

Supported functionality in SCML:

```c
{{#include socketpair.scml}}
```

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/socketpair.2.html).

## Socket Setup

### `bind`

Supported functionality in SCML:

```c
{{#include bind.scml}}
```

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/bind.2.html).

### `connect`

Supported functionality in SCML:

```c
{{#include connect.scml}}
```

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/connect.2.html).

## Socket Communication

### `sendto` and `sendmsg`

Supported functionality in SCML:

```c
{{#include sendto_and_sendmsg.scml}}
```

Unsupported flags:
* `MSG_CONFIRM`
* `MSG_DONTROUTE`
* `MSG_DONTWAIT`
* `MSG_EOR`
* `MSG_MORE`
* `MSG_CONFIRM`
* `MSG_NOSIGNAL`
* `MSG_OOB`
* `MSG_FASTOPEN`

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/sendto.2.html).

### `recvfrom` and `recvmsg`

Supported functionality in SCML:

```c
{{#include recvfrom_and_recvmsg.scml}}
```

Partially-supported flags:
* `MSG_PEEK` because it is only supported in netlink socket

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/recvfrom.2.html).

## Socket Options

### `getsockopt` and `setsockopt`

Supported functionality in SCML:

```c
{{#include getsockopt_and_setsockopt.scml}}
```

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/getsockopt.2.html).
