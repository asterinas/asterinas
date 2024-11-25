// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <signal.h>
#include <unistd.h>
#include <stdlib.h>
#include <stdatomic.h>
#include <assert.h>

volatile sig_atomic_t signaled = 0; // Volatile to avoid optimization issues

void read_df_flag_and_assert(int expect)
{
	unsigned long rflags;
	asm volatile("pushfq\n\t" // Push the RFLAGS register onto the stack
		     "popq %0\n\t" // Pop it into a variable
		     : "=r"(rflags)); // Output constraint
	int df = (rflags >> 10) & 1; // The DF flag is the 10th bit of RFLAGS
	assert(df == expect);
}

void sigint_handler(int signum)
{
	read_df_flag_and_assert(0);
	signaled = 1; // Update volatile variable to notify main
}

// A regression test for the DF flag bug fixed in https://github.com/asterinas/asterinas/pull/1638.
// Also checks PR #1637 when dynamically linked. If calling a function from a shared library mapped
// via mmap triggers a page fault and #1637 is not fixed, it may cause kernel panic.
int main()
{
	// Check DF flag in main before setting it
	read_df_flag_and_assert(0);
	asm volatile("std"); // Set DF flag
	read_df_flag_and_assert(1);

	// Set up the SIGINT signal handler
	struct sigaction sa;
	sa.sa_handler = sigint_handler;
	sa.sa_flags = 0;
	sigemptyset(&sa.sa_mask);
	sigaction(SIGINT, &sa, NULL);

	// Send SIGINT to itself
	if (kill(getpid(), SIGINT) != 0) {
		perror("kill");
		_exit(EXIT_FAILURE);
	}

	// Wait for the signal handler to update the variable
	while (!signaled) {
		// Spin until the signal is handled
	}

	// Check DF flag in main after signal handling
	read_df_flag_and_assert(1);

	_exit(0); // Exit immediately without cleanup because DF is set
}
