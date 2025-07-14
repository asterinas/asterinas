// SPDX-License-Identifier: MPL-2.0

// Simple dmesg implementation for Asterinas
// Displays kernel ring buffer messages similar to Linux dmesg

#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <sys/syscall.h>
#include <errno.h>
#include <string.h>

// Syslog action constants
#define SYSLOG_ACTION_READ_ALL      3
#define SYSLOG_ACTION_CLEAR         5
#define SYSLOG_ACTION_SIZE_BUFFER   10

void print_usage(const char *progname) {
    printf("Usage: %s [options]\n", progname);
    printf("Display kernel ring buffer messages\n\n");
    printf("Options:\n");
    printf("  -c, --clear        Clear the ring buffer after printing\n");
    printf("  -h, --help         Show this help message\n");
}

int main(int argc, char *argv[]) {
    int clear_buffer = 0;
    
    // Parse command line arguments
    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "-c") == 0 || strcmp(argv[i], "--clear") == 0) {
            clear_buffer = 1;
        } else if (strcmp(argv[i], "-h") == 0 || strcmp(argv[i], "--help") == 0) {
            print_usage(argv[0]);
            return 0;
        } else {
            printf("Unknown option: %s\n", argv[i]);
            print_usage(argv[0]);
            return 1;
        }
    }
    
    // Get kernel log buffer size
    long buffer_size = syscall(SYS_syslog, SYSLOG_ACTION_SIZE_BUFFER, NULL, 0);
    if (buffer_size <= 0) {
        fprintf(stderr, "dmesg: Unable to get kernel buffer size: %s\n", strerror(errno));
        return 1;
    }
    
    // Allocate buffer
    char *buffer = malloc(buffer_size);
    if (!buffer) {
        fprintf(stderr, "dmesg: Cannot allocate memory\n");
        return 1;
    }
    
    // Read all kernel messages
    long bytes_read = syscall(SYS_syslog, SYSLOG_ACTION_READ_ALL, buffer, buffer_size - 1);
    if (bytes_read < 0) {
        fprintf(stderr, "dmesg: Unable to read kernel buffer: %s\n", strerror(errno));
        free(buffer);
        return 1;
    }
    
    // Print the messages
    if (bytes_read > 0) {
        buffer[bytes_read] = '\0';  // Ensure null termination
        printf("%s", buffer);
    }
    
    // Clear buffer if requested
    if (clear_buffer) {
        long result = syscall(SYS_syslog, SYSLOG_ACTION_CLEAR, NULL, 0);
        if (result < 0) {
            fprintf(stderr, "dmesg: Unable to clear kernel buffer: %s\n", strerror(errno));
            free(buffer);
            return 1;
        }
    }
    
    free(buffer);
    return 0;
} 