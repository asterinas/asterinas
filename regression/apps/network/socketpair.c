// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <sys/socket.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#define MESG1 "Hello from child"
#define MESG2 "Hello from parent"

int main() {
    int sockets[2], child;
    char buf[1024];
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, sockets) < 0) {
        perror("create socket pair");
        exit(1);
    }
    if ((child = fork()) == -1)
        perror("fork");
    else if (child) {
        // parent
        close(sockets[0]);
        if (read(sockets[1], buf, 1024) < 0)
            perror("read from child");
        printf("Receive from child: %s\n", buf);
        if (write(sockets[1], MESG2, sizeof(MESG2)) < 0) 
            perror("write to child");
        close(sockets[1]);
    } else {     
        // child
        close(sockets[1]);
        if (write(sockets[0], MESG1, sizeof(MESG1)) < 0)
            perror("write to parent");
        if (read(sockets[0], buf, 1024) < 0)
            perror("read from parent");
        printf("Receive from parent: %s\n", buf);
        close(sockets[0]);
    }
    return 0;
}