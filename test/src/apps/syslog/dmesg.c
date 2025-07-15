// SPDX-License-Identifier: MPL-2.0

// Enhanced dmesg implementation for Asterinas
// Displays kernel ring buffer messages with various formatting options

#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <sys/syscall.h>
#include <errno.h>
#include <string.h>
#include <ctype.h>

// Syslog action constants
#define SYSLOG_ACTION_READ_ALL 3
#define SYSLOG_ACTION_CLEAR 5
#define SYSLOG_ACTION_SIZE_BUFFER 10

// Log level constants
#define KERN_EMERG 0 // system is unusable
#define KERN_ALERT 1 // action must be taken immediately
#define KERN_CRIT 2 // critical conditions
#define KERN_ERR 3 // error conditions
#define KERN_WARNING 4 // warning conditions
#define KERN_NOTICE 5 // normal but significant condition
#define KERN_INFO 6 // informational
#define KERN_DEBUG 7 // debug-level messages

const char *level_names[] = { "emerg", "alert",	 "crit", "err",
			      "warn",  "notice", "info", "debug" };

void print_usage(const char *progname)
{
	printf("Usage: %s [options]\n", progname);
	printf("Display kernel ring buffer messages\n\n");
	printf("Options:\n");
	printf("  -c, --clear        Clear the ring buffer after printing\n");
	printf("  -T, --notime       Don't print timestamps\n");
	printf("  -t, --ctime        Show human readable timestamps (default: kernel timestamps)\n");
	printf("  -x, --decode       Show log level prefix names\n");
	printf("  -l LEVEL           Restrict output to given log level\n");
	printf("  -r, --raw          Show raw messages without any formatting\n");
	printf("  -s SIZE            Use buffer of specified size\n");
	printf("  -h, --help         Show this help message\n");
	printf("\nLog levels: 0=emerg, 1=alert, 2=crit, 3=err, 4=warn, 5=notice, 6=info, 7=debug\n");
}

// Parse log level from message
int parse_log_level(const char *msg, int *offset)
{
	if (msg[0] == '<' && isdigit(msg[1]) && msg[2] == '>') {
		*offset = 3;
		return msg[1] - '0';
	}
	*offset = 0;
	return -1; // no level found
}

// Format timestamp from kernel format
void format_timestamp(const char *timestamp_str, char *output,
		      int human_readable)
{
	if (!timestamp_str || !output)
		return;

	if (human_readable) {
		// For simplicity, just copy the timestamp as-is
		// In a real implementation, you'd convert to human-readable format
		strcpy(output, timestamp_str);
	} else {
		strcpy(output, timestamp_str);
	}
}

void process_message(const char *msg, int show_time, int human_time,
		     int decode_level, int min_level, int raw_output)
{
	if (!msg || strlen(msg) == 0)
		return;

	int level_offset = 0;
	int level = parse_log_level(msg, &level_offset);

	// Filter by log level if specified
	if (min_level >= 0 && level >= 0 && level > min_level) {
		return;
	}

	const char *content = msg + level_offset;

	if (raw_output) {
		printf("%s", msg);
		return;
	}

	// Parse timestamp from content if it starts with [
	char timestamp[64] = "";
	const char *message_text = content;

	if (content[0] == '[') {
		const char *end_bracket = strchr(content + 1, ']');
		if (end_bracket) {
			int ts_len = end_bracket - content - 1;
			if (ts_len > 0 && ts_len < sizeof(timestamp) - 1) {
				strncpy(timestamp, content + 1, ts_len);
				timestamp[ts_len] = '\0';
				message_text = end_bracket + 1;
				// Skip leading space if present
				if (message_text[0] == ' ')
					message_text++;
			}
		}
	}

	// Build output line
	if (decode_level && level >= 0 && level <= 7) {
		printf("<%d>", level);
	}

	if (show_time && strlen(timestamp) > 0) {
		char formatted_ts[128];
		format_timestamp(timestamp, formatted_ts, human_time);
		printf("[%s] ", formatted_ts);
	}

	if (decode_level && level >= 0 && level <= 7) {
		printf("%s: ", level_names[level]);
	}

	printf("%s", message_text);

	// Add newline if not present
	if (strlen(message_text) > 0 &&
	    message_text[strlen(message_text) - 1] != '\n') {
		printf("\n");
	}
}

int main(int argc, char *argv[])
{
	int clear_buffer = 0;
	int show_time = 1;
	int human_time = 0;
	int decode_level = 0;
	int min_level = -1;
	int raw_output = 0;
	int buffer_size = 0;

	// Parse command line arguments
	for (int i = 1; i < argc; i++) {
		if (strcmp(argv[i], "-c") == 0 ||
		    strcmp(argv[i], "--clear") == 0) {
			clear_buffer = 1;
		} else if (strcmp(argv[i], "-T") == 0 ||
			   strcmp(argv[i], "--notime") == 0) {
			show_time = 0;
		} else if (strcmp(argv[i], "-t") == 0 ||
			   strcmp(argv[i], "--ctime") == 0) {
			human_time = 1;
		} else if (strcmp(argv[i], "-x") == 0 ||
			   strcmp(argv[i], "--decode") == 0) {
			decode_level = 1;
		} else if (strcmp(argv[i], "-r") == 0 ||
			   strcmp(argv[i], "--raw") == 0) {
			raw_output = 1;
		} else if (strcmp(argv[i], "-l") == 0) {
			if (i + 1 < argc) {
				min_level = atoi(argv[++i]);
				if (min_level < 0 || min_level > 7) {
					fprintf(stderr,
						"Invalid log level: %d (must be 0-7)\n",
						min_level);
					return 1;
				}
			} else {
				fprintf(stderr,
					"Option -l requires a level argument\n");
				return 1;
			}
		} else if (strcmp(argv[i], "-s") == 0) {
			if (i + 1 < argc) {
				buffer_size = atoi(argv[++i]);
				if (buffer_size <= 0) {
					fprintf(stderr,
						"Invalid buffer size: %d\n",
						buffer_size);
					return 1;
				}
			} else {
				fprintf(stderr,
					"Option -s requires a size argument\n");
				return 1;
			}
		} else if (strcmp(argv[i], "-h") == 0 ||
			   strcmp(argv[i], "--help") == 0) {
			print_usage(argv[0]);
			return 0;
		} else {
			printf("Unknown option: %s\n", argv[i]);
			print_usage(argv[0]);
			return 1;
		}
	}

	// Get kernel log buffer size if not specified
	if (buffer_size == 0) {
		long size =
			syscall(SYS_syslog, SYSLOG_ACTION_SIZE_BUFFER, NULL, 0);
		if (size <= 0) {
			fprintf(stderr,
				"dmesg: Unable to get kernel buffer size: %s\n",
				strerror(errno));
			return 1;
		}
		buffer_size = size;
	}

	// Allocate buffer
	char *buffer = malloc(buffer_size);
	if (!buffer) {
		fprintf(stderr, "dmesg: Cannot allocate memory\n");
		return 1;
	}

	// Read all kernel messages
	long bytes_read = syscall(SYS_syslog, SYSLOG_ACTION_READ_ALL, buffer,
				  buffer_size - 1);
	if (bytes_read < 0) {
		fprintf(stderr, "dmesg: Unable to read kernel buffer: %s\n",
			strerror(errno));
		free(buffer);
		return 1;
	}

	// Process and print the messages
	if (bytes_read > 0) {
		buffer[bytes_read] = '\0'; // Ensure null termination

		if (raw_output) {
			printf("%s", buffer);
		} else {
			// Process line by line
			char *line = strtok(buffer, "\n");
			while (line != NULL) {
				process_message(line, show_time, human_time,
						decode_level, min_level,
						raw_output);
				line = strtok(NULL, "\n");
			}
		}
	}

	// Clear buffer if requested
	if (clear_buffer) {
		long result = syscall(SYS_syslog, SYSLOG_ACTION_CLEAR, NULL, 0);
		if (result < 0) {
			fprintf(stderr,
				"dmesg: Unable to clear kernel buffer: %s\n",
				strerror(errno));
			free(buffer);
			return 1;
		}
	}

	free(buffer);
	return 0;
}