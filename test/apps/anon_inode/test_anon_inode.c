// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/stat.h>
#include <sys/epoll.h>
#include <sys/signalfd.h>
#include <sys/eventfd.h>
#include <signal.h>
#include <string.h>
#include <errno.h>

// Function to print file descriptor information
void print_fd_info(int fd, const char *fd_type) {
    char path[256];
    char link[256];
    struct stat st;

    // Construct file descriptor path
    snprintf(path, sizeof(path), "/proc/self/fd/%d", fd);

    // Read symbolic link contents
    ssize_t len = readlink(path, link, sizeof(link) - 1);
    if (len == -1) {
        perror("readlink");
        return;
    }
    link[len] = '\0';

    // Get file metadata
    if (fstat(fd, &st) == -1) {
        perror("fstat");
        return;
    }

    // Print information
    printf("\n%s (fd %d):\n", fd_type, fd);
    printf("  Symbolic link contents: %s\n", link);
    printf("  File metadata:\n");
    printf("    Inode: %ld\n", (long)st.st_ino);
    printf("    Mode: %o\n", st.st_mode & 0777);
    printf("    Size: %ld bytes\n", (long)st.st_size);
    printf("    UID: %d\n", st.st_uid);
    printf("    GID: %d\n", st.st_gid);
}

int main() {
    int epoll_fd, signal_fd, event_fd;
    
    // Test epoll_create
    epoll_fd = epoll_create1(0);
    if (epoll_fd == -1) {
        perror("epoll_create1");
        exit(EXIT_FAILURE);
    }
    print_fd_info(epoll_fd, "epoll");

    // Test signalfd
    sigset_t mask;
    sigemptyset(&mask);
    sigaddset(&mask, SIGUSR1);
    signal_fd = signalfd(-1, &mask, 0);
    if (signal_fd == -1) {
        perror("signalfd");
        close(epoll_fd);
        exit(EXIT_FAILURE);
    }
    print_fd_info(signal_fd, "signalfd");

    // Test eventfd
    event_fd = eventfd(0, 0);
    if (event_fd == -1) {
        perror("eventfd");
        close(epoll_fd);
        close(signal_fd);
        exit(EXIT_FAILURE);
    }
    print_fd_info(event_fd, "eventfd");

    // Cleanup
    close(epoll_fd);
    close(signal_fd);
    close(event_fd);

    return 0;
}