/* SPDX-License-Identifier: MPL-2.0 */

#ifndef DEVICE_MODEL_TEST_COMMON_H
#define DEVICE_MODEL_TEST_COMMON_H

#include <errno.h>
#include <limits.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>

#define ARRAY_SIZE(array) (sizeof(array) / sizeof((array)[0]))

struct device_attribute {
	/* Attribute path relative to the device directory. */
	const char *path;
	/* Expected permission bits, excluding the file type. */
	mode_t mode;
};

struct device_node {
	/* Absolute path to the device node under `/dev`. */
	const char *path;
	/* Expected `S_IFCHR` or `S_IFBLK` combined with permission bits. */
	mode_t mode;
};

struct device_spec {
	/* Canonical absolute path to the device directory under `/sys/devices`. */
	const char *path;
	/*
	 * Canonical `subsystem` link target under `/sys/bus` or `/sys/class`.
	 * `NULL` requires the link to be absent.
	 */
	const char *subsystem;
	/*
	 * Canonical `driver` link target under `/sys/bus/<bus>/drivers`.
	 * `NULL` requires the link to be absent.
	 */
	const char *driver;
	/*
	 * Canonical target of a class device's `device` link to its parent.
	 * `NULL` requires the link to be absent. Only checked for class devices.
	 */
	const char *device_link;
	/*
	 * Attributes to check, or `NULL` to skip attribute checks.
	 * End the list with `{ NULL, 0 }`.
	 * Additional attributes are allowed.
	 */
	const struct device_attribute *attributes;
	/* Expected device node, or `NULL` to require the `dev` attribute to be absent. */
	const struct device_node *node;
};

static inline int device_error(const char *path, const char *format, ...)
{
	va_list args;

	fprintf(stderr, "%s: ", path);
	va_start(args, format);
	vfprintf(stderr, format, args);
	va_end(args);
	fputc('\n', stderr);
	errno = EINVAL;
	return -1;
}

static inline int check_absent(const char *path)
{
	struct stat st;

	if (lstat(path, &st) == 0)
		return device_error(path, "expected no entry");
	if (errno != ENOENT)
		return device_error(path, "lstat failed: %s", strerror(errno));
	errno = 0;
	return 0;
}

static inline int check_mode(const char *path, mode_t mode)
{
	struct stat st;

	if (lstat(path, &st) < 0)
		return device_error(path, "lstat failed: %s", strerror(errno));
	if ((st.st_mode & (S_IFMT | 07777)) != mode)
		return device_error(path, "expected mode %#o, got %#o", mode,
				    st.st_mode & (S_IFMT | 07777));
	return 0;
}

static inline int check_link(const char *path, const char *target)
{
	struct stat st;
	char resolved[PATH_MAX];

	if (lstat(path, &st) < 0)
		return device_error(path, "lstat failed: %s", strerror(errno));
	if (!S_ISLNK(st.st_mode))
		return device_error(path, "expected a symlink");
	if (!realpath(path, resolved))
		return device_error(path, "realpath failed: %s",
				    strerror(errno));
	if (strcmp(resolved, target))
		return device_error(path, "expected target %s, got %s", target,
				    resolved);
	return 0;
}

/*
 * Checks `<device>/subsystem` and the reverse link under the bus or class.
 * If `subsystem` is `NULL`, checks that `<device>/subsystem` is absent.
 */
static inline int check_subsystem(const char *device, const char *subsystem)
{
	char path[PATH_MAX];
	const char *members = "";

	snprintf(path, sizeof(path), "%s/subsystem", device);
	if (!subsystem)
		return check_absent(path);
	if (check_link(path, subsystem) < 0)
		return -1;

	if (!strncmp(subsystem, "/sys/bus/", strlen("/sys/bus/")))
		members = "/devices";
	else if (!strncmp(subsystem, "/sys/class/", strlen("/sys/class/")))
		;
	else
		return device_error(subsystem, "expected a bus or class");

	snprintf(path, sizeof(path), "%s%s/%s", subsystem, members,
		 strrchr(device, '/') + 1);
	return check_link(path, device);
}

/*
 * Checks `<device>/driver` and the reverse link under the driver directory.
 * If `driver` is `NULL`, checks that `<device>/driver` is absent.
 */
static inline int check_driver(const char *device, const char *driver)
{
	char path[PATH_MAX];

	snprintf(path, sizeof(path), "%s/driver", device);
	if (!driver)
		return check_absent(path);
	if (check_link(path, driver) < 0)
		return -1;

	snprintf(path, sizeof(path), "%s/%s", driver, strrchr(device, '/') + 1);
	return check_link(path, device);
}

/* Checks a class device's `device` link, or its absence when no target is specified. */
static inline int check_device_link(const struct device_spec *spec)
{
	/* Bus devices may expose `device` as an attribute instead of a parent link. */
	if (!spec->subsystem ||
	    strncmp(spec->subsystem, "/sys/class/", strlen("/sys/class/")))
		return 0;

	char path[PATH_MAX];

	snprintf(path, sizeof(path), "%s/device", spec->path);
	if (!spec->device_link)
		return check_absent(path);
	return check_link(path, spec->device_link);
}

/*
 * Checks the type and permissions of `spec->node->path`,
 * and that its device number matches `<spec->path>/dev`.
 * The corresponding `/sys/dev/{char,block}/<major>:<minor>` link
 * must point back to `spec->path`.
 * If `spec->node` is `NULL`, checks that `<spec->path>/dev` is absent.
 */
static inline int check_node(const struct device_spec *spec)
{
	char path[PATH_MAX];
	struct stat st;
	unsigned int major_id, minor_id;

	snprintf(path, sizeof(path), "%s/dev", spec->path);
	if (!spec->node)
		return check_absent(path);

	if (lstat(path, &st) < 0)
		return device_error(path, "lstat failed: %s", strerror(errno));
	if (!S_ISREG(st.st_mode))
		return device_error(path, "expected a regular attribute");

	/* Read the device number from `dev` because allocation can vary across kernels. */
	FILE *file = fopen(path, "r");
	if (!file)
		return device_error(path, "open failed: %s", strerror(errno));
	int fields = fscanf(file, "%u:%u", &major_id, &minor_id);
	fclose(file);
	if (fields != 2)
		return device_error(path, "expected major:minor");

	/* Use the expected node type to locate the reverse link under `/sys/dev`. */
	if (!S_ISCHR(spec->node->mode) && !S_ISBLK(spec->node->mode))
		return device_error(spec->node->path,
				    "expected a character or block device");
	snprintf(path, sizeof(path), "/sys/dev/%s/%u:%u",
		 S_ISBLK(spec->node->mode) ? "block" : "char", major_id,
		 minor_id);
	if (check_link(path, spec->path) < 0)
		return -1;

	/* Check the `/dev` node against both the specification and the sysfs device number. */
	if (lstat(spec->node->path, &st) < 0)
		return device_error(spec->node->path, "lstat failed: %s",
				    strerror(errno));
	if ((st.st_mode & (S_IFMT | 07777)) != spec->node->mode)
		return device_error(spec->node->path,
				    "expected mode %#o, got %#o",
				    spec->node->mode,
				    st.st_mode & (S_IFMT | 07777));
	if (st.st_rdev != makedev(major_id, minor_id))
		return device_error(spec->node->path,
				    "expected device number %u:%u, got %u:%u",
				    major_id, minor_id, major(st.st_rdev),
				    minor(st.st_rdev));
	return 0;
}

/*
 * Checks a device's canonical path, membership links, attributes, and node.
 * Returns `-1` with a diagnostic and sets `errno` on failure for `TEST_SUCC()`.
 */
static inline int check_device(const struct device_spec *spec)
{
	char path[PATH_MAX], resolved[PATH_MAX];
	struct stat st;

	/* Require an existing directory at its canonical path under `/sys/devices`. */
	if (strncmp(spec->path, "/sys/devices/", strlen("/sys/devices/")))
		return device_error(spec->path, "expected a /sys/devices path");
	if (lstat(spec->path, &st) < 0)
		return device_error(spec->path, "lstat failed: %s",
				    strerror(errno));
	if (!S_ISDIR(st.st_mode))
		return device_error(spec->path, "expected a directory");
	if (!realpath(spec->path, resolved))
		return device_error(spec->path, "realpath failed: %s",
				    strerror(errno));
	if (strcmp(resolved, spec->path))
		return device_error(spec->path, "expected canonical path %s",
				    resolved);

	/* Check `subsystem` and `driver` links in both directions, or their absence. */
	if (check_subsystem(spec->path, spec->subsystem) < 0 ||
	    check_driver(spec->path, spec->driver) < 0)
		return -1;

	/* Check the class device's parent link, or its absence. */
	if (check_device_link(spec) < 0)
		return -1;

	/*
	 * Require each listed attribute to be a regular file with the expected permissions.
	 * Ignore additional attributes so the same specification can run on Linux.
	 */
	for (const struct device_attribute *attr = spec->attributes;
	     attr && attr->path; attr++) {
		snprintf(path, sizeof(path), "%s/%s", spec->path, attr->path);
		if (check_mode(path, S_IFREG | attr->mode) < 0)
			return -1;
	}

	/* Check the device node and its sysfs links, or require `dev` to be absent. */
	if (check_node(spec) < 0)
		return -1;

	errno = 0;
	return 0;
}

#endif /* DEVICE_MODEL_TEST_COMMON_H */
