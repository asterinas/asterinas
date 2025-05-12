// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <stdlib.h>
#include <fcntl.h>
#include <unistd.h>
#include <linux/input.h>
#include <string.h>
#include <errno.h>
#include <signal.h>

volatile sig_atomic_t stop = 0; // Flag to indicate when to stop the program

void handle_signal(int signal) {
    stop = 1; // Set the stop flag when a signal is received
}

int main() {
    char device[32] = "/dev/input/event";
    int input;

    while (1) {
        printf("Please input a number to specify input/device: (0 -- 2)\n");
        if (scanf("%d", &input) != 1) {
            int c;
            while ((c = getchar()) != '\n' && c != EOF);
            printf("Invalid input! Please input a number\n");
            continue;
        }

        if (input == 0 || input == 1 || input == 2) {
            char number[2];
            sprintf(number, "%d", input);
            strcat(device, number);
            printf("The device file is: %s\n", device);
            break;
        } else {
            printf("Invalid number! Please input 0 -- 2\n");
        }
    }


    int fd = open(device, O_RDONLY);
    if (fd == -1) {
        perror("Failed to open device");
        return EXIT_FAILURE;
    }

    printf("Successfully opened %s\n", device);

    // Register signal handlers for SIGINT (Ctrl+C) and SIGTSTP (Ctrl+Z)
    signal(SIGINT, handle_signal);
    signal(SIGTSTP, handle_signal);

    struct input_event ev;
    while (!stop) { // Run until the stop flag is set
        ssize_t bytes = read(fd, &ev, sizeof(ev));
        if (bytes == -1) {
            if (errno == EINTR) {
                continue; // Interrupted by a signal, retry
            }
            perror("Failed to read event");
            break;
        } else if (bytes == 0) {
            continue;
        } else if (bytes != sizeof(ev)) {
            fprintf(stderr, "Unexpected read size: %zd, sizeof(ev): %zd\n", bytes, sizeof(ev));
            break;
        }

        // Successfully read an event
        printf("Event: time %ld.%06ld, type %d, code %d, value %d\n",
               ev.time.tv_sec, ev.time.tv_usec, ev.type, ev.code, ev.value);
    }

    if (close(fd) == -1) {
        perror("Failed to close device");
        return EXIT_FAILURE;
    }

    printf("Device closed successfully\n");
    return EXIT_SUCCESS;
}