// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <unistd.h>

#define SERVER_IP "127.0.0.1"
#define SERVER_PORT 1234
#define BUFFER_SIZE 1024

int main() {
    int sock_fd;
    char buffer[BUFFER_SIZE];
    struct sockaddr_in serv_addr;

    // Create UDP socket
    if ((sock_fd = socket(AF_INET, SOCK_DGRAM, 0)) < 0) {
        perror("socket creation failed");
        exit(EXIT_FAILURE);
    }

    // Set server address
    memset(&serv_addr, 0, sizeof(serv_addr));
    serv_addr.sin_family = AF_INET;
    serv_addr.sin_port = htons(SERVER_PORT);
    if (inet_pton(AF_INET, SERVER_IP, &serv_addr.sin_addr) <= 0) {
        perror("invalid IP address");
        exit(EXIT_FAILURE);
    }

    // Send massage to server
    const char* message = "Hello world from udp client!";
    if (sendto(sock_fd, message, strlen(message), 0, (const struct sockaddr *)&serv_addr, sizeof(serv_addr)) < 0) {
        perror("sendto failed");
        exit(EXIT_FAILURE);
    }

    // Receive message from server
    struct sockaddr_in sender_addr;
    socklen_t sender_len = sizeof(sender_addr);
    int recv_len;
    if ((recv_len = recvfrom(sock_fd, buffer, BUFFER_SIZE, 0, (struct sockaddr *)&sender_addr, &sender_len)) < 0) {
        perror("recvfrom failed");
        exit(EXIT_FAILURE);
    }
    buffer[recv_len] = '\0';
    printf("Received %s from %s:%d\n", buffer, inet_ntoa(sender_addr.sin_addr), ntohs(sender_addr.sin_port));

    // close socket
    close(sock_fd);
    return 0;
}