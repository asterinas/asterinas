// SPDX-License-Identifier: MPL-2.0

#ifndef EXT_FS_TEST_H
#define EXT_FS_TEST_H

// The common mount root shared by every ext test binary. The runner
// (run_test.sh) bind-mounts the filesystem under test here before running
// the binaries listed in ext.tests.
#define EXT_TEST_ROOT "/ext-test"

#endif
