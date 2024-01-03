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
    int server_fd, accepted_fd, len;
    struct sockaddr_un server_addr, client_addr;
    char buf[BUFFER_SIZE];

    // 创建Server Socket
    server_fd = socket(AF_UNIX, SOCK_STREAM, 0);
    if (server_fd == -1) {
        perror("socket");
        exit(EXIT_FAILURE);
    }

    // 绑定Socket地址
    memset(&server_addr, 0, sizeof(server_addr));
    server_addr.sun_family = AF_UNIX;
    strncpy(server_addr.sun_path, SOCKET_NAME, sizeof(server_addr.sun_path) - 1);

    if (bind(server_fd, (struct sockaddr*)&server_addr, sizeof(server_addr)) == -1) {
        perror("bind");
        exit(EXIT_FAILURE);
    }

    // 监听连接请求
    if (listen(server_fd, 5) == -1) {
        perror("listen");
        exit(EXIT_FAILURE);
    }

    printf("Server is listening...\n");

    // 接收连接请求
    len = sizeof(client_addr);
    accepted_fd = accept(server_fd, (struct sockaddr*)&client_addr, &len);
    if (accepted_fd == -1) {
        perror("accept");
        exit(EXIT_FAILURE);
    }

    int addrlen = sizeof(client_addr);
    int rc = getpeername(accepted_fd, (struct sockaddr *)&client_addr,
                &addrlen);
    if (rc == -1) {
        perror("getpeername");
        exit(EXIT_FAILURE);
    }
    printf("accepted client path = %s\n", client_addr.sun_path);    

    printf("Server is connected to client\n");
    char* mesg = "Hello from unix socket server";
    write(accepted_fd, mesg, strlen(mesg));
    // 读取客户端发送的数据并打印
    memset(buf, 0, BUFFER_SIZE);
    if (read(accepted_fd, buf, BUFFER_SIZE) == -1) {
        perror("read");
        exit(EXIT_FAILURE);
    }
    printf("Server Received: %s\n", buf);

    // 关闭Socket
    close(accepted_fd);
    close(server_fd);
    unlink(SOCKET_NAME);

    return 0;
}