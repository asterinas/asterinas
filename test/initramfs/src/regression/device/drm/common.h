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
#define DRM_FIELD_LEN 64
#define DRM_MAJOR 226

#define DRM_IOCTL_VERSION _IOWR('d', 0x00, struct drm_version)
#define DRM_IOCTL_GET_MAGIC _IOR('d', 0x02, struct drm_auth)
#define DRM_IOCTL_GET_CAP _IOWR('d', 0x0c, struct drm_get_cap)
#define DRM_IOCTL_AUTH_MAGIC _IOW('d', 0x11, struct drm_auth)
#define DRM_IOCTL_SET_MASTER _IO('d', 0x1e)
#define DRM_IOCTL_DROP_MASTER _IO('d', 0x1f)

#define DRM_CAP_PRIME 0x5
#define DRM_CAP_TIMESTAMP_MONOTONIC 0x6
#define DRM_CAP_SYNCOBJ 0x13
#define DRM_CAP_SYNCOBJ_TIMELINE 0x14

#define DRM_PRIME_CAP_IMPORT 0x1
#define DRM_PRIME_CAP_EXPORT 0x2

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

struct drm_auth {
	uint32_t magic;
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

static inline int open_optional_drm_node(const char *path)
{
	int fd = open(path, O_RDWR);

	if (fd >= 0 || errno == ENOENT || errno == ENODEV || errno == ENXIO) {
		return fd;
	}

	fprintf(stderr, "fatal error: open('%s') failed: %s\n", path,
		strerror(errno));
	exit(EXIT_FAILURE);
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

#endif /* DRM_TEST_COMMON_H */
