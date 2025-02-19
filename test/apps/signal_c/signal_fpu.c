// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <stdlib.h>
#include <time.h>
#include <unistd.h>
#include <signal.h>
#include <stdint.h>
#include <stdbool.h>

volatile bool ok = true;

void set_xmm8(uint64_t xmm8_low, uint64_t xmm8_high)
{
	// Create a buffer to hold 128 bits (16 bytes)
	uint8_t xmm8_buffer[16] = {
		(uint8_t)(xmm8_low),	    (uint8_t)(xmm8_low >> 8),
		(uint8_t)(xmm8_low >> 16),  (uint8_t)(xmm8_low >> 24),
		(uint8_t)(xmm8_low >> 32),  (uint8_t)(xmm8_low >> 40),
		(uint8_t)(xmm8_low >> 48),  (uint8_t)(xmm8_low >> 56),
		(uint8_t)(xmm8_high),	    (uint8_t)(xmm8_high >> 8),
		(uint8_t)(xmm8_high >> 16), (uint8_t)(xmm8_high >> 24),
		(uint8_t)(xmm8_high >> 32), (uint8_t)(xmm8_high >> 40),
		(uint8_t)(xmm8_high >> 48), (uint8_t)(xmm8_high >> 56)
	};

	// Load xmm8 with the 128-bit value from xmm8_buffer
	asm volatile(
		"movdqu %0, %%xmm8" // Load xmm8 with the value in xmm8_buffer
		:
		: "m"(xmm8_buffer) // Input: xmm8_buffer
	);
}

void print_assert_xmm8(const char *prompt, uint64_t expected_xmm8_low,
		       uint64_t expected_xmm8_high)
{
	// Read xmm8 in the signal handler (128 bits into a buffer)
	uint8_t xmm8_buffer[16]; // 128 bits
	asm volatile(
		"movdqu %%xmm8, %0" // Move the entire xmm8 register to memory
		: "=m"(xmm8_buffer) // Output the value to xmm8_buffer
	);

	uint64_t xmm8_low, xmm8_high;
	// Now extract the low and high 64-bits from the buffer
	xmm8_low = *(uint64_t *)&xmm8_buffer[0]; // First 64 bits (low part)
	xmm8_high = *(uint64_t *)&xmm8_buffer[8]; // Second 64 bits (high part)

	// Output xmm8 value (NOTE: dprintf might destroy xmm8, it can be only used once!)
	dprintf(STDOUT_FILENO, "%s: xmm8 = 0x%016lx%016lx\n", prompt, xmm8_high,
		xmm8_low);

	// Assert
	if (expected_xmm8_low != xmm8_low || expected_xmm8_high != xmm8_high) {
		ok = false;
	}
}

// Signal handler
void signal_handler(int signum)
{
	print_assert_xmm8("In signal", 0, 0);
	set_xmm8(0x1234567890abcdef, 0xabcdef1234567890);
}

int main()
{
	uint64_t xmm8_low = 0xcafebabecafebabe;
	uint64_t xmm8_high = 0xdeadbeefdeadbeef;

	// Install signal handler
	signal(SIGUSR1, signal_handler);

	// Load xmm8 with the values
	set_xmm8(xmm8_low, xmm8_high);

	// Trigger the signal handler
	kill(getpid(), SIGUSR1);

	// After the signal handler returns, read and output xmm8 value again
	print_assert_xmm8("After signal", xmm8_low, xmm8_high);

	// Check result
	if (ok) {
		dprintf(STDOUT_FILENO, "All tests passed\n");
		return 0;
	} else {
		dprintf(STDOUT_FILENO, "ERROR: test failed!\n");
		exit(EXIT_FAILURE);
	}
}
