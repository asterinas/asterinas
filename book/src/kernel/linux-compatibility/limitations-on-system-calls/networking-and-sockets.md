# Networking & Sockets

<!--
Put system calls such as

socket, socketpair, bind, listen, accept, connect, getsockname, getpeername, 
sendto, recvfrom, sendmsg, recvmsg, shutdown, setsockopt, getsockopt, 
sendmmsg, recvmmsg, accept4, recvmsg, and socketcall
under this category.
-->

## `socket`

Supported functionality in SCML:

```c
// Optional flags for socket type
opt_type_flags = SOCK_NONBLOCK | SOCK_CLOEXEC;

// Create a UNIX socket
socket(
    family = AF_UNIX,
    type = SOCK_STREAM | SOCK_SEQPACKET | <opt_type_flags>,
    protocol = 0
);

// Create an IPv4 socket (TCP or UDP)
socket(
    family = AF_INET, 
    type = SOCK_STREAM | SOCK_DGRAM | <opt_type_flags>,
    protocol = IPPROTO_IP | IPPROTO_TCP | IPPROTO_UDP
);

// Create a netlink socket
socket(
    family = AF_NETLINK, 
    type = SOCK_RAW | SOCK_DGRAM | <opt_type_flags>,
    protocol = NETLINK_ROUTE | NETLINK_KOBJECT_UEVENT
);

// Create a VSOCK socket
socket(
    family = AF_VSOCK, 
    type = SOCK_STREAM | <opt_type_flags>,
    protocol = 0
);
```

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/socket.2.html).

## `socketpair`

Supported functionality in SCML:

```c
// Create a pair of connected UNIX sockets
socketpair(
    family = AF_UNIX,
    type = SOCK_STREAM | SOCK_SEQPACKET | <opt_type_flags>,
    protocol = 0,
    sv
);
```

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/socketpair.2.html).

## `bind`

Supported functionality in SCML:

```c
struct sockaddr = {
    sa_family = AF_INET | AF_UNIX | AF_NETLINK | AF_VSOCK,
    ..
};

// Bind a socket to an address
bind(
    sockfd, addr = <sockaddr>, addrlen
);
```

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/bind.2.html).

## `connect`

Supported functionality in SCML:

```c
// Connect to a peer socket
connect(
    sockfd, addr = <sockaddr>, addrlen
);
```

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/connect.2.html).

## `sendto` and `sendmsg`

Supported functionality in SCML:

```c
// Send message on a socket
sendto(
    sockfd, buf, len,
    flags = 0,
    dest_addr = <sockaddr>,
    addrlen
);

// Send message using scatter-gather buffers and ancillary data
sendmsg(
    sockfd,
    msg = {
        msg_name = <sockaddr>,
        msg_control = NULL,
        ..
    },
    flags = 0
);
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

## `recvfrom` and `recvmsg`

Supported functionality in SCML:

```c
// Receive message from a socket
recvfrom(
    sockfd, buf, size,
    flags = 0,
    src_addr, addrlen
);

// Receive message using scatter-gather buffers and ancillary data
recvmsg(
    sockfd,
    msg,
    flags = 0
);
```

Partially-supported flags:
* `MSG_PEEK` because it is only supported in netlink socket

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/recvfrom.2.html).

## `getsockopt` and `setsockopt`

Supported functionality in SCML:

```c
socket_options = SO_SNDBUF | SO_RCVBUF | SO_REUSEADDR | SO_REUSEPORT |
                 SO_PRIORITY | SO_LINGER | SO_PASSCRED | SO_KEEPALIVE |
                 SO_SNDBUFFORCE | SO_RCVBUFFORCE | SO_ERROR |
                 SO_PEERCRED | SO_ACCEPTCONN | SO_PEERGROUPS;

ip_options = IP_TOS | IP_TTL | IP_HDRINCL;

tcp_options = TCP_NODELAY | TCP_MAXSEG | TCP_KEEPIDLE | TCP_SYNCNT |
              TCP_DEFER_ACCEPT | TCP_WINDOW_CLAMP | TCP_CONGESTION |
              TCP_USER_TIMEOUT | TCP_INQ;

// Get options at socket level
getsockopt(
    sockfd, level = SOL_SOCKET,
    optname = <socket_options>,
    optval, optlen
);

// Get options at IP level
getsockopt(
    sockfd, level = SOL_IP,
    optname = <ip_options>,
    optval, optlen
);

// Get options at TCP level
getsockopt(
    sockfd, level = SOL_TCP,
    optname = <tcp_options>,
    optval, optlen
);

// Set options at socket level
setsockopt(
    sockfd, level = SOL_SOCKET,
    optname = <socket_options>,
    optval, optlen
);

// Set options at IP level
setsockopt(
    sockfd, level = SOL_IP,
    optname = <ip_options>,
    optval, optlen
);

// Set options at TCP level
setsockopt(
    sockfd, level = SOL_TCP,
    optname = <tcp_options>,
    optval, optlen
);

// Set options at netlink level
setsockopt(
    sockfd, level = SOL_NETLINK,
    optname = NETLINK_ADD_MEMBERSHIP | NETLINK_DROP_MEMBERSHIP,
    optval, optlen
);
```

For more information,
see [the man page](https://man7.org/linux/man-pages/man2/getsockopt.2.html).