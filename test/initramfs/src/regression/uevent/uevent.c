// SPDX-License-Identifier: MPL-2.0

/*
 * Userspace tests for device uevents.
 *
 * The tests read a device's sysfs `uevent` attribute, coldplug the existing
 * `null` device by writing `add`, send synthetic actions and arguments, and
 * check strict parsing. They also cover the 64-entry and 2048-byte
 * environment limits, group-1 multicast, `/sys/kernel/uevent_seqnum`, and
 * concurrent writers.
 */

#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <linux/netlink.h>
#include <poll.h>
#include <pthread.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <time.h>
#include <unistd.h>

#include "../common/test.h"

#define NULL_DEVPATH "/devices/virtual/mem/null"
#define UUID "12345678-ABCD-1234-abcd-123456789ABC"
#define MAX_ENV_FIELDS 64
#define MAX_ENV_BYTES 2048
#define EVENT_TIMEOUT_MS 2000
#define QUIET_TIMEOUT_MS 100
#define NUM_WRITERS 8

struct event {
	char data[4096];
	size_t fields[MAX_ENV_FIELDS];
	size_t num_fields;
	size_t env_bytes;
	uint64_t seqnum;
};

static const char *field(const struct event *event, size_t index)
{
	return event->data + event->fields[index];
}

static const char *value(const struct event *event, const char *key)
{
	size_t len = strlen(key);
	for (size_t i = 0; i < event->num_fields; i++) {
		const char *entry = field(event, i);
		if (!strncmp(entry, key, len) && entry[len] == '=')
			return entry + len + 1;
	}
	return NULL;
}

static bool has_value(const struct event *event, const char *key,
		      const char *expected)
{
	const char *actual = value(event, key);
	return actual && !strcmp(actual, expected);
}

static int parse_number(const char *text, uint64_t *number)
{
	if (*text < '0' || *text > '9')
		return -1;
	char *end;
	errno = 0;
	unsigned long long parsed = strtoull(text, &end, 10);
	if (errno || *end)
		return -1;
	*number = parsed;
	return 0;
}

static int parse_event(struct event *event, size_t len)
{
	if (!len || event->data[len - 1] != '\0')
		return -1;
	char *header = event->data;
	char *at = strchr(header, '@');
	if (!at || at == header || at[1] != '/')
		return -1;
	size_t offset = strlen(header) + 1;
	event->num_fields = 0;
	event->env_bytes = len - offset;
	if (event->env_bytes > MAX_ENV_BYTES)
		return -1;
	while (offset < len) {
		char *entry = event->data + offset;
		char *equals = strchr(entry, '=');
		if (!equals || equals == entry ||
		    event->num_fields == MAX_ENV_FIELDS)
			return -1;
		event->fields[event->num_fields++] = offset;
		offset += strlen(entry) + 1;
	}
	const char *base[] = { "ACTION", "DEVPATH", "SUBSYSTEM", "SEQNUM" };
	for (size_t i = 0; i < sizeof(base) / sizeof(base[0]); i++) {
		size_t count = 0, key_len = strlen(base[i]);
		for (size_t j = 0; j < event->num_fields; j++) {
			const char *entry = field(event, j);
			count += !strncmp(entry, base[i], key_len) &&
				 entry[key_len] == '=';
		}
		if (count != 1)
			return -1;
	}
	const char *action = value(event, "ACTION");
	if (strlen(action) != (size_t)(at - header) ||
	    strncmp(header, action, at - header) ||
	    strcmp(at + 1, value(event, "DEVPATH")) ||
	    !*value(event, "SUBSYSTEM"))
		return -1;
	return parse_number(value(event, "SEQNUM"), &event->seqnum);
}

static int64_t now_ms(void)
{
	struct timespec now;
	CHECK(clock_gettime(CLOCK_MONOTONIC, &now));
	return (int64_t)now.tv_sec * 1000 + now.tv_nsec / 1000000;
}

static int open_listener(unsigned groups)
{
	int fd = CHECK(socket(AF_NETLINK, SOCK_DGRAM | SOCK_NONBLOCK,
			      NETLINK_KOBJECT_UEVENT));
	struct sockaddr_nl address = {
		.nl_family = AF_NETLINK,
		.nl_groups = groups,
	};
	CHECK(bind(fd, (struct sockaddr *)&address, sizeof(address)));
	return fd;
}

static int receive_event(int fd, const char *action, const char *uuid,
			 struct event *event, int64_t deadline_ms)
{
	for (;;) {
		int64_t remaining_ms = deadline_ms - now_ms();
		if (remaining_ms <= 0) {
			errno = 0;
			return 0;
		}
		struct pollfd pfd = { .fd = fd, .events = POLLIN };
		int ready = poll(&pfd, 1, (int)remaining_ms);
		if (ready < 0 && errno == EINTR)
			continue;
		if (ready < 0)
			return -1;
		if (ready == 0) {
			errno = 0;
			return 0;
		}
		if (pfd.revents & (POLLERR | POLLHUP | POLLNVAL)) {
			errno = EIO;
			return -1;
		}
		struct sockaddr_nl sender = { 0 };
		socklen_t sender_len = sizeof(sender);
		ssize_t len = recvfrom(fd, event->data, sizeof(event->data),
				       MSG_TRUNC, (struct sockaddr *)&sender,
				       &sender_len);
		if (len < 0 && (errno == EAGAIN || errno == EINTR))
			continue;
		if (len < 0)
			return -1;
		if (sender.nl_pid != 0)
			continue;
		if ((size_t)len > sizeof(event->data) ||
		    parse_event(event, len)) {
			errno = EPROTO;
			return -1;
		}
		if (strcmp(value(event, "DEVPATH"), NULL_DEVPATH))
			continue;
		if (action && strcmp(value(event, "ACTION"), action))
			continue;
		if (uuid && (!value(event, "SYNTH_UUID") ||
			     strcmp(value(event, "SYNTH_UUID"), uuid)))
			continue;
		return 1;
	}
}

static ssize_t write_command(const void *command, size_t len)
{
	int fd = open("/sys" NULL_DEVPATH "/uevent", O_WRONLY);
	if (fd < 0)
		return -1;
	ssize_t result = write(fd, command, len);
	int saved_errno = errno;
	close(fd);
	errno = saved_errno;
	return result;
}

static uint64_t sequence(void)
{
	char text[64];
	int fd = CHECK(open("/sys/kernel/uevent_seqnum", O_RDONLY));
	ssize_t len = CHECK_WITH(read(fd, text, sizeof(text) - 1), _ret > 0);
	CHECK(close(fd));
	CHECK_WITH(text[len - 1], _ret == '\n');
	text[len - 1] = '\0';
	uint64_t number;
	CHECK(parse_number(text, &number));
	return number;
}

FN_SETUP(deadline)
{
	alarm(60);
}
END_SETUP()

FN_TEST(coldplug_null)
{
	char text[256];
	int fd = CHECK(open("/sys" NULL_DEVPATH "/uevent", O_RDONLY));
	ssize_t len = CHECK_WITH(read(fd, text, sizeof(text) - 1), _ret > 0);
	CHECK(close(fd));
	text[len] = '\0';
	struct stat statbuf;
	CHECK(stat("/dev/null", &statbuf));
	TEST_RES(S_ISCHR(statbuf.st_mode), _ret);
	TEST_RES(strstr(text, "DEVNAME=null\n"), _ret != NULL);

	fd = open_listener(1);
	TEST_RES(write_command("add\n", 4), _ret == 4);
	struct event event;
	int result = TEST_RES(receive_event(fd, "add", "0", &event,
					    now_ms() + EVENT_TIMEOUT_MS),
			      _ret == 1);
	if (result == 1) {
		TEST_RES(has_value(&event, "SUBSYSTEM", "mem"), _ret);
		TEST_RES(has_value(&event, "DEVNAME", "null"), _ret);
		char number[32];
		snprintf(number, sizeof(number), "%u", major(statbuf.st_rdev));
		TEST_RES(has_value(&event, "MAJOR", number), _ret);
		snprintf(number, sizeof(number), "%u", minor(statbuf.st_rdev));
		TEST_RES(has_value(&event, "MINOR", number), _ret);
	}
	CHECK(close(fd));
}
END_TEST()

FN_TEST(actions_and_ordered_arguments)
{
	int fd = open_listener(1);
	const char *actions[] = { "add",    "remove",  "change", "move",
				  "online", "offline", "bind",	 "unbind" };
	for (size_t i = 0; i < sizeof(actions) / sizeof(actions[0]); i++) {
		char command[128];
		int len = snprintf(command, sizeof(command), "%s %s\n",
				   actions[i], UUID);
		TEST_RES(write_command(command, len), _ret == len);
		struct event event;
		TEST_RES(receive_event(fd, actions[i], UUID, &event,
				       now_ms() + EVENT_TIMEOUT_MS),
			 _ret == 1);
	}
	const char command[] = "change " UUID " FOO=1 BAR=2 FOO=3\n";
	TEST_RES(write_command(command, sizeof(command) - 1),
		 _ret == sizeof(command) - 1);
	struct event event;
	int result = TEST_RES(receive_event(fd, "change", UUID, &event,
					    now_ms() + EVENT_TIMEOUT_MS),
			      _ret == 1);
	if (result == 1) {
		const char *expected[] = { "SYNTH_ARG_FOO=1", "SYNTH_ARG_BAR=2",
					   "SYNTH_ARG_FOO=3" };
		size_t found = 0;
		for (size_t i = 0; i < event.num_fields; i++) {
			if (!strncmp(field(&event, i), "SYNTH_ARG_", 10)) {
				TEST_RES(found < 3 && !strcmp(field(&event, i),
							      expected[found]),
					 _ret);
				found++;
			}
		}
		TEST_RES(found, _ret == 3);
	}
	CHECK(close(fd));
	fd = CHECK(open("/dev/null", O_RDWR));
	TEST_RES(write(fd, "x", 1), _ret == 1);
	TEST_RES(read(fd, &event, 1), _ret == 0);
	CHECK(close(fd));
}
END_TEST()

struct bad_input {
	const char *data;
	size_t len;
};

FN_TEST(strict_invalid_writes)
{
#define BAD(text) ((struct bad_input){ text, sizeof(text) - 1 })
	struct bad_input inputs[] = {
		BAD(""),
		BAD("ADD\n"),
		BAD("invalid\n"),
		BAD("add\r\n"),
		BAD("add\n\n"),
		BAD("add\n\0"),
		BAD("add\0\0"),
		BAD("ad\0d\n"),
		BAD(" add\n"),
		BAD("\tadd\n"),
		BAD("add  " UUID "\n"),
		BAD("add 12345678-1234-1234-1234-123456789ab\n"),
		BAD("add 12345678-1234-1234-1234-123456789abcd\n"),
		BAD("add 1234567-81234-1234-1234-123456789abc\n"),
		BAD("add 12345678_1234-1234-1234-123456789abc\n"),
		BAD("add 12345678-1234-1234-1234-123456789abg\n"),
		BAD("add " UUID " \n"),
		BAD("add " UUID " A=1  B=2\n"),
		BAD("add " UUID " A=1 \n"),
		BAD("add " UUID " =1\n"),
		BAD("add " UUID " A=\n"),
		BAD("add " UUID " A=1=2\n"),
		BAD("add " UUID " A_B=1\n"),
		BAD("add " UUID " A=1_2\n"),
		BAD("add " UUID " \xe4\xb8\xad=1\n"),
	};
#undef BAD
	int fd = open_listener(1);
	uint64_t before = sequence();
	for (size_t i = 0; i < sizeof(inputs) / sizeof(inputs[0]); i++)
		TEST(write_command(inputs[i].data, inputs[i].len), EINVAL,
		     _ret == -1);
	struct event event;
	TEST_RES(receive_event(fd, NULL, NULL, &event,
			       now_ms() + QUIET_TIMEOUT_MS),
		 _ret == 0);
	TEST_RES(sequence(), _ret == before);
	CHECK(close(fd));
}
END_TEST()

static size_t argument_command(char *command, size_t capacity, size_t count)
{
	size_t len = snprintf(command, capacity, "change %s", UUID);
	for (size_t i = 0; i < count; i++)
		len += snprintf(command + len, capacity - len, " K%02zu=1", i);
	command[len++] = '\n';
	return len;
}

static size_t value_command(char *command, size_t value_len)
{
	size_t len = strlen("change " UUID " A=");
	memcpy(command, "change " UUID " A=", len);
	memset(command + len, 'a', value_len);
	command[len + value_len] = '\n';
	return len + value_len + 1;
}

FN_TEST(final_environment_budgets)
{
	int fd = open_listener(1);
	char command[4096];
	struct event event;
	size_t len = argument_command(command, sizeof(command), 0);
	CHECK_WITH(write_command(command, len), _ret == len);
	CHECK_WITH(receive_event(fd, "change", UUID, &event,
				 now_ms() + EVENT_TIMEOUT_MS),
		   _ret == 1);
	size_t base_env_bytes =
		event.env_bytes - strlen(value(&event, "SEQNUM"));
	size_t max_args = MAX_ENV_FIELDS - event.num_fields;
	len = argument_command(command, sizeof(command), max_args);
	TEST_RES(write_command(command, len), _ret == len);
	int result = TEST_RES(receive_event(fd, "change", UUID, &event,
					    now_ms() + EVENT_TIMEOUT_MS),
			      _ret == 1);
	if (result == 1)
		TEST_RES(event.num_fields, _ret == MAX_ENV_FIELDS);
	char next_seqnum[32];
	int seq_digits = snprintf(next_seqnum, sizeof(next_seqnum), "%llu",
				  (unsigned long long)(sequence() + 1));
	size_t value_len = MAX_ENV_BYTES - base_env_bytes - seq_digits -
			   sizeof("SYNTH_ARG_A=");
	len = value_command(command, value_len);
	TEST_RES(write_command(command, len), _ret == len);
	result = TEST_RES(receive_event(fd, "change", UUID, &event,
					now_ms() + EVENT_TIMEOUT_MS),
			  _ret == 1);
	if (result == 1) {
		const char *argument = value(&event, "SYNTH_ARG_A");
		TEST_RES(argument && strlen(argument) == value_len &&
				 strspn(argument, "a") == value_len,
			 _ret);
		TEST_RES(event.env_bytes, _ret == MAX_ENV_BYTES);
	}
	uint64_t before = sequence();
	len = argument_command(command, sizeof(command), max_args + 1);
	TEST(write_command(command, len), ENOMEM, _ret == -1);
	len = argument_command(command, sizeof(command), 64);
	TEST(write_command(command, len), EINVAL, _ret == -1);
	len = value_command(command, MAX_ENV_BYTES - base_env_bytes + 1);
	TEST(write_command(command, len), ENOMEM, _ret == -1);
	TEST_RES(sequence(), _ret == before);

	seq_digits = snprintf(next_seqnum, sizeof(next_seqnum), "%llu",
			      (unsigned long long)(before + 1));
	value_len = MAX_ENV_BYTES - base_env_bytes - seq_digits -
		    sizeof("SYNTH_ARG_A=");
	len = value_command(command, value_len + 1);
	TEST(write_command(command, len), ENOMEM, _ret == -1);
	TEST_RES(sequence(), _ret == before + 1);
	TEST_RES(receive_event(fd, NULL, NULL, &event,
			       now_ms() + QUIET_TIMEOUT_MS),
		 _ret == 0);
	CHECK(close(fd));
}
END_TEST()

FN_TEST(multicast_and_sequence_snapshot)
{
	int subscribers[] = { open_listener(1), open_listener(1),
			      open_listener(0), open_listener(2) };
	uint64_t before = sequence();
	TEST_RES(sequence(), _ret == before);
	int fd = open("/sys/kernel/uevent_seqnum", O_WRONLY);
	int result = -1, saved_errno = errno;
	if (fd >= 0) {
		result = write(fd, "1", 1);
		saved_errno = errno;
		CHECK(close(fd));
	}
	TEST_RES(result == -1 &&
			 (saved_errno == EACCES || saved_errno == EPERM ||
			  saved_errno == EIO || saved_errno == EROFS),
		 _ret);
	const char command[] = "change " UUID "\n";
	TEST_RES(write_command(command, sizeof(command) - 1),
		 _ret == sizeof(command) - 1);
	struct event first, second;
	int first_result =
		TEST_RES(receive_event(subscribers[0], "change", UUID, &first,
				       now_ms() + EVENT_TIMEOUT_MS),
			 _ret == 1);
	int second_result =
		TEST_RES(receive_event(subscribers[1], "change", UUID, &second,
				       now_ms() + EVENT_TIMEOUT_MS),
			 _ret == 1);
	if (first_result == 1 && second_result == 1) {
		TEST_RES(first.seqnum, _ret == before + 1);
		TEST_RES(second.seqnum, _ret == first.seqnum);
		TEST_RES(sequence(), _ret == first.seqnum);
	}
	for (size_t i = 2; i < 4; i++)
		TEST_RES(receive_event(subscribers[i], "change", UUID, &second,
				       now_ms() + QUIET_TIMEOUT_MS),
			 _ret == 0);
	for (size_t i = 0; i < 4; i++)
		CHECK(close(subscribers[i]));
}
END_TEST()

struct writer {
	pthread_barrier_t *barrier;
	char uuid[37];
	ssize_t result;
	int error;
};

static void *write_concurrently(void *arg)
{
	struct writer *writer = arg;
	int result = pthread_barrier_wait(writer->barrier);
	if (result != 0 && result != PTHREAD_BARRIER_SERIAL_THREAD) {
		writer->result = -1;
		writer->error = result;
		return NULL;
	}
	char command[64];
	int len =
		snprintf(command, sizeof(command), "change %s\n", writer->uuid);
	writer->result = write_command(command, len);
	writer->error = writer->result < 0 ? errno : 0;
	return NULL;
}

FN_TEST(concurrent_writers)
{
	int fd = open_listener(1);
	pthread_barrier_t barrier;
	CHECK_WITH(pthread_barrier_init(&barrier, NULL, NUM_WRITERS),
		   _ret == 0);
	pthread_t threads[NUM_WRITERS];
	struct writer writers[NUM_WRITERS];
	uint64_t before = sequence();
	for (size_t i = 0; i < NUM_WRITERS; i++) {
		writers[i].barrier = &barrier;
		snprintf(writers[i].uuid, sizeof(writers[i].uuid),
			 "77777777-6666-5555-4444-%012zu", i);
		CHECK_WITH(pthread_create(&threads[i], NULL, write_concurrently,
					  &writers[i]),
			   _ret == 0);
	}
	for (size_t i = 0; i < NUM_WRITERS; i++) {
		CHECK_WITH(pthread_join(threads[i], NULL), _ret == 0);
		TEST_RES(writers[i].result == 44 && writers[i].error == 0,
			 _ret);
	}
	CHECK_WITH(pthread_barrier_destroy(&barrier), _ret == 0);
	bool found[NUM_WRITERS] = { false };
	uint64_t sequences[NUM_WRITERS] = { 0 };
	size_t count = 0;
	int64_t deadline_ms = now_ms() + EVENT_TIMEOUT_MS;
	while (count < NUM_WRITERS) {
		struct event event;
		int result =
			receive_event(fd, "change", NULL, &event, deadline_ms);
		if (result != 1) {
			TEST_RES(result, _ret == 1);
			break;
		}
		const char *uuid = value(&event, "SYNTH_UUID");
		for (size_t i = 0; uuid && i < NUM_WRITERS; i++) {
			if (strcmp(uuid, writers[i].uuid))
				continue;
			TEST_RES(found[i], !_ret);
			if (!found[i]) {
				found[i] = true;
				sequences[count++] = event.seqnum;
			}
			break;
		}
	}
	TEST_RES(count, _ret == NUM_WRITERS);
	for (size_t i = 0; i < count; i++) {
		TEST_RES(sequences[i] > before &&
				 sequences[i] <= before + NUM_WRITERS,
			 _ret);
		for (size_t j = 0; j < i; j++)
			TEST_RES(sequences[i] != sequences[j], _ret);
	}
	TEST_RES(sequence(), _ret == before + NUM_WRITERS);
	struct event event;
	TEST_RES(receive_event(fd, "change", NULL, &event,
			       now_ms() + QUIET_TIMEOUT_MS),
		 _ret == 0);
	CHECK(close(fd));
}
END_TEST()
