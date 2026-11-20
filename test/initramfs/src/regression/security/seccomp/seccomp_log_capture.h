/* SPDX-License-Identifier: MPL-2.0 */

#ifndef SECCOMP_LOG_CAPTURE_H
#define SECCOMP_LOG_CAPTURE_H

#include "seccomp_common.h"

struct log_capture {
	FILE *output;
	int console;
};

/* Kernel logs and terminal output use separate writers. Spool test output
 * until every notifying child has exited so serial records cannot be split
 * by a LOG_EXPECT line or a test result from userspace.
 */
static inline struct log_capture capture_log_output(void)
{
	FILE *output = tmpfile();
	CHECK_WITH(output != NULL, _ret);
	fflush(stdout);
	int console = CHECK(dup(STDOUT_FILENO));
	CHECK(dup2(fileno(output), STDOUT_FILENO));
	return (struct log_capture){ .output = output, .console = console };
}

static inline void flush_log_output(struct log_capture capture)
{
	fflush(stdout);
	CHECK(dup2(capture.console, STDOUT_FILENO));
	CHECK(close(capture.console));
	CHECK(fseek(capture.output, 0, SEEK_SET));
	int byte;
	while ((byte = fgetc(capture.output)) != EOF)
		putchar(byte);
	CHECK_WITH(ferror(capture.output), _ret == 0);
	CHECK(fclose(capture.output));
}

#endif
