/* SPDX-License-Identifier: MPL-2.0 */

/*
 * A framework for writing general tests.
 *
 * A general test typically consists of two parts, the setup part and the
 * test part. The setup part contains setup functions that set up the context
 * for the subsequent tests to run. The setup functions cannot fail, and if they
 * do, execution is aborted because the subsequent tests will not work as
 * expected either. The test functions, on the other hand, can fail, and if they
 * do, they are reported as test failures.
 *
 * The framework provides basic utilities for writing general tests:
 *
 *  - To define a setup function or a test function, FN_SETUP() or FN_TEST() can
 * be used. These functions are automatically executed in the order of their
 * definition. Note that the order of execution is _not_ related to whether a
 * function is a setup function or a test function.
 *
 *  - Within a setup function, CHECK() can be used to write a setup expression
 * that must succeed. If the expression fails, a fatal error will be reported
 * and the execution will be aborted.
 *
 *  - Within a test function, TEST_SUCC() can be used to write a test expression
 * that should succeed. If the expression fails, a test failure will be reported
 * but the execution will continue.
 *
 *  - The number of successful and failed tests is tracked. When a test function
 * finishes, a summary sentence is printed describing the number of test
 * failures. The program will exit with a non-zero code if and only if there is
 * at least one test failure.
 */

#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/** Starts the definition of a setup function. */
#define FN_SETUP(name)                                           \
	void setup_##name(void)                                  \
		__attribute__((constructor(__COUNTER__ + 200))); \
                                                                 \
	void setup_##name(void)                                  \
	{
/** Ends the definition of a setup function. */
#define END_SETUP() }

#define __CHECK(func, cond)                                                   \
	errno = 0;                                                            \
	__auto_type _ret = (func);                                            \
	if (!(cond)) {                                                        \
		fprintf(stderr,                                               \
			"fatal error: %s: `" #cond "` is false after `" #func \
			"` [got %s]\n",                                       \
			__func__, strerror(errno));                           \
		exit(EXIT_FAILURE);                                           \
	}

/**
 * Makes a function call and checks its return value is positive.
 *
 * The execution will be aborted if the check fails.
 */
#define CHECK(func)                       \
	({                                \
		__CHECK(func, _ret >= 0); \
		_ret;                     \
	})

/**
 * Makes a function call and checks its result with the specified condition.
 *
 * The execution will be aborted if the check fails.
 *
 * The return value of the function can be accessed with a local variable named
 * _ret.
 */
#define CHECK_WITH(func, cond)       \
	({                           \
		__CHECK(func, cond); \
		_ret;                \
	})

static int __total_failures;

/** Starts the definition of a test function. */
#define FN_TEST(name)                                            \
	void test_##name(void)                                   \
		__attribute__((constructor(__COUNTER__ + 200))); \
                                                                 \
	void test_##name(void)                                   \
	{                                                        \
		int __tests_passed = 0, __tests_failed = 0;

/** Ends the definition of a test function. */
#define END_TEST()                                                        \
	fprintf(stderr, "%s summary: %d tests passed, %d tests failed\n", \
		__func__, __tests_passed, __tests_failed);                \
	__total_failures += __tests_failed;                               \
	}

#define __TEST(func, err, cond)                                                \
	errno = 0;                                                             \
	__auto_type _ret = (func);                                             \
	if (errno != (err)) {                                                  \
		__tests_failed++;                                              \
		fprintf(stderr,                                                \
			"%s: `" #func "` failed [got %s, but expected %s]\n",  \
			__func__, strerror(errno), strerror(err));             \
	} else if (!(cond)) {                                                  \
		__tests_failed++;                                              \
		fprintf(stderr,                                                \
			"%s: `" #func "` failed [got %s, but `" #cond          \
			"` is false]\n",                                       \
			__func__, strerror(errno));                            \
	} else {                                                               \
		__tests_passed++;                                              \
		fprintf(stderr, "%s: `" #func "` passed [got %s]\n", __func__, \
			strerror(errno));                                      \
	}

/**
 * Makes a function call and checks its result.
 *
 * A test failure will be reported if the function does not set the specified
 * errno or the specified condition does not meet.
 *
 * The return value of the function can be accessed with a local variable named
 * _ret.
 */
#define TEST(func, err, cond)            \
	({                               \
		__TEST(func, err, cond); \
		_ret;                    \
	})
/**
 * Makes a function call and checks whether it succeeds.
 *
 * A test failure will be reported if the function sets a non-zero errno.
 */
#define TEST_SUCC(func) TEST(func, 0, 1)

/**
 * Makes a function call and checks whether it fails in the specified way.
 *
 * A test failure will be reported if the function does not set the specified
 * errno.
 */
#define TEST_ERRNO(func, err) TEST(func, err, 1)

/**
 * Makes a function call and checks whether it produces expected results.
 *
 * A test failure will be reported if the function sets a non-zero errno or the
 * specified condition does not meet.
 *
 * The return value of the function can be accessed with a local variable named
 * _ret.
 */
#define TEST_RES(func, cond) TEST(func, 0, cond)

int main(void)
{
	return __total_failures ? 1 : 0;
}
