/* SPDX-License-Identifier: MPL-2.0 */

#ifndef DRM_TEST_COMMON_H
#define DRM_TEST_COMMON_H

#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <unistd.h>

#define DRM_CARD_DEVICE "/dev/dri/card0"
#define DRM_RENDER_DEVICE "/dev/dri/renderD128"
#define DRM_SIMPLEDRM_NAME "simpledrm"
#define DRM_FIELD_LEN 64
#define DRM_MAJOR 226

#define DRM_IOCTL_VERSION _IOWR('d', 0x00, struct drm_version)
#define DRM_IOCTL_GET_CAP _IOWR('d', 0x0c, struct drm_get_cap)
#define DRM_IOCTL_SET_CLIENT_CAP _IOW('d', 0x0d, struct drm_set_client_cap)
#define DRM_IOCTL_SET_MASTER _IO('d', 0x1e)
#define DRM_IOCTL_DROP_MASTER _IO('d', 0x1f)

#define DRM_CAP_DUMB_BUFFER 0x1
#define DRM_CAP_DUMB_PREFERRED_DEPTH 0x3
#define DRM_CAP_PRIME 0x5
#define DRM_CAP_TIMESTAMP_MONOTONIC 0x6
#define DRM_CAP_CURSOR_WIDTH 0x8
#define DRM_CAP_CURSOR_HEIGHT 0x9
#define DRM_CAP_SYNCOBJ 0x13
#define DRM_CAP_SYNCOBJ_TIMELINE 0x14

#define DRM_PRIME_CAP_IMPORT 0x1
#define DRM_PRIME_CAP_EXPORT 0x2

#define DRM_CLIENT_CAP_ATOMIC 0x3
#define DRM_CLIENT_CAP_WRITEBACK_CONNECTORS 0x5

struct drm_version {
	int version_major;
	int version_minor;
	int version_patchlevel;
	unsigned long name_len;
	char *name;
	unsigned long date_len;
	char *date;
	unsigned long desc_len;
	char *desc;
};

struct drm_get_cap {
	uint64_t capability;
	uint64_t value;
};

struct drm_set_client_cap {
	uint64_t capability;
	uint64_t value;
};

struct drm_node {
	const char *path;
	unsigned int major;
	unsigned int minor;
};

static inline const struct drm_node *drm_test_nodes(size_t *count)
{
	static const struct drm_node nodes[] = {
		{ DRM_CARD_DEVICE, DRM_MAJOR, 0 },
		{ DRM_RENDER_DEVICE, DRM_MAJOR, 128 },
	};

	*count = sizeof(nodes) / sizeof(nodes[0]);
	return nodes;
}

static inline int is_drm_render_node(const struct drm_node *node)
{
	return strcmp(node->path, DRM_RENDER_DEVICE) == 0;
}

static inline int open_drm_node_or_skip(const char *path)
{
	int fd = open(path, O_RDWR);

	if (fd < 0) {
		if (errno == ENOENT || errno == ENODEV || errno == ENXIO) {
			fprintf(stderr, "DRM tests skipped: %s (%s)\n", path,
				strerror(errno));
			exit(EXIT_SUCCESS);
		}

		fprintf(stderr, "fatal error: open('%s') failed: %s\n", path,
			strerror(errno));
		exit(EXIT_FAILURE);
	}

	return fd;
}

static inline int get_drm_version(int fd, struct drm_version *version,
				  char *name, char *date, char *desc,
				  size_t len)
{
	memset(name, 0, len);
	memset(date, 0, len);
	memset(desc, 0, len);
	memset(version, 0, sizeof(*version));

	version->name_len = len;
	version->name = name;
	version->date_len = len;
	version->date = date;
	version->desc_len = len;
	version->desc = desc;

	return ioctl(fd, DRM_IOCTL_VERSION, version);
}

static inline int get_drm_driver_name(int fd, char *name, size_t len)
{
	struct drm_version version;

	if (len == 0) {
		errno = EINVAL;
		return -1;
	}

	memset(name, 0, len);
	memset(&version, 0, sizeof(version));

	version.name_len = len - 1;
	version.name = name;

	return ioctl(fd, DRM_IOCTL_VERSION, &version);
}

static inline int should_skip_drm_render_node(int card_fd, int *skip)
{
	char driver_name[DRM_FIELD_LEN];
	int ret =
		get_drm_driver_name(card_fd, driver_name, sizeof(driver_name));

	if (ret < 0) {
		return ret;
	}

	/*
	 * simpledrm is a modeset-only bootstrap driver. It does not expose a
	 * render node, so render-node checks are only meaningful for future DRM
	 * drivers that advertise a different driver name.
	 */
	*skip = strcmp(driver_name, DRM_SIMPLEDRM_NAME) == 0;
	return 0;
}

static inline int get_drm_cap(int fd, uint64_t capability, uint64_t *value)
{
	struct drm_get_cap cap = {
		.capability = capability,
	};
	int ret = ioctl(fd, DRM_IOCTL_GET_CAP, &cap);

	*value = cap.value;
	return ret;
}

static inline int is_boolean_drm_cap(uint64_t value)
{
	return value == 0 || value == 1;
}

static inline int set_drm_client_cap(int fd, uint64_t capability,
				     uint64_t value)
{
	struct drm_set_client_cap cap = {
		.capability = capability,
		.value = value,
	};

	return ioctl(fd, DRM_IOCTL_SET_CLIENT_CAP, &cap);
}

#endif /* DRM_TEST_COMMON_H */
