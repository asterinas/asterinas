/* SPDX-License-Identifier: MPL-2.0 */

#ifndef DRM_TEST_COMMON_H
#define DRM_TEST_COMMON_H

#include <errno.h>
#include <drm/drm.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <unistd.h>
#include <xf86drm.h>

#define DRM_CARD_DEVICE "/dev/dri/card0"
#define DRM_RENDER_DEVICE "/dev/dri/renderD128"
#define DRM_MAJOR 226

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

static inline int is_boolean_drm_cap(uint64_t value)
{
	return value == 0 || value == 1;
}

#endif /* DRM_TEST_COMMON_H */
