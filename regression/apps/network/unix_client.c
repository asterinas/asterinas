// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/types.h>
#include <sys/socket.h>
#include <sys/un.h>

#define SOCKET_NAME "/tmp/test.sock"
#define BUFFER_SIZE 128

int main() {
    int client_fd, len;
    struct sockaddr_un server_addr, peer_addr;
    char buf[BUFFER_SIZE];

    // Create Client Socket
    client_fd = socket(AF_UNIX, SOCK_STREAM, 0);
    if (client_fd == -1) {
        perror("socket");
        exit(EXIT_FAILURE);
    }

    // Connect Server
    memset(&server_addr, 0, sizeof(server_addr));
    server_addr.sun_family = AF_UNIX;
    strncpy(server_addr.sun_path, SOCKET_NAME, sizeof(server_addr.sun_path) - 1);

    if (connect(client_fd, (struct sockaddr*)&server_addr, sizeof(server_addr)) == -1) {
        perror("connect");
        exit(EXIT_FAILURE);
    }

    int addrlen = sizeof(peer_addr);
    int rc = getpeername(client_fd, (struct sockaddr *)&peer_addr,
                &addrlen);
    if (rc == -1) {
        perror("getpeername");
        exit(EXIT_FAILURE);
    }
    printf("server path = %s\n", peer_addr.sun_path);
    // Read from server
    memset(buf, 0, BUFFER_SIZE);
    if (read(client_fd, buf, BUFFER_SIZE) == -1) {
        perror("read");
        exit(EXIT_FAILURE);
    }
    
    printf("Client Received: %s\n", buf);

    // Send message to server
    printf("Client is connected to server\n");
    char* mesg = "Hello from unix socket client";
    if (write(client_fd, mesg, strlen(mesg)) == -1) {
        perror("write");
        exit(EXIT_FAILURE);
    }

    // Close socket
    close(client_fd);

    return 0;
}