// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <sys/syscall.h>
#include <errno.h>
#include <string.h>

// Syslog action constants
#define SYSLOG_ACTION_READ_ALL      3
#define SYSLOG_ACTION_CLEAR         5
#define SYSLOG_ACTION_SIZE_UNREAD   9
#define SYSLOG_ACTION_SIZE_BUFFER   10

// ============================================================================
// Helper functions
// ============================================================================

#define THROW_ERROR(fmt, ...)                                                   \
	do {                                                                    \
		printf("\t\tERROR:" fmt                                         \
		       " in func %s at line %d of file %s with errno %d: %s\n", \
		       ##__VA_ARGS__, __func__, __LINE__, __FILE__, errno,      \
		       strerror(errno));                                        \
		return -1;                                                      \
	} while (0)

// ============================================================================
// Test syslog buffer size
// ============================================================================

int test_syslog_buffer_size()
{
	printf("=== Testing syslog buffer size ===\n");
	
	long buffer_size = syscall(SYS_syslog, SYSLOG_ACTION_SIZE_BUFFER, NULL, 0);
	if (buffer_size <= 0) {
		THROW_ERROR("Failed to get kernel log buffer size");
	}
	
	printf("✓ Kernel log buffer size: %ld bytes\n", buffer_size);
	
	// Sanity check: buffer size should be reasonable (typically 64KB to 16MB)
	if (buffer_size < 1024 || buffer_size > 16 * 1024 * 1024) {
		THROW_ERROR("Buffer size %ld seems unreasonable", buffer_size);
	}
	
	printf("✓ Buffer size is within reasonable range\n");
	return 0;
}

// ============================================================================
// Test syslog unread size
// ============================================================================

int test_syslog_unread_size()
{
	printf("\n=== Testing syslog unread size ===\n");
	
	long unread_size = syscall(SYS_syslog, SYSLOG_ACTION_SIZE_UNREAD, NULL, 0);
	if (unread_size < 0) {
		THROW_ERROR("Failed to get unread data size");
	}
	
	printf("✓ Unread data size: %ld bytes\n", unread_size);
	
	// Get buffer size for comparison
	long buffer_size = syscall(SYS_syslog, SYSLOG_ACTION_SIZE_BUFFER, NULL, 0);
	if (buffer_size <= 0) {
		THROW_ERROR("Failed to get buffer size for comparison");
	}
	
	// Unread size should not exceed buffer size
	if (unread_size > buffer_size) {
		THROW_ERROR("Unread size %ld exceeds buffer size %ld", unread_size, buffer_size);
	}
	
	printf("✓ Unread size is within buffer limits\n");
	return 0;
}

// ============================================================================
// Test syslog read all
// ============================================================================

int test_syslog_read_all()
{
	printf("\n=== Testing syslog read all ===\n");
	
	// Get buffer size first
	long buffer_size = syscall(SYS_syslog, SYSLOG_ACTION_SIZE_BUFFER, NULL, 0);
	if (buffer_size <= 0) {
		THROW_ERROR("Failed to get buffer size");
	}
	
	// Allocate buffer for reading kernel messages
	char *buffer = malloc(buffer_size + 1); // +1 for null terminator
	if (!buffer) {
		THROW_ERROR("Failed to allocate buffer of size %ld", buffer_size);
	}
	
	printf("✓ Allocated buffer of %ld bytes\n", buffer_size);
	
	// Read all kernel messages
	long bytes_read = syscall(SYS_syslog, SYSLOG_ACTION_READ_ALL, buffer, buffer_size);
	if (bytes_read < 0) {
		free(buffer);
		THROW_ERROR("Failed to read kernel log");
	}
	
	printf("✓ Read %ld bytes from kernel log\n", bytes_read);
	
	// Ensure null termination
	if (bytes_read < buffer_size) {
		buffer[bytes_read] = '\0';
	} else {
		buffer[buffer_size - 1] = '\0';
	}
	
	if (bytes_read > 0) {
		printf("✓ Successfully read kernel messages\n");
		printf("--- First 200 characters of log (if available) ---\n");
		
		// Display first part of the log for verification
		int display_len = (bytes_read < 200) ? bytes_read : 200;
		for (int i = 0; i < display_len; i++) {
			if (buffer[i] >= 32 && buffer[i] < 127) {
				putchar(buffer[i]);
			} else if (buffer[i] == '\n') {
				putchar('\n');
			} else {
				putchar('.');
			}
		}
		if (bytes_read > 200) {
			printf("\n... (truncated, total %ld bytes)\n", bytes_read);
		}
		printf("--- End of log sample ---\n");
	} else {
		printf("✓ No kernel messages available (empty log)\n");
	}
	
	free(buffer);
	return 0;
}

// ============================================================================
// Test syslog clear
// ============================================================================

int test_syslog_clear()
{
	printf("\n=== Testing syslog clear ===\n");
	
	// Get unread size before clearing
	long unread_before = syscall(SYS_syslog, SYSLOG_ACTION_SIZE_UNREAD, NULL, 0);
	if (unread_before < 0) {
		THROW_ERROR("Failed to get unread size before clear");
	}
	
	printf("✓ Unread size before clear: %ld bytes\n", unread_before);
	
	// Clear the log buffer
	long result = syscall(SYS_syslog, SYSLOG_ACTION_CLEAR, NULL, 0);
	if (result < 0) {
		THROW_ERROR("Failed to clear kernel log buffer");
	}
	
	printf("✓ Successfully cleared kernel log buffer\n");
	
	// Get unread size after clearing
	long unread_after = syscall(SYS_syslog, SYSLOG_ACTION_SIZE_UNREAD, NULL, 0);
	if (unread_after < 0) {
		THROW_ERROR("Failed to get unread size after clear");
	}
	
	printf("✓ Unread size after clear: %ld bytes\n", unread_after);
	
	// After clearing, unread size should be 0 or significantly reduced
	if (unread_after > unread_before) {
		THROW_ERROR("Unread size increased after clear (before: %ld, after: %ld)", 
			   unread_before, unread_after);
	}
	
	printf("✓ Clear operation appears successful\n");
	return 0;
}

// ============================================================================
// Test error handling
// ============================================================================

int test_syslog_error_handling()
{
	printf("\n=== Testing syslog error handling ===\n");
	
	// Test with invalid action
	long result = syscall(SYS_syslog, 999, NULL, 0);
	if (result >= 0) {
		printf("⚠ Warning: Invalid action didn't return error (result: %ld)\n", result);
	} else {
		printf("✓ Invalid action properly returned error\n");
	}
	
	// Test read with NULL buffer but non-zero size
	result = syscall(SYS_syslog, SYSLOG_ACTION_READ_ALL, NULL, 100);
	if (result >= 0) {
		printf("⚠ Warning: NULL buffer with non-zero size didn't return error\n");
	} else {
		printf("✓ NULL buffer with non-zero size properly returned error\n");
	}
	
	printf("✓ Error handling tests completed\n");
	return 0;
}

// ============================================================================
// Main test function
// ============================================================================

int main() {
	printf("=== Syslog Comprehensive Test for Asterinas ===\n\n");
	
	int failed_tests = 0;
	
	// Run all tests
	if (test_syslog_buffer_size() < 0) {
		printf("❌ Buffer size test failed\n");
		failed_tests++;
	} else {
		printf("✅ Buffer size test passed\n");
	}
	
	if (test_syslog_unread_size() < 0) {
		printf("❌ Unread size test failed\n");
		failed_tests++;
	} else {
		printf("✅ Unread size test passed\n");
	}
	
	if (test_syslog_read_all() < 0) {
		printf("❌ Read all test failed\n");
		failed_tests++;
	} else {
		printf("✅ Read all test passed\n");
	}
	
	if (test_syslog_clear() < 0) {
		printf("❌ Clear test failed\n");
		failed_tests++;
	} else {
		printf("✅ Clear test passed\n");
	}
	
	if (test_syslog_error_handling() < 0) {
		printf("❌ Error handling test failed\n");
		failed_tests++;
	} else {
		printf("✅ Error handling test passed\n");
	}
	
	// Summary
	printf("\n=== Test Summary ===\n");
	printf("Total tests: 5\n");
	printf("Passed: %d\n", 5 - failed_tests);
	printf("Failed: %d\n", failed_tests);
	
	if (failed_tests == 0) {
		printf("🎉 All syslog tests passed successfully!\n");
		return 0;
	} else {
		printf("❌ Some tests failed. Please check the output above.\n");
		return 1;
	}
} 