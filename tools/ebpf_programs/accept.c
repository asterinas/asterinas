#include <stddef.h>

// Metadata header layout (must match kernel netfilter serialization)
struct ebpf_metadata {
    unsigned char version;          // offset 0
    unsigned char family;           // offset 1 (4=IPv4, 6=IPv6, 0=unknown)
    unsigned short dst_port;        // offset 2 (big-endian)
    unsigned short src_port;        // offset 4 (big-endian)
    unsigned char dst_addr[16];     // offset 6
    unsigned char src_addr[16];     // offset 22
} __attribute__((packed));

__attribute__((section("prog"), used))
int ebpf_accept(void *ctx) {
    (void)ctx;
    return 1;
}
