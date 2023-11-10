#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <netinet/ip.h>
#include <netinet/udp.h>
#include <arpa/inet.h>
#include <unistd.h>

#define DEST_IP "127.0.0.1"
#define DEST_PORT 1234

int main() {
    int sock_fd;
    struct sockaddr_in dest_addr;
    char *packet_data = "Hello from raw client!";
    struct udphdr *udp_header;

    // Allocate space for both the UDP header and the data payload
    char buffer[sizeof(struct udphdr) + strlen(packet_data)];
    memset(buffer, 0, sizeof(buffer));

    // Point udp_header to the buffer's location
    udp_header = (struct udphdr *)buffer;
    udp_header->source = htons(12345); // Arbitrary source port
    udp_header->dest = htons(DEST_PORT);
    udp_header->len = htons(sizeof(struct udphdr) + strlen(packet_data)); // UDP header size + data size

    // Copy the packet data after the header in the buffer
    memcpy(buffer + sizeof(struct udphdr), packet_data, strlen(packet_data));

    // Create a raw socket using IPPROTO_UDP to let the kernel handle the IP header
    if ((sock_fd = socket(AF_INET, SOCK_RAW, IPPROTO_UDP)) == -1) {
        perror("socket creation failed");
        exit(EXIT_FAILURE);
    }

    // Set destination address
    memset(&dest_addr, 0, sizeof(dest_addr));
    dest_addr.sin_family = AF_INET;
    dest_addr.sin_port = htons(DEST_PORT); // The port on which the server is listening
    dest_addr.sin_addr.s_addr = inet_addr(DEST_IP);

    // Send the packet with the UDP header and the payload
    if (sendto(sock_fd, buffer, sizeof(buffer), 0, (struct sockaddr *)&dest_addr, sizeof(dest_addr)) == -1) {
        perror("sendto failed");
        close(sock_fd);
        exit(EXIT_FAILURE);
    }

    printf("Packet sent to %s\n", DEST_IP);

    // Close the socket
    close(sock_fd);
    return 0;
}
