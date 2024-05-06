// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/syscall.h>
#include <linux/capability.h>

int set_caps(__u32 target_pid, __u32 capabilities)
{
	struct __user_cap_header_struct capheader;
	struct __user_cap_data_struct capdata[2];
	capheader.version = _LINUX_CAPABILITY_VERSION_3;
	capheader.pid = target_pid;
	memset(&capdata, 0, sizeof(capdata));

	// Set specified capabilities
	capdata[0].effective = capdata[0].permitted = capabilities;
	capdata[0].inheritable = 0;
	if (syscall(SYS_capset, &capheader, &capdata) < 0) {
		perror("capset failed");
		return 1;
	}
	printf("Process capabilities set successfully.\n");
	return 0;
}

int check_caps(__u32 target_pid, __u32 capabilities)
{
	struct __user_cap_header_struct capheader;
	struct __user_cap_data_struct capdata[2];
	memset(&capheader, 0, sizeof(capheader));
	memset(&capdata, 0, sizeof(capdata));
	capheader.version = _LINUX_CAPABILITY_VERSION_3;
	capheader.pid = target_pid;
	if (syscall(SYS_capget, &capheader, &capdata) == -1) {
		perror("capget failed");
		exit(EXIT_FAILURE);
	}
	printf("Process capabilities retrieved successfully.\n");
	return (capdata[0].permitted & capabilities) &&
	       (capdata[0].effective & capabilities);
}

int main(void)
{
	__u32 target_pid = getpid();
	printf("Process Pid: %u.\n", target_pid);

	__u32 caps_to_set =
		(1 << CAP_NET_RAW) |
		(1 << CAP_NET_ADMIN); // Define the desired capabilities.

	// Try setting the specified capabilities.
	if (set_caps(target_pid, caps_to_set) != 0) {
		fprintf(stderr, "Failed to set capabilities.\n");
		return 1;
	}

	// Check for CAP_NET_RAW among the process's capabilities.
	if (check_caps(target_pid, 1 << CAP_NET_RAW)) {
		printf("Process has CAP_NET_RAW capability.\n");
	} else {
		fprintf(stderr,
			"Process does NOT have CAP_NET_RAW capability.\n");
		return 1;
	}

	// Check for CAP_NET_ADMIN among the process's capabilities.
	if (check_caps(target_pid, 1 << CAP_NET_ADMIN)) {
		printf("Process has CAP_NET_ADMIN capability.\n");
	} else {
		fprintf(stderr,
			"Process does NOT have CAP_NET_ADMIN capability.\n");
		return 1;
	}

	return 0;
}