// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <stdlib.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <unistd.h>

int main() {
    int sockfd;
    int option;

    // Create tcp socket
    sockfd = socket(AF_INET, SOCK_STREAM, 0);
    if (sockfd < 0) {
        perror("Socket creation failed");
        exit(EXIT_FAILURE);
    }

    // Get send buffer size
    int sendbuf;
    socklen_t sendbuf_len = sizeof(sendbuf);
    if (getsockopt(sockfd, SOL_SOCKET, SO_SNDBUF, &sendbuf, &sendbuf_len) < 0 || sendbuf_len != sizeof(sendbuf)) {
        perror("Getting SO_SNDBUF option failed");
        exit(EXIT_FAILURE);
    }

    int error;
    socklen_t error_len = sizeof(error);
    if (getsockopt(sockfd, SOL_SOCKET, SO_ERROR, &error, &error_len ) < 0 || error_len != sizeof(error) || error != 0) {
        perror("Getting SO_SNDBUF option failed");
        exit(EXIT_FAILURE);
    }

    // Disable Nagle algorithm
    option = 1;
    if (setsockopt(sockfd, IPPROTO_TCP, TCP_NODELAY, &option, sizeof(option)) < 0) {
        perror("Setting TCP_NODELAY option failed");
        exit(EXIT_FAILURE);
    }

    // Enable reuse addr
    option = 1;
    if (setsockopt(sockfd, SOL_SOCKET, SO_REUSEADDR, &option, sizeof(option)) < 0) {
        perror("Setting SO_REUSEADDR option failed");
        exit(EXIT_FAILURE);
    }
    
    // Print new value
    int nagle;
    socklen_t nagle_len = sizeof(nagle);
    if (getsockopt(sockfd, IPPROTO_TCP, TCP_NODELAY, &nagle, &nagle_len) < 0 || nagle != 1) {
        perror("Getting TCP_NODELAY option failed.");
        exit(EXIT_FAILURE);
    } 

    int reuseaddr;
    socklen_t reuseaddr_len = sizeof(reuseaddr);
    if (getsockopt(sockfd, SOL_SOCKET, SO_REUSEADDR, &reuseaddr, &reuseaddr_len) < 0 || reuseaddr != 1) {
        perror("Getting SO_REUSEADDR option failed.");
        exit(EXIT_FAILURE);
    }

    // Close socket
    close(sockfd);

    return 0;
}