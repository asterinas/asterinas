# Networking & Sockets

<!--
Put system calls such as

socket, socketpair, bind, listen, accept, connect, getsockname, getpeername, 
sendto, recvfrom, sendmsg, recvmsg, shutdown, setsockopt, getsockopt, 
sendmmsg, recvmmsg, accept4, recvmsg, and socketcall
under this category.
-->

## `socket`


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