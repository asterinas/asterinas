// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <fcntl.h>
#include <termios.h>
#include <pty.h>

int main() {
    int master, slave;
    char name[256];
    struct termios term;

    if (openpty(&master, &slave, name, NULL, NULL) == -1) {
        perror("openpty");
        exit(EXIT_FAILURE);
    }

    printf("slave name: %s\n", name);

    // Set pty slave terminal attributes
    tcgetattr(slave, &term);
    term.c_lflag &= ~(ICANON | ECHO);
    term.c_cc[VMIN] = 1;
    term.c_cc[VTIME] = 0;
    tcsetattr(slave, TCSANOW, &term);

    // Print to pty slave
    dprintf(slave, "Hello world!\n");

    // Read from pty slave
    char buf[256];
    ssize_t n = read(master, buf, sizeof(buf));
    if (n > 0) {
        printf("read %ld bytes from slave: %.*s", n, (int)n, buf);
    }

    // Write to pty master
    dprintf(master, "hello world from master\n");

    // Read from pty master
    char nbuf[256];
    ssize_t nn = read(slave, nbuf, sizeof(nbuf));
    if (nn > 0) {
        printf("read %ld bytes from master: %.*s", nn, (int)nn, nbuf);
    }

    close(master);
    close(slave);

    return 0;
}
